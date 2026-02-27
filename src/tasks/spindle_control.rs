//! Combined spindle control task.
//!
//! This task handles:
//! 1. Reading PWM input from Carvera (requested speed)
//! 2. Calculating and outputting PWM to ESCON (speed control)
//! 3. Reading actual speed from ESCON (for display and stall detection)
//! 4. Stall detection with dynamic grace period
//! 5. Setting error output when stall or ESCON alert detected
//! 6. Forcing spindle disable on any error (safety shutdown)
//!
//! Note: ESCON speed output (4 PPR) is wired directly to Carvera encoder input,
//! bypassing the Pico for speed feedback.

use core::sync::atomic::Ordering;
use embassy_rp::gpio::{Input, Output};
use embassy_rp::pwm::{Pwm, SetDutyCycle};
use embassy_time::{Duration, Instant, Timer};

use crate::display::DisplayStatus;
use crate::state::{
    CAL_SEQUENCE_ACTIVE, ENABLED, any_error_active, config, get_active_error_type, get_current_ma,
    is_safety_shutdown, report_escon_alert, report_stall_alert, report_stall_latched,
    update_display,
};
use crate::{
    StabilizationStatus, StabilizationTracker, StallConfig, StallDetector, StallStatus,
    spindle_to_motor_rpm,
};

/// Control loop interval (20ms = 50Hz)
const CONTROL_INTERVAL_MS: u64 = 20;

/// Read raw PWM input duty from PIO measurement task and apply calibration correction.
/// Returns (corrected_duty, uncorrected_duty, min_enable_duty).
fn read_input_duty() -> (u16, u16, u16) {
    // MEASURED_DUTY is 0-10000 scale (0.00% to 100.00%) - cycle-aligned measurement
    let uncorrected = crate::tasks::pwm_input::MEASURED_DUTY.load(Ordering::SeqCst) as u16;
    // Apply calibration correction if available (piecewise-linear interpolation)
    let corrected = crate::calibration::correct_duty(uncorrected);
    // Compute enable threshold from MIN_ENABLE_RPM (e.g. 750 RPM -> 367 in 0-10000 scale)
    let min_enable_duty = ((config::MIN_ENABLE_RPM as u64 * 10000
        + config::CARVERA_SPINDLE_MAX_RPM as u64 / 2)
        / config::CARVERA_SPINDLE_MAX_RPM as u64) as u16;
    (corrected, uncorrected, min_enable_duty)
}

/// Check if input has timed out (no valid signal for >200ms).
/// Updates `last_valid_input` when signal is above threshold.
fn check_timeout(
    raw_duty_fine: u16,
    min_enable_duty: u16,
    last_valid_input: &mut Instant,
    timeout_duration: Duration,
) -> bool {
    if raw_duty_fine > min_enable_duty {
        *last_valid_input = Instant::now();
        false
    } else {
        Instant::now().duration_since(*last_valid_input) > timeout_duration
    }
}

/// Compute whether spindle should be enabled, target RPMs, and ESCON output duty.
/// Returns (enabled, spindle_rpm, target_motor_rpm, output_duty).
fn compute_output(
    timed_out: bool,
    raw_duty_fine: u16,
    raw_duty_fine_uncorrected: u16,
    min_enable_duty: u16,
    safety_shutdown: bool,
    has_error: bool,
) -> (bool, u32, u32, u16) {
    let enabled = !timed_out && raw_duty_fine > min_enable_duty && !safety_shutdown && !has_error;

    // Use calibrated RPM directly to avoid lossy duty->RPM->duty->RPM round-trip
    let spindle_rpm = if enabled {
        crate::calibration::duty_to_calibrated_rpm(raw_duty_fine_uncorrected)
    } else {
        0
    };
    let target_motor_rpm = spindle_to_motor_rpm(spindle_rpm, config::BELT_RATIO_X1000);

    // Linear formula: maps motor RPM to ESCON output duty (100-900 scale)
    let output_duty = if enabled {
        crate::motor_rpm_to_output_duty(target_motor_rpm, config::MIN_RPM, config::MAX_RPM)
    } else {
        config::PWM_MIN_DUTY
    };

    (enabled, spindle_rpm, target_motor_rpm, output_duty)
}

/// Update the enable pin and ENABLED atomic based on computed enable state.
fn update_enable(enable_pin: &mut Output<'static>, enabled: bool) {
    if enabled {
        enable_pin.set_high();
        ENABLED.store(true, Ordering::SeqCst);
    } else {
        enable_pin.set_low();
        ENABLED.store(false, Ordering::SeqCst);
    }
}

/// Run stall detection, handling calibration-active suppression and edge-triggered reset.
/// Returns (stall_status, stall_detected).
fn run_stall_detection(
    stall_detector: &mut StallDetector,
    was_cal_active: &mut bool,
    target_motor_rpm: u32,
    actual_rpm: u32,
    now_ms: u64,
    stall_config: &StallConfig,
) -> (StallStatus, bool) {
    let cal_active = CAL_SEQUENCE_ACTIVE.load(Ordering::SeqCst);
    // Edge-triggered reset: only reset once when calibration ends, not every iteration.
    // reset() clears initialized flag so the next check() gets a fresh grace period
    // instead of seeing stale timestamps from 30+ seconds ago.
    if *was_cal_active && !cal_active {
        stall_detector.reset();
    }
    *was_cal_active = cal_active;

    let stall_status = if cal_active {
        StallStatus::Ok
    } else {
        stall_detector.check(target_motor_rpm, actual_rpm, now_ms, stall_config)
    };
    let stall_detected = matches!(stall_status, StallStatus::Stalled);
    (stall_status, stall_detected)
}

/// Report stall and ESCON alert errors to centralized error state.
/// Returns the re-read error_active flag (after stall/escon updates).
fn report_errors(
    stall_detector: &StallDetector,
    stall_detected: bool,
    target_motor_rpm: u32,
    now_ms: u64,
    escon_alert_active: bool,
) -> bool {
    report_escon_alert(escon_alert_active);
    let stall_alert_active = stall_detector.is_alert_active(target_motor_rpm, now_ms);
    report_stall_alert(stall_alert_active);
    report_stall_latched(stall_detected);
    // Re-read error state after updating stall/escon (overcurrent/thermal set by other tasks)
    any_error_active()
}

/// Log stabilization event once per speed change (not every loop iteration).
fn log_stabilization(
    stab_status: StabilizationStatus,
    stab_time_ms: Option<u32>,
    tracker: &mut StabilizationTracker,
) {
    if matches!(stab_status, StabilizationStatus::Stabilized) && !tracker.is_reported() {
        if let Some(time_ms) = stab_time_ms {
            let (from_rpm, to_rpm) = tracker.get_debug_info();
            let from_spindle = (from_rpm * config::BELT_RATIO_X1000 + 500) / 1000;
            let to_spindle = (to_rpm * config::BELT_RATIO_X1000 + 500) / 1000;
            defmt::info!(
                "STABILIZED: {} -> ~{} in {}.{} seconds",
                from_spindle,
                to_spindle,
                time_ms / 1000,
                (time_ms % 1000) / 100
            );
            tracker.mark_reported();
        }
    }
}

/// Update error output GPIO (active-low signaling: LOW = fault, HIGH = OK).
fn update_error_output(error_out: &mut Output<'static>, error_active: bool) {
    if error_active {
        error_out.set_low(); // Signal fault to Carvera
    } else {
        error_out.set_high(); // Signal OK to Carvera
    }
}

/// Build display status from current loop state.
fn build_display_status(
    spindle_rpm: u32,
    actual_rpm: u32,
    current_ma: u32,
    error_active: bool,
    enabled: bool,
    stab_time_ms: Option<u32>,
) -> DisplayStatus {
    DisplayStatus {
        requested_rpm: spindle_rpm,
        actual_rpm: (actual_rpm * config::BELT_RATIO_X1000 + 500) / 1000,
        current_ma,
        error: error_active,
        error_type: get_active_error_type(),
        enabled,
        stabilization_time_ms: stab_time_ms,
    }
}

/// Combined spindle control task
#[embassy_executor::task]
pub async fn spindle_control_task(
    mut pwm_out: Pwm<'static>,
    mut enable_pin: Output<'static>,
    mut error_out: Output<'static>,
    escon_alert: Input<'static>,
) {
    // Initialize state
    let mut stall_detector = StallDetector::new();
    let mut was_cal_active = false;
    let mut stabilization_tracker = StabilizationTracker::new();
    // PWM clock divider (150MHz / 150 = 1MHz)
    const PWM_DIVIDER: u8 = 150;
    // PWM TOP value for 1kHz signal at 1MHz clock = 1000 counts per period
    let pwm_top = (config::CLOCK_HZ / (PWM_DIVIDER as u32) / config::PWM_OUTPUT_FREQ_HZ) as u16;

    // Track last valid input time for timeout
    let mut last_valid_input = Instant::now();
    let timeout_duration = Duration::from_millis(200);

    let stall_config = StallConfig {
        threshold_pct: config::STALL_THRESHOLD_PCT,
        base_grace_ms: config::BASE_GRACE_MS,
        rpm_grace_factor: config::RPM_GRACE_FACTOR,
        debounce_ms: config::STALL_DEBOUNCE_MS,
        recovery_ms: config::STALL_RECOVERY_MS,
        rate_threshold: config::SPEED_CHANGE_RATE_THRESHOLD,
    };

    defmt::info!("Spindle control task started, pwm_top={}", pwm_top);
    defmt::info!(
        "Stall config: threshold={}% debounce={}ms recovery={}ms grace={}ms+{}ms/kRPM",
        config::STALL_THRESHOLD_PCT,
        config::STALL_DEBOUNCE_MS,
        config::STALL_RECOVERY_MS,
        config::BASE_GRACE_MS,
        config::RPM_GRACE_FACTOR
    );

    let mut loop_count: u32 = 0;

    loop {
        let now_ms = Instant::now().as_millis();
        loop_count = loop_count.wrapping_add(1);

        // Record heartbeat for watchdog health monitoring
        crate::state::heartbeat(&crate::state::HEARTBEAT_SPINDLE_CONTROL, now_ms);

        // === SAFETY CHECK: Force disable on any error ===
        let safety_shutdown = is_safety_shutdown();
        let has_error = any_error_active();

        // 1. Read and correct PWM input
        let (raw_duty_fine, raw_duty_fine_uncorrected, min_enable_duty) = read_input_duty();

        // 2. Check for timeout
        let timed_out = check_timeout(
            raw_duty_fine,
            min_enable_duty,
            &mut last_valid_input,
            timeout_duration,
        );

        // 3. Compute enable state, RPMs, and output duty
        let (enabled, spindle_rpm, target_motor_rpm, output_duty) = compute_output(
            timed_out,
            raw_duty_fine,
            raw_duty_fine_uncorrected,
            min_enable_duty,
            safety_shutdown,
            has_error,
        );

        // 4. Update enable pin
        update_enable(&mut enable_pin, enabled);

        // 5. Set PWM output duty cycle to ESCON
        let compare_value = (((output_duty as u32) * (pwm_top as u32) + 500) / 1000) as u16;
        let _ = pwm_out.set_duty_cycle(compare_value);

        // 6. Read actual speed from speed_measure_task (T-Method: high-resolution)
        let actual_rpm = crate::tasks::speed_measure::MEASURED_RPM.load(Ordering::SeqCst);
        let frequency_hz = (actual_rpm * config::ESCON_PULSES_PER_REV) / 60;

        // 7. Check for ESCON alert (active-low: LOW = alert, HIGH = OK)
        let escon_alert_active = escon_alert.is_low();

        // 8. Stall detection (suppressed during calibration)
        let (_stall_status, stall_detected) = run_stall_detection(
            &mut stall_detector,
            &mut was_cal_active,
            target_motor_rpm,
            actual_rpm,
            now_ms,
            &stall_config,
        );

        // 8b. Stabilization tracking
        let (stab_status, stab_time_ms) =
            stabilization_tracker.check(target_motor_rpm, actual_rpm, now_ms);

        // 9. Report errors to centralized error state
        let error_active = report_errors(
            &stall_detector,
            stall_detected,
            target_motor_rpm,
            now_ms,
            escon_alert_active,
        );

        // 10. Read current from current_monitor task
        let current_ma = get_current_ma();

        // 11. Debug output every 50 loops (~1Hz)
        if loop_count % 50 == 0 {
            let actual_spindle_rpm = (actual_rpm * config::BELT_RATIO_X1000 + 500) / 1000;
            defmt::debug!(
                "[{}ms] IN {}.{:02}% | MOTOR req={} act={}({}Hz) | SPINDLE req={} act={} | OUT {}.{}% En={} Err={} Alert={} Cur={}mA",
                now_ms,
                raw_duty_fine_uncorrected / 100,
                raw_duty_fine_uncorrected % 100,
                target_motor_rpm,
                actual_rpm,
                frequency_hz,
                spindle_rpm,
                actual_spindle_rpm,
                output_duty / 10,
                output_duty % 10,
                enabled as u8,
                error_active as u8,
                escon_alert_active as u8,
                current_ma
            );
        }

        // 12. Log stabilization event
        log_stabilization(stab_status, stab_time_ms, &mut stabilization_tracker);

        // 13. Update error output GPIO
        update_error_output(&mut error_out, error_active);

        // 14. Update display data
        let status = build_display_status(
            spindle_rpm,
            actual_rpm,
            current_ma,
            error_active,
            enabled,
            stab_time_ms,
        );
        update_display(status);

        // Wait for next control interval
        Timer::after(Duration::from_millis(CONTROL_INTERVAL_MS)).await;
    }
}

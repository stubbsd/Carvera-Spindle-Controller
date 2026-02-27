//! Shared state for inter-task communication.
//!
//! Uses Embassy Watch channel for display data and atomic variables for real-time control.

#[cfg(feature = "embedded")]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
#[cfg(feature = "embedded")]
use embassy_sync::watch::Watch;

#[cfg(feature = "embedded")]
use crate::calibration::CalibrationStatus;
#[cfg(feature = "embedded")]
use crate::display::DisplayStatus;
use crate::display::ErrorType;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ============================================================================
// Task Heartbeat Tracking (for watchdog health monitoring)
// ============================================================================
//
// Each critical task stores a wrapping millisecond timestamp every iteration.
// The watchdog task checks that all heartbeats are recent before feeding
// the hardware watchdog. Uses AtomicU32 (wrapping ms) since AtomicU64 is
// not available on thumbv8m.

/// Heartbeat timestamp from spindle_control task (20ms interval)
pub static HEARTBEAT_SPINDLE_CONTROL: AtomicU32 = AtomicU32::new(0);

/// Heartbeat timestamp from current_monitor task (100ms interval)
pub static HEARTBEAT_CURRENT_MONITOR: AtomicU32 = AtomicU32::new(0);

/// Heartbeat timestamp from pwm_input task (~32ms averaging window)
pub static HEARTBEAT_PWM_INPUT: AtomicU32 = AtomicU32::new(0);

/// Heartbeat timestamp from speed_measure task (edge-driven)
pub static HEARTBEAT_SPEED_MEASURE: AtomicU32 = AtomicU32::new(0);

/// Record a heartbeat for a task (stores current millisecond timestamp).
pub fn heartbeat(target: &AtomicU32, now_ms: u64) {
    target.store(now_ms as u32, Ordering::SeqCst);
}

/// Check if a heartbeat is recent (within max_age_ms of now).
/// Handles u32 wrapping correctly for monotonic timestamps.
pub fn is_heartbeat_recent(target: &AtomicU32, now_ms: u64, max_age_ms: u32) -> bool {
    let last = target.load(Ordering::SeqCst);
    let now_u32 = now_ms as u32;
    let elapsed = now_u32.wrapping_sub(last);
    elapsed <= max_age_ms
}

// ============================================================================
// Atomic Variables for Real-Time Control
// ============================================================================

/// Spindle enable state (true = enabled)
pub static ENABLED: AtomicBool = AtomicBool::new(false);

/// Current reading in mA (shared between current_monitor and spindle_control)
pub static CURRENT_MA: AtomicU32 = AtomicU32::new(0);

/// Calibration sequence detection active (suppresses stall detection)
pub static CAL_SEQUENCE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Calibration recording/dump/clear in progress (bypasses cal table in interpolate_rpm)
pub static CAL_RECORDING: AtomicBool = AtomicBool::new(false);

/// Get current reading in mA
pub fn get_current_ma() -> u32 {
    CURRENT_MA.load(Ordering::SeqCst)
}

/// Set current reading in mA
pub fn set_current_ma(value: u32) {
    CURRENT_MA.store(value, Ordering::SeqCst)
}

// ============================================================================
// Centralized Error State
// ============================================================================
//
// Per-source error flags with priority-based arbitration.
// Priority order: Overcurrent > Thermal > Stall > EsconAlert > None
//
// Overcurrent and Thermal are permanent latches (only power cycle clears).
// Stall follows the StallDetector's two-phase latch behavior.
// ESCON alert follows pin state (not latched).

/// Permanent safety shutdown latch. Once set, only a power cycle clears it.
/// Set by overcurrent or thermal errors.
static SAFETY_SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Per-source error flags
static OVERCURRENT_ERROR: AtomicBool = AtomicBool::new(false);
static THERMAL_ERROR: AtomicBool = AtomicBool::new(false);
static STALL_ALERT: AtomicBool = AtomicBool::new(false);
static ESCON_ALERT_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Stall visually latched (display shows "Stall" even after alert released)
static STALL_LATCHED: AtomicBool = AtomicBool::new(false);

/// Report overcurrent error. Permanently latches SAFETY_SHUTDOWN.
pub fn report_overcurrent() {
    OVERCURRENT_ERROR.store(true, Ordering::SeqCst);
    SAFETY_SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Report thermal error. Permanently latches SAFETY_SHUTDOWN.
pub fn report_thermal() {
    THERMAL_ERROR.store(true, Ordering::SeqCst);
    SAFETY_SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Report stall alert state. Follows detector state (not permanently latched).
pub fn report_stall_alert(active: bool) {
    STALL_ALERT.store(active, Ordering::SeqCst);
}

/// Report stall visual latch state (for display: shows "Stall" vs "StallCleared").
pub fn report_stall_latched(latched: bool) {
    STALL_LATCHED.store(latched, Ordering::SeqCst);
}

/// Report ESCON alert state. Follows pin state (not latched).
pub fn report_escon_alert(active: bool) {
    ESCON_ALERT_ACTIVE.store(active, Ordering::SeqCst);
}

/// Check if a permanent safety shutdown is latched (overcurrent or thermal).
pub fn is_safety_shutdown() -> bool {
    SAFETY_SHUTDOWN.load(Ordering::SeqCst)
}

/// Check if any error source is currently active.
pub fn any_error_active() -> bool {
    OVERCURRENT_ERROR.load(Ordering::SeqCst)
        || THERMAL_ERROR.load(Ordering::SeqCst)
        || STALL_ALERT.load(Ordering::SeqCst)
        || ESCON_ALERT_ACTIVE.load(Ordering::SeqCst)
}

/// Get the highest-priority active error type for display and GPIO output.
/// Priority: Overcurrent > Thermal > Stall > EsconAlert > None
pub fn get_active_error_type() -> ErrorType {
    if OVERCURRENT_ERROR.load(Ordering::SeqCst) {
        return ErrorType::Overcurrent;
    }
    if THERMAL_ERROR.load(Ordering::SeqCst) {
        return ErrorType::Thermal;
    }
    // Stall display: if stall_latched but alert not active, show StallCleared
    let stall_latched = STALL_LATCHED.load(Ordering::SeqCst);
    let stall_alert = STALL_ALERT.load(Ordering::SeqCst);
    if stall_alert {
        return ErrorType::Stall;
    }
    if stall_latched {
        return ErrorType::StallCleared;
    }
    if ESCON_ALERT_ACTIVE.load(Ordering::SeqCst) {
        return ErrorType::EsconAlert;
    }
    ErrorType::None
}

// ============================================================================
// Watch Channel for Display Data (Embassy only)
// ============================================================================

/// Watch channel for display status updates.
/// The display task subscribes to this to get the latest status snapshot.
/// Other tasks send updates here when values change.
#[cfg(feature = "embedded")]
pub static DISPLAY_DATA: Watch<CriticalSectionRawMutex, DisplayStatus, 2> = Watch::new();

/// Initialize display data with defaults
#[cfg(feature = "embedded")]
pub fn init_display_data() {
    DISPLAY_DATA.sender().send(DisplayStatus::default());
}

/// Update display data (called by control tasks)
#[cfg(feature = "embedded")]
pub fn update_display(status: DisplayStatus) {
    DISPLAY_DATA.sender().send(status);
}

// ============================================================================
// Watch Channel for Calibration Status (Embassy only)
// ============================================================================

/// Watch channel for calibration status updates.
/// The LCD task subscribes to this for calibration display mode.
#[cfg(feature = "embedded")]
pub static CAL_STATUS: Watch<CriticalSectionRawMutex, CalibrationStatus, 2> = Watch::new();

/// Update calibration status (called by calibration task)
#[cfg(feature = "embedded")]
pub fn update_cal_status(status: CalibrationStatus) {
    CAL_STATUS.sender().send(status);
}

// ============================================================================
// Configuration Constants
// ============================================================================

/// ESCON controller configuration - MUST MATCH ESCON STUDIO SETTINGS!
///
/// These values define how the Pico translates between Carvera's PWM signals
/// and the ESCON motor controller. Adjust them to match your ESCON Studio
/// configuration and motor capabilities.
pub mod config {
    // ========================================================================
    // PWM Duty Cycle Range (ESCON input range)
    // ========================================================================

    /// Minimum PWM duty cycle to ESCON (0.1% units, 100 = 10.0%)
    /// Below this threshold, spindle is disabled.
    /// ESCON typically uses 10% as minimum set value.
    pub const PWM_MIN_DUTY: u16 = 100;

    /// Maximum PWM duty cycle to ESCON (0.1% units, 900 = 90.0%)
    /// Above this threshold, output is clamped.
    /// ESCON typically uses 90% as maximum set value.
    pub const PWM_MAX_DUTY: u16 = 900;

    // ========================================================================
    // Speed Range (must match ESCON Studio configuration)
    // ========================================================================

    /// Speed at minimum duty cycle (ESCON Studio "Speed at 10%" setting)
    /// Set to 0 for full range down to stop, or set higher (e.g., 2000)
    /// if your motor doesn't run well below a certain speed.
    pub const MIN_RPM: u32 = 0;

    /// Speed at maximum duty cycle (ESCON Studio "Speed at 90%" setting)
    pub const MAX_RPM: u32 = 12500;

    /// Maximum spindle RPM (motor max × belt ratio).
    /// Must match Carvera firmware config: `spindle.max_rpm` (default 15000 for
    /// Carvera, 13000 for Carvera Air). When using open-loop mode with ESCON,
    /// set `spindle.max_rpm` to this computed value for correct PWM scaling.
    ///
    /// Carvera open-loop mode uses LINEAR formula: duty = (S / max_rpm) × 100%
    /// ESCON interprets: motor_rpm = (duty - 10%) / 80% × 12500
    /// Derived: MAX_RPM × BELT_RATIO_X1000 / 1000 = 12500 × 1635 / 1000 = 20437
    pub const CARVERA_SPINDLE_MAX_RPM: u32 = MAX_RPM * BELT_RATIO_X1000 / 1000;

    // ========================================================================
    // Current Monitoring (ESCON Analog Output 1)
    // ========================================================================

    /// Current at ADC max (3.3V) in mA - ESCON analog output setting (5.2A)
    pub const CURRENT_AT_3V3_MA: u32 = 5200;

    /// Encoder pulses per rotation from ESCON speed output
    pub const ESCON_PULSES_PER_REV: u32 = 4;

    /// Belt drive ratio (motor to spindle) x1000 for integer math
    /// Carvera stock ratio is 1.635:1 (from config.default spindle.acc_ratio)
    /// Motor RPM * 1635 / 1000 = Spindle RPM
    pub const BELT_RATIO_X1000: u32 = 1635;

    /// Stall if actual < 30% of requested
    pub const STALL_THRESHOLD_PCT: u32 = 30;

    /// Stall condition must persist for 100ms
    pub const STALL_DEBOUNCE_MS: u64 = 100;

    /// Speed must stay above threshold for 300ms to clear stall
    pub const STALL_RECOVERY_MS: u64 = 300;

    /// Base grace period after speed change
    pub const BASE_GRACE_MS: u64 = 200;

    /// Additional grace ms per 1000 RPM
    pub const RPM_GRACE_FACTOR: u64 = 15;

    /// Speed change rate threshold - changes slower than this are considered noise (RPM/second)
    /// 500 RPM/sec means a 5000 RPM change must happen in <10 seconds to be considered "real"
    /// This prevents tiny jitter (1-2 RPM fluctuations) from resetting the grace period timer
    pub const SPEED_CHANGE_RATE_THRESHOLD: u32 = 500;

    /// Overcurrent threshold (100% of CURRENT_AT_3V3_MA = full ADC scale).
    /// The ESCON's own current loop limits at 5A with I2t thermal protection.
    /// This threshold is a last-resort backup: triggers only if current exceeds
    /// the analog output range, indicating ESCON protection failure.
    pub const OVERCURRENT_THRESHOLD_PCT: u32 = 100;

    /// Overcurrent must persist 500ms before triggering.
    /// The ESCON's current control loop responds in microseconds, so any
    /// transient overshoot settles quickly. A sustained 500ms above full-scale
    /// indicates a genuine fault, not normal control loop dynamics.
    pub const OVERCURRENT_DEBOUNCE_MS: u64 = 500;

    /// Hardware watchdog timeout
    pub const WATCHDOG_TIMEOUT_MS: u64 = 1000;

    /// Delay before enabling spindle on startup
    pub const STARTUP_DELAY_MS: u64 = 100;

    /// MCU temperature limit (Celsius)
    pub const THERMAL_SHUTDOWN_C: i32 = 70;

    /// Show config values on display for this duration
    pub const STARTUP_CONFIG_DISPLAY_MS: u64 = 3000;

    /// System clock frequency (default for RP2350)
    pub const CLOCK_HZ: u32 = 150_000_000;

    /// Minimum RPM to enable spindle output.
    /// Below this threshold, spindle is disabled. Set above Carvera's idle offset
    /// (~2.44% duty = S500) to prevent spindle creep on power-up.
    pub const MIN_ENABLE_RPM: u32 = 750;

    /// Expected PWM input frequency from Carvera (analog mode with pwm_period=50)
    /// Carvera: config-set sd spindle.pwm_period 50 → 1,000,000 / 50 = 20,000 Hz
    pub const PWM_INPUT_FREQ_HZ: u32 = 20_000;

    /// PWM output frequency to ESCON (unchanged at 1kHz)
    pub const PWM_OUTPUT_FREQ_HZ: u32 = 1_000;
}

// ============================================================================
// GPIO Pin Assignments
// ============================================================================

/// GPIO pin assignments for all peripherals
pub mod pins {
    /// PWM Input from Carvera (speed request)
    pub const PWM_INPUT: u8 = 3;

    /// PWM Output to ESCON (speed control)
    pub const PWM_OUTPUT: u8 = 4;

    /// Enable output to ESCON
    pub const ENABLE: u8 = 5;

    /// ESCON alert input (digital)
    pub const ESCON_ALERT: u8 = 8;

    /// Speed input from ESCON (4 ppr)
    pub const SPEED_INPUT: u8 = 9;

    /// Error output to Carvera
    pub const ERROR_OUTPUT: u8 = 10;

    /// Onboard status LED
    pub const STATUS_LED: u8 = 25;

    /// ADC input for current monitoring
    pub const CURRENT_ADC: u8 = 26;

    // LCD Display (HD44780 16x2 with RGB backlight)
    /// LCD Register Select
    pub const LCD_RS: u8 = 16;
    /// LCD Enable
    pub const LCD_E: u8 = 17;
    /// LCD Data bit 4
    pub const LCD_D4: u8 = 18;
    /// LCD Data bit 5 (moved from 19 - was stuck HIGH)
    pub const LCD_D5: u8 = 22;
    /// LCD Data bit 6
    pub const LCD_D6: u8 = 20;
    /// LCD Data bit 7
    pub const LCD_D7: u8 = 21;
    /// LCD RGB Backlight - Red (PWM)
    pub const LCD_RED: u8 = 14;
    /// LCD RGB Backlight - Green (PWM)
    pub const LCD_GREEN: u8 = 12;
    /// LCD RGB Backlight - Blue (PWM)
    pub const LCD_BLUE: u8 = 13;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Reset all error atomics to prevent cross-test pollution.
    fn reset_error_state() {
        SAFETY_SHUTDOWN.store(false, Ordering::SeqCst);
        OVERCURRENT_ERROR.store(false, Ordering::SeqCst);
        THERMAL_ERROR.store(false, Ordering::SeqCst);
        STALL_ALERT.store(false, Ordering::SeqCst);
        STALL_LATCHED.store(false, Ordering::SeqCst);
        ESCON_ALERT_ACTIVE.store(false, Ordering::SeqCst);
        CURRENT_MA.store(0, Ordering::SeqCst);
    }

    // --- current_ma round-trip ---

    #[test]
    #[serial]
    fn test_current_ma_round_trip() {
        reset_error_state();
        set_current_ma(4200);
        assert_eq!(get_current_ma(), 4200);
        set_current_ma(0);
        assert_eq!(get_current_ma(), 0);
        set_current_ma(u32::MAX);
        assert_eq!(get_current_ma(), u32::MAX);
        // cleanup
        set_current_ma(0);
    }

    // --- report_overcurrent latching ---

    #[test]
    #[serial]
    fn test_overcurrent_latches_safety_shutdown() {
        reset_error_state();
        assert!(!is_safety_shutdown());
        report_overcurrent();
        assert!(is_safety_shutdown());
        assert!(any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::Overcurrent);
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_overcurrent_latch_is_permanent() {
        reset_error_state();
        report_overcurrent();
        // Clearing stall/escon does not clear overcurrent
        report_stall_alert(false);
        report_escon_alert(false);
        assert!(is_safety_shutdown());
        assert_eq!(get_active_error_type(), ErrorType::Overcurrent);
        reset_error_state();
    }

    // --- report_thermal latching ---

    #[test]
    #[serial]
    fn test_thermal_latches_safety_shutdown() {
        reset_error_state();
        assert!(!is_safety_shutdown());
        report_thermal();
        assert!(is_safety_shutdown());
        assert!(any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::Thermal);
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_thermal_latch_is_permanent() {
        reset_error_state();
        report_thermal();
        report_stall_alert(false);
        report_escon_alert(false);
        assert!(is_safety_shutdown());
        assert_eq!(get_active_error_type(), ErrorType::Thermal);
        reset_error_state();
    }

    // --- stall alert (not permanently latched) ---

    #[test]
    #[serial]
    fn test_stall_alert_follows_state() {
        reset_error_state();
        report_stall_alert(true);
        assert!(any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::Stall);

        report_stall_alert(false);
        assert!(!STALL_ALERT.load(Ordering::SeqCst));
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_stall_latched_shows_stall_cleared() {
        reset_error_state();
        // Stall alert released but visual latch still active
        report_stall_alert(false);
        report_stall_latched(true);
        assert_eq!(get_active_error_type(), ErrorType::StallCleared);

        report_stall_latched(false);
        assert_eq!(get_active_error_type(), ErrorType::None);
        reset_error_state();
    }

    // --- escon alert (follows pin state) ---

    #[test]
    #[serial]
    fn test_escon_alert_follows_state() {
        reset_error_state();
        report_escon_alert(true);
        assert!(any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::EsconAlert);

        report_escon_alert(false);
        assert!(!any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::None);
        reset_error_state();
    }

    // --- priority ordering ---

    #[test]
    #[serial]
    fn test_overcurrent_has_highest_priority() {
        reset_error_state();
        report_escon_alert(true);
        report_stall_alert(true);
        report_thermal();
        report_overcurrent();
        // Overcurrent wins over all others
        assert_eq!(get_active_error_type(), ErrorType::Overcurrent);
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_thermal_beats_stall_and_escon() {
        reset_error_state();
        report_escon_alert(true);
        report_stall_alert(true);
        report_thermal();
        // Thermal wins over stall and escon
        assert_eq!(get_active_error_type(), ErrorType::Thermal);
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_stall_beats_escon() {
        reset_error_state();
        report_escon_alert(true);
        report_stall_alert(true);
        assert_eq!(get_active_error_type(), ErrorType::Stall);
        reset_error_state();
    }

    #[test]
    #[serial]
    fn test_no_errors_returns_none() {
        reset_error_state();
        assert!(!any_error_active());
        assert_eq!(get_active_error_type(), ErrorType::None);
    }

    // --- any_error_active OR behavior ---

    #[test]
    #[serial]
    fn test_any_error_active_or_behavior() {
        reset_error_state();
        assert!(!any_error_active());

        // Each individual error source triggers any_error_active
        report_overcurrent();
        assert!(any_error_active());
        reset_error_state();

        report_thermal();
        assert!(any_error_active());
        reset_error_state();

        report_stall_alert(true);
        assert!(any_error_active());
        reset_error_state();

        report_escon_alert(true);
        assert!(any_error_active());
        reset_error_state();
    }

    // --- heartbeat tests ---

    #[test]
    fn test_heartbeat_recent() {
        let target = AtomicU32::new(0);
        heartbeat(&target, 1000);
        assert!(is_heartbeat_recent(&target, 1050, 100));
        assert!(!is_heartbeat_recent(&target, 1200, 100));
    }

    #[test]
    fn test_heartbeat_wrapping() {
        let target = AtomicU32::new(0);
        // Simulate near u32::MAX
        heartbeat(&target, u32::MAX as u64 - 10);
        // 20ms later wraps around
        let now = u32::MAX as u64 + 10;
        assert!(is_heartbeat_recent(&target, now, 100));
    }
}

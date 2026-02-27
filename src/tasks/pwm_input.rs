//! PIO-based PWM duty cycle measurement for Carvera input.
//!
//! Uses the RP2350's PIO state machine to measure the HIGH time and full period
//! of each PWM cycle from the Carvera CNC. This provides precise cycle-by-cycle
//! measurement, eliminating window misalignment jitter from the previous
//! hardware counter approach.
//!
//! ## Why PIO Instead of Hardware PWM Counter?
//!
//! The previous approach used `Pwm::new_input(InputMode::Level)` which counts
//! HIGH clock cycles over a fixed 20ms window. Problems:
//! 1. **Window misalignment**: 20ms doesn't align with 1ms PWM cycles
//! 2. **Edge timing uncertainty**: Partial cycles at window boundaries cause jitter
//! 3. **Resolution limit**: ~0.1% precision
//!
//! The PIO approach measures each complete PWM cycle individually, giving:
//! - Cycle-aligned measurements (no partial cycles)
//! - 0.01% resolution at 20kHz PWM with 150MHz PIO clock
//! - Stable readings when Carvera PWM is constant
//!
//! ## Hardware Resources
//! - Uses PIO0 SM1 (SM0 is used for speed measurement)
//! - Input pin: GPIO3 (Carvera PWM output)

use core::sync::atomic::{AtomicU32, Ordering};
use embassy_futures::select::{Either, select};
use embassy_rp::Peri;
use embassy_rp::gpio::Pull;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::{Common, Config, Direction, PioPin, ShiftDirection, StateMachine};
use embassy_time::{Duration, Timer};
use fixed::traits::ToFixed;

/// Measured duty cycle in 0-10000 scale (0.00% to 100.00%)
///
/// Other tasks read this via `MEASURED_DUTY.load(Ordering::SeqCst)`.
/// Updated after averaging AVERAGING_CYCLES PWM cycles (~32ms at 20kHz).
pub static MEASURED_DUTY: AtomicU32 = AtomicU32::new(0);

/// Clock divider: 150MHz / 1 = 150MHz (~6.7ns resolution)
/// At 20kHz PWM: ~7500 counts per period = 0.01% resolution
/// This provides high precision for accurate RPM display.
///
/// Previous value (8) gave only ~937 counts/period = 0.1% resolution,
/// causing S10000 to display as 10013 RPM due to quantization error.
const CLOCK_DIVIDER: u16 = 1;

/// Number of PWM cycles to average before updating MEASURED_DUTY.
/// 640 cycles at 20kHz = 32ms averaging window (same as original 32 cycles @ 1kHz).
/// This eliminates per-cycle jitter while maintaining responsiveness.
/// Control loop runs at 50Hz (20ms), so 32ms provides excellent smoothing.
const AVERAGING_CYCLES: u32 = 640;

/// Timeout for detecting no-signal condition.
/// If no PWM edges are detected within this time, MEASURED_DUTY is reset to 0.
/// Should be longer than one averaging window to avoid false positives.
const SIGNAL_TIMEOUT_MS: u64 = 100;

/// Expected PIO counts per PWM period at 20kHz with 150MHz clock.
/// 150_000_000 / 20_000 = 7500 counts.
/// PIO loop runs at 2 cycles per iteration, so expected count is 7500 / 2 = 3750.
/// We validate that total (high + low) is within 0.5x to 2x of this value.
const EXPECTED_PERIOD_COUNTS: u64 = 3750;
const MIN_PERIOD_COUNTS: u64 = EXPECTED_PERIOD_COUNTS / 2;
const MAX_PERIOD_COUNTS: u64 = EXPECTED_PERIOD_COUNTS * 2;

/// Number of consecutive FIFO timeouts before logging error and restarting PIO.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 10;

/// Set up the PIO state machine for PWM duty cycle measurement.
///
/// This function must be called from main() before spawning the pwm_input_task.
/// It configures the PIO program and pin settings.
///
/// # Arguments
/// * `common` - PIO common resources
/// * `sm` - State machine 1 (SM0 is used for speed measurement)
/// * `pin` - The GPIO pin to use for PWM input (GPIO3)
pub fn setup_pwm_input_pio<'a>(
    common: &mut Common<'a, PIO0>,
    sm: &mut StateMachine<'a, PIO0, 1>,
    pin: Peri<'a, impl PioPin>,
) {
    // PIO program: measure HIGH time and LOW time separately with symmetric timing
    //
    // Based on GitJer's proven PwmIn implementation:
    // https://github.com/GitJer/Some_RPI-Pico_stuff/tree/main/PwmIn
    //
    // Algorithm:
    // 1. Wait for signal to go LOW (ensure we start from known state)
    // 2. Wait for rising edge
    // 3. Count HIGH time with Y counter (2 cycles per iteration)
    // 4. Count LOW time with X counter (2 cycles per iteration)
    // 5. Push both values to FIFO
    // 6. Repeat
    //
    // CRITICAL: Both loops MUST have identical timing (2 cycles per iteration).
    // The previous implementation had 2 cycles for HIGH but 3 cycles for LOW,
    // causing 48.48% actual duty to measure as 58.5%.
    //
    // duty = high_time / (high_time + low_time)
    // Where high_time = ~Y and low_time = ~X (inverted counter values)
    let prg = pio_asm!(
        "start:",
        "    mov y, !null", // Y = 0xFFFFFFFF for high period counter
        "    mov x, !null", // X = 0xFFFFFFFF for low period counter
        "    wait 0 pin 0", // Wait for LOW (ensure clean start)
        "    wait 1 pin 0", // Wait for rising edge
        "timer_hp:",        // HIGH period loop (2 cycles per iteration)
        "    jmp y-- test", // Decrement Y, always succeeds (1 cycle)
        "test:",
        "    jmp pin timer_hp",  // Loop while pin is HIGH (1 cycle)
        "timer_lp:",             // LOW period loop (2 cycles per iteration)
        "    jmp pin timerstop", // Exit if pin went HIGH (1 cycle)
        "    jmp x-- timer_lp",  // Decrement X and loop while LOW (1 cycle)
        "timerstop:",
        "    mov isr, !y", // Push ~Y = high_time (inverted gives count)
        "    push noblock",
        "    mov isr, !x", // Push ~X = low_time
        "    push noblock",
        "    jmp start",
    );

    let program = common.load_program(&prg.program);

    // Configure the pin for PIO with pull-down (Carvera PWM is push-pull)
    let mut pio_pin = common.make_pio_pin(pin);
    pio_pin.set_pull(Pull::Down);

    // Set pin as input
    sm.set_pin_dirs(Direction::In, &[&pio_pin]);

    // Configure state machine
    let mut cfg = Config::default();
    cfg.use_program(&program, &[]);

    // Set input pin for 'wait' and 'in' instructions
    cfg.set_in_pins(&[&pio_pin]);
    // Set jump pin for conditional jumps (jmp pin)
    cfg.set_jmp_pin(&pio_pin);

    // Set clock divider for timing resolution
    // 150MHz / 1 = 150MHz -> ~6.7ns resolution
    // At 20kHz PWM: ~7500 counts per period = 0.01% resolution
    cfg.clock_divider = (CLOCK_DIVIDER as u32).to_fixed();

    // Configure shift register for pushing to FIFO
    cfg.shift_in.direction = ShiftDirection::Left;

    sm.set_config(&cfg);
}

/// PWM input measurement task using PIO.
///
/// This task:
/// 1. Reads HIGH count and period count from PIO FIFO
/// 2. Accumulates duty readings over AVERAGING_CYCLES cycles
/// 3. Publishes averaged duty via atomic for spindle_control_task to read
///
/// Cycle averaging eliminates per-cycle jitter (±0.1%) that occurs due to
/// edge detection timing variations at quantization boundaries.
///
/// **Important**: Call `setup_pwm_input_pio()` before spawning this task.
#[embassy_executor::task]
pub async fn pwm_input_task(mut sm: StateMachine<'static, PIO0, 1>) {
    defmt::info!(
        "PWM input task starting (PIO-based, {}-cycle avg)",
        AVERAGING_CYCLES
    );

    // Enable the state machine
    sm.set_enable(true);

    // Log configuration for diagnostics
    let clock_mhz = 150 / CLOCK_DIVIDER;
    defmt::info!(
        "PIO PWM measurement active: divider={}, clock={}MHz, avg={}cycles",
        CLOCK_DIVIDER,
        clock_mhz,
        AVERAGING_CYCLES
    );

    // Cycle averaging state
    let mut duty_accumulator: u64 = 0;
    let mut cycle_count: u32 = 0;
    let mut consecutive_timeouts: u32 = 0;

    loop {
        // Record heartbeat for watchdog health monitoring
        let now_ms = embassy_time::Instant::now().as_millis();
        crate::state::heartbeat(&crate::state::HEARTBEAT_PWM_INPUT, now_ms);

        // Read high_count from FIFO with timeout
        // When signal disappears (0% duty), the PIO blocks waiting for rising edge
        // The timeout detects this and resets MEASURED_DUTY to 0
        let timeout = Timer::after(Duration::from_millis(SIGNAL_TIMEOUT_MS));
        let high_count = match select(sm.rx().wait_pull(), timeout).await {
            Either::First(count) => count,
            Either::Second(_) => {
                // Timeout: no signal detected (expected when spindle stopped)
                MEASURED_DUTY.store(0, Ordering::SeqCst);
                duty_accumulator = 0;
                cycle_count = 0;
                consecutive_timeouts = 0;
                continue;
            }
        };

        // Read period_count from FIFO (should be available immediately after high_count)
        let timeout = Timer::after(Duration::from_millis(10)); // Short timeout for second value
        let period_count = match select(sm.rx().wait_pull(), timeout).await {
            Either::First(count) => count,
            Either::Second(_) => {
                consecutive_timeouts += 1;
                if consecutive_timeouts >= MAX_CONSECUTIVE_TIMEOUTS {
                    defmt::error!(
                        "PWM input: {} consecutive FIFO timeouts, restarting PIO",
                        consecutive_timeouts
                    );
                    sm.set_enable(false);
                    sm.restart();
                    sm.set_enable(true);
                    consecutive_timeouts = 0;
                    duty_accumulator = 0;
                    cycle_count = 0;
                } else {
                    defmt::warn!("PWM input: missing period count");
                }
                continue;
            }
        };

        // Both FIFO reads succeeded - reset consecutive timeout counter
        consecutive_timeouts = 0;

        // Calculate duty cycle from HIGH time and LOW time
        // Both values are already inverted in PIO (mov isr, !y and mov isr, !x)
        // So high_count IS the high_time and low_count IS the low_time
        let high_time = high_count;
        let low_time = period_count; // This is actually low_time from the new PIO program
        let period = high_time as u64 + low_time as u64;

        // Sanity check: total period must be within 0.5x to 2x expected counts
        // At 20kHz with 150MHz PIO clock and 2-cycle loop: ~3750 counts per period
        if period < MIN_PERIOD_COUNTS || period > MAX_PERIOD_COUNTS {
            continue;
        }

        if period > 0 {
            // Calculate duty as 0-10000 scale (0.00% to 100.00%)
            let duty = ((high_time as u64 * 10000) / period) as u32;

            // Accumulate for averaging
            duty_accumulator += duty as u64;
            cycle_count += 1;

            // Publish averaged duty after AVERAGING_CYCLES
            if cycle_count >= AVERAGING_CYCLES {
                let avg_duty = (duty_accumulator / cycle_count as u64) as u32;
                MEASURED_DUTY.store(avg_duty.min(10000), Ordering::SeqCst);

                // Reset accumulator for next averaging window
                duty_accumulator = 0;
                cycle_count = 0;
            }
        }
    }
}

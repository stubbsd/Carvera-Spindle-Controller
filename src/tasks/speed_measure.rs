//! PIO-based speed measurement with hardware debouncing.
//!
//! Uses the RP2350's PIO state machine to implement a hardware debouncer
//! based on GitJer's proven algorithm. This filters noise at the hardware
//! level before it ever reaches software, eliminating false edge detections
//! that cause wild RPM fluctuations.
//!
//! ## How the PIO Debouncer Works
//!
//! The algorithm requires the signal to be stable for N clock cycles before
//! accepting a transition:
//!
//! 1. Wait for pin to go HIGH
//! 2. Start countdown (31 iterations)
//! 3. Each iteration: check if pin is still HIGH
//!    - If pin bounced back LOW -> restart from step 1
//!    - If still HIGH -> decrement counter
//! 4. Counter reaches 0 -> transition confirmed, signal IRQ
//! 5. Wait for pin to go LOW (repeat similar process)
//!
//! Key insight: Noise glitches are short (nanoseconds to microseconds).
//! Real encoder pulses are long (~1ms at 6100 RPM with 4 PPR).
//! By requiring stability, glitches are rejected.
//!
//! ## Timing Calculation
//!
//! With PIO at 125 MHz system clock:
//! - 62 cycles (31 iterations x 2 instructions) per debounce check
//! - Debounce time = 62 x clock_divider / 125,000,000
//!
//! | Clock Divider | Debounce Time | % of 13000 RPM pulse |
//! |---------------|---------------|----------------------|
//! | 64            | ~32 us        | 5.5%                 |
//!
//! At 13000 RPM with 4 PPR: pulse HIGH time ~577 us
//! 32 us debounce is only 5.5% of pulse width - safe margin.

use core::sync::atomic::{AtomicU32, Ordering};
use embassy_rp::Peri;
use embassy_rp::gpio::Pull;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::{
    Common, Config, Direction as PioDirection, Irq, PioPin, ShiftDirection, StateMachine,
};
use embassy_time::{Duration, Instant};
use fixed::traits::ToFixed;

use crate::state::config;
use crate::{CircularBuffer, periods_to_rpm};

/// Number of revolution periods to store for median filtering.
/// 4 samples gives good noise rejection while keeping latency low.
const REV_PERIOD_BUFFER_SIZE: usize = 4;

/// Timeout for no pulses (motor stopped) in milliseconds
const NO_PULSE_TIMEOUT_MS: u64 = 500;

/// Global atomic for sharing measured RPM with other tasks
pub static MEASURED_RPM: AtomicU32 = AtomicU32::new(0);

/// Global atomic for sharing measured frequency (mHz) with other tasks
pub static MEASURED_FREQ_MHZ: AtomicU32 = AtomicU32::new(0);

/// Clock divider for PIO debouncer.
/// With divider=64 at 125MHz: debounce time = 62 * 64 / 125,000,000 = ~32 us
///
/// Tuning guide:
/// - If still seeing noise spikes: increase divider (e.g., 128 for ~64 us)
/// - If missing real pulses at high RPM: decrease divider (e.g., 32 for ~16 us)
pub const CLOCK_DIVIDER: u16 = 64;

/// Set up the PIO state machine for debounced speed measurement.
///
/// This function must be called from main() before spawning the speed_measure_task.
/// It configures the PIO program and pin settings.
///
/// # Arguments
/// * `common` - PIO common resources
/// * `sm` - State machine 0
/// * `pin` - The GPIO pin to use for speed input (GPIO9)
pub fn setup_speed_measure_pio<'a>(
    common: &mut Common<'a, PIO0>,
    sm: &mut StateMachine<'a, PIO0, 0>,
    pin: Peri<'a, impl PioPin>,
) {
    // PIO assembly for edge debouncing (GitJer's algorithm)
    //
    // The program flow:
    // 1. wait_high: Wait for pin to go HIGH
    // 2. set x, 31: Load debounce counter
    // 3. check_still_high: Loop checking pin is still HIGH
    //    - If pin bounced LOW -> restart at wait_high
    //    - If still HIGH -> decrement counter
    // 4. When counter hits 0: stable HIGH detected, trigger IRQ
    // 5. wait_for_fall: Wait for pin to go LOW
    // 6. set x, 31: Load debounce counter for falling edge
    // 7. check_still_low: Loop checking pin is still LOW
    //    - If pin bounced HIGH -> restart at wait_for_fall
    //    - If still LOW -> decrement counter
    // 8. When counter hits 0: stable LOW detected, loop back to wait_high
    let prg = pio_asm!(
        // Wait for rising edge with debounce
        "wait_high:",
        "    wait 1 pin 0", // Wait for pin to go HIGH
        "    set x, 31",    // Load debounce counter (31 iterations)
        "check_still_high:",
        "    jmp pin high_ok", // If still HIGH, continue countdown
        "    jmp wait_high",   // Bounced back to LOW, restart
        "high_ok:",
        "    jmp x-- check_still_high", // Decrement counter, loop if not zero
        // Pin has been stable HIGH for 62 cycles - valid rising edge!
        "    irq 0 rel", // Signal IRQ to CPU (relative to state machine)
        // Now wait for falling edge with debounce (to complete the cycle)
        "wait_for_fall:",
        "    wait 0 pin 0", // Wait for pin to go LOW
        "    set x, 31",    // Load debounce counter
        "check_still_low:",
        "    jmp pin wait_for_fall", // If went back HIGH, restart falling wait
        "    jmp x-- check_still_low", // Decrement counter, loop if not zero
        // Pin has been stable LOW - cycle complete, loop back
        "    jmp wait_high",
    );

    let program = common.load_program(&prg.program);

    // Configure the pin for PIO with pull-up (ESCON DOUT is open-drain)
    let mut pio_pin = common.make_pio_pin(pin);
    pio_pin.set_pull(Pull::Up);

    // Set pin as input
    sm.set_pin_dirs(PioDirection::In, &[&pio_pin]);

    // Configure state machine
    let mut cfg = Config::default();
    cfg.use_program(&program, &[]);

    // Set input pin for 'wait' and 'in' instructions
    cfg.set_in_pins(&[&pio_pin]);
    // Set jump pin for conditional jumps
    cfg.set_jmp_pin(&pio_pin);

    // Set clock divider for debounce timing
    // debounce_time = 62 * clock_divider / 125_000_000 seconds
    cfg.clock_divider = (CLOCK_DIVIDER as u32).to_fixed();

    // Configure shift register (not really used but required)
    cfg.shift_in.direction = ShiftDirection::Left;

    sm.set_config(&cfg);
}

/// Speed measurement task using PIO hardware debouncing.
///
/// This task:
/// 1. Waits for IRQ signals from PIO (debounced rising edges)
/// 2. Measures time between edges using Embassy's Instant timer
/// 3. Stores periods in a circular buffer
/// 4. Calculates RPM using median filtering
/// 5. Publishes RPM via atomic for other tasks to read
///
/// **Important**: Call `setup_speed_measure_pio()` before spawning this task.
#[embassy_executor::task]
pub async fn speed_measure_task(
    mut sm0: StateMachine<'static, PIO0, 0>,
    mut irq0: Irq<'static, PIO0, 0>,
) {
    defmt::info!("Speed measurement task starting (PIO hardware debounced)");

    // Enable the state machine
    sm0.set_enable(true);

    // Calculate and log debounce time for diagnostics
    let debounce_us = (62u32 * CLOCK_DIVIDER as u32) / 125;
    defmt::info!(
        "PIO debouncer active: divider={}, debounce=~{}us",
        CLOCK_DIVIDER,
        debounce_us
    );

    // Revolution-based measurement: store timestamps of the last PPR edges.
    // Each revolution period = time from edge[i] to edge[i + PPR], spanning
    // one complete revolution. This eliminates per-pulse asymmetry bias.
    let ppr = config::ESCON_PULSES_PER_REV as usize; // 4
    let mut edge_timestamps = [0u64; 4]; // Ring buffer of edge timestamps (us)
    let mut ts_idx: usize = 0; // Current index into ring buffer
    let mut pulse_count: u32 = 0; // Total pulses seen since reset

    // Circular buffer for revolution period measurements (in microseconds)
    let mut rev_periods: CircularBuffer<REV_PERIOD_BUFFER_SIZE> = CircularBuffer::new();

    loop {
        // Record heartbeat for watchdog health monitoring
        let hb_now = Instant::now().as_millis();
        crate::state::heartbeat(&crate::state::HEARTBEAT_SPEED_MEASURE, hb_now);

        // Wait for debounced rising edge (PIO IRQ 0) with timeout
        let timeout = Duration::from_millis(NO_PULSE_TIMEOUT_MS);

        match embassy_time::with_timeout(timeout, irq0.wait()).await {
            Ok(()) => {
                // Debounced rising edge detected by PIO
                let now_us = Instant::now().as_micros();

                // Store timestamp in ring buffer (overwrites PPR-edges-ago value)
                let oldest_us = edge_timestamps[ts_idx];
                edge_timestamps[ts_idx] = now_us;
                ts_idx = (ts_idx + 1) % ppr;
                pulse_count += 1;

                // Need at least PPR+1 edges to compute one revolution period
                if pulse_count > ppr as u32 {
                    // Revolution period = time spanning PPR consecutive edges
                    let rev_period_us = (now_us - oldest_us) as u32;

                    // Sanity check for revolution periods:
                    // At 13000 RPM: rev period = 60_000_000 / 13000 = ~4615 us
                    // At 100 RPM: rev period = 60_000_000 / 100 = 600,000 us
                    if rev_period_us > 4000 && rev_period_us < 700_000 {
                        rev_periods.push(rev_period_us);

                        // Calculate RPM: period already spans 1 revolution
                        let rpm = periods_to_rpm(rev_periods.as_slice(), 1);
                        MEASURED_RPM.store(rpm, Ordering::SeqCst);

                        // Calculate frequency in mHz for diagnostics (per-pulse)
                        let pulse_period_us = rev_period_us / ppr as u32;
                        if pulse_period_us > 0 {
                            let freq_mhz = 1_000_000_000 / pulse_period_us;
                            MEASURED_FREQ_MHZ.store(freq_mhz, Ordering::SeqCst);
                        }
                    } else {
                        defmt::trace!("Rev period out of range: {}us", rev_period_us);
                    }
                }
            }
            Err(_) => {
                // Timeout - no pulse received, motor stopped
                if !rev_periods.is_empty() {
                    defmt::info!("No pulses for {}ms, motor stopped", NO_PULSE_TIMEOUT_MS);
                }
                MEASURED_RPM.store(0, Ordering::SeqCst);
                MEASURED_FREQ_MHZ.store(0, Ordering::SeqCst);
                rev_periods.clear();
                edge_timestamps = [0u64; 4];
                ts_idx = 0;
                pulse_count = 0;
            }
        }
    }
}

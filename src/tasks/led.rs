//! Status LED task.
//!
//! Blinks the onboard LED to indicate system status:
//! - Slow blink (~1Hz): Waiting, no active signal
//! - Fast blink (~4Hz): Active signal detected
//! - Solid: Error state

use embassy_rp::gpio::Output;
use embassy_time::{Duration, Timer};

use crate::state::{ENABLED, any_error_active, pins};

/// LED status patterns
const SLOW_BLINK_MS: u64 = 500; // 1Hz blink
const FAST_BLINK_MS: u64 = 125; // 4Hz blink

/// Status LED task
#[embassy_executor::task]
pub async fn led_task(mut led: Output<'static>) {
    defmt::info!("Status LED ready on GPIO{}", pins::STATUS_LED);

    loop {
        let error = any_error_active();
        let enabled = ENABLED.load(core::sync::atomic::Ordering::SeqCst);

        if error {
            // Solid on for error
            led.set_high();
            Timer::after(Duration::from_millis(100)).await;
        } else if enabled {
            // Fast blink when active
            led.toggle();
            Timer::after(Duration::from_millis(FAST_BLINK_MS)).await;
        } else {
            // Slow blink when idle
            led.toggle();
            Timer::after(Duration::from_millis(SLOW_BLINK_MS)).await;
        }
    }
}

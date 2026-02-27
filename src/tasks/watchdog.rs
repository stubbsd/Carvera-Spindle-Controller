//! Hardware watchdog feeder task with health monitoring.
//!
//! Feeds the hardware watchdog every 250ms, but only if all critical tasks
//! have recorded recent heartbeats. If any task stops updating its heartbeat
//! (due to a hang or deadlock), the watchdog will starve and reset the system.

use embassy_rp::watchdog::Watchdog;
use embassy_time::{Duration, Instant, Timer};

use crate::state::{
    HEARTBEAT_CURRENT_MONITOR, HEARTBEAT_PWM_INPUT, HEARTBEAT_SPEED_MEASURE,
    HEARTBEAT_SPINDLE_CONTROL, config, is_heartbeat_recent,
};

/// Watchdog feed interval (must be less than watchdog timeout)
const FEED_INTERVAL_MS: u64 = 250;

/// Maximum allowed age for spindle_control heartbeat (2x its 20ms interval)
const SPINDLE_CONTROL_MAX_AGE_MS: u32 = 500;

/// Maximum allowed age for current_monitor heartbeat (2x its 100ms interval)
const CURRENT_MONITOR_MAX_AGE_MS: u32 = 500;

/// Maximum allowed age for pwm_input heartbeat (2x its ~32ms window + margin)
const PWM_INPUT_MAX_AGE_MS: u32 = 500;

/// Maximum allowed age for speed_measure heartbeat.
/// Speed measurement is edge-driven and may not fire when motor is stopped,
/// so we use a generous timeout.
const SPEED_MEASURE_MAX_AGE_MS: u32 = 1000;

/// Grace period after boot before requiring heartbeats (let tasks start up).
const STARTUP_GRACE_MS: u64 = 2000;

/// Hardware watchdog feeder task with health monitoring
#[embassy_executor::task]
pub async fn watchdog_task(mut watchdog: Watchdog) {
    defmt::info!("Watchdog task started");

    let timeout = Duration::from_millis(config::WATCHDOG_TIMEOUT_MS);

    // Pause watchdog during debug sessions to prevent resets while stepping
    watchdog.pause_on_debug(true);

    // Start the watchdog (enables it with initial timeout)
    watchdog.start(timeout);
    defmt::info!(
        "Watchdog enabled with {}ms timeout",
        config::WATCHDOG_TIMEOUT_MS
    );

    let boot_time = Instant::now();

    loop {
        let now_ms = Instant::now().as_millis();
        let uptime_ms = Instant::now().duration_since(boot_time).as_millis();

        // During startup grace period, feed unconditionally to let tasks start
        if uptime_ms < STARTUP_GRACE_MS {
            watchdog.feed(timeout);
        } else {
            // Check all critical task heartbeats
            let spindle_ok = is_heartbeat_recent(
                &HEARTBEAT_SPINDLE_CONTROL,
                now_ms,
                SPINDLE_CONTROL_MAX_AGE_MS,
            );
            let current_ok = is_heartbeat_recent(
                &HEARTBEAT_CURRENT_MONITOR,
                now_ms,
                CURRENT_MONITOR_MAX_AGE_MS,
            );
            let pwm_ok = is_heartbeat_recent(&HEARTBEAT_PWM_INPUT, now_ms, PWM_INPUT_MAX_AGE_MS);
            let speed_ok =
                is_heartbeat_recent(&HEARTBEAT_SPEED_MEASURE, now_ms, SPEED_MEASURE_MAX_AGE_MS);

            if spindle_ok && current_ok && pwm_ok && speed_ok {
                watchdog.feed(timeout);
            } else {
                defmt::error!(
                    "Watchdog: heartbeat check failed! spindle={} current={} pwm={} speed={}",
                    spindle_ok as u8,
                    current_ok as u8,
                    pwm_ok as u8,
                    speed_ok as u8
                );
                // Do NOT feed the watchdog - system will reset
            }
        }

        Timer::after(Duration::from_millis(FEED_INTERVAL_MS)).await;
    }
}

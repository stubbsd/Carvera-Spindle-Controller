//! Current and temperature monitoring task.
//!
//! Reads the spindle current from ESCON analog output via ADC.
//! Monitors for overcurrent conditions and updates display data.
//! Also reads the RP2350 internal temperature sensor and triggers
//! thermal shutdown if the MCU temperature exceeds the limit.

use embassy_rp::adc::{Adc, Channel};
use embassy_time::{Duration, Instant, Timer};

use crate::adc_to_current_ma;
use crate::adc_to_temp_c;
use crate::state::{config, report_overcurrent, report_thermal, set_current_ma};
use crate::{ThresholdDetector, ThresholdStatus};

/// ADC read interval (100ms)
const ADC_INTERVAL_MS: u64 = 100;

/// Read temperature every N iterations (50 * 100ms = 5s)
const THERMAL_CHECK_INTERVAL: u32 = 50;

/// Current and temperature monitoring task
#[embassy_executor::task]
pub async fn current_monitor_task(
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut current_channel: Channel<'static>,
    mut temp_channel: Channel<'static>,
) {
    let mut overcurrent_detector = ThresholdDetector::new();
    let overcurrent_threshold =
        (config::CURRENT_AT_3V3_MA * config::OVERCURRENT_THRESHOLD_PCT) / 100;

    defmt::info!(
        "Current monitor: scale=0-{}mA threshold={}mA({}%) interval={}ms thermal_interval={}ms",
        config::CURRENT_AT_3V3_MA,
        overcurrent_threshold,
        config::OVERCURRENT_THRESHOLD_PCT,
        ADC_INTERVAL_MS,
        ADC_INTERVAL_MS * THERMAL_CHECK_INTERVAL as u64
    );

    let mut iteration: u32 = 0;

    loop {
        // Record heartbeat for watchdog health monitoring
        let now_heartbeat = embassy_time::Instant::now().as_millis();
        crate::state::heartbeat(&crate::state::HEARTBEAT_CURRENT_MONITOR, now_heartbeat);

        // Read current ADC value
        let adc_value = match adc.read(&mut current_channel).await {
            Ok(v) => v,
            Err(_) => {
                defmt::warn!("ADC read failed");
                Timer::after(Duration::from_millis(ADC_INTERVAL_MS)).await;
                continue;
            }
        };

        // Convert to current in mA (0V = 0mA, 3.3V = 5200mA per ESCON config)
        let current_ma = adc_to_current_ma(adc_value, config::CURRENT_AT_3V3_MA);

        // Store to atomic for other tasks (spindle_control reads this for display)
        set_current_ma(current_ma);

        // Check for overcurrent condition using threshold detector
        let now_ms = Instant::now().as_millis();
        let status = overcurrent_detector.check(
            current_ma,
            overcurrent_threshold,
            now_ms,
            config::OVERCURRENT_DEBOUNCE_MS,
        );

        if status == ThresholdStatus::Triggered {
            defmt::warn!("Overcurrent detected: {} mA", current_ma);
            report_overcurrent();
        }

        // Read temperature sensor periodically
        if iteration % THERMAL_CHECK_INTERVAL == 0 {
            if let Ok(temp_adc) = adc.read(&mut temp_channel).await {
                let temp_c = adc_to_temp_c(temp_adc);
                defmt::debug!("MCU temperature: {} C", temp_c);

                if temp_c >= config::THERMAL_SHUTDOWN_C {
                    defmt::error!(
                        "THERMAL SHUTDOWN: {} C >= {} C",
                        temp_c,
                        config::THERMAL_SHUTDOWN_C
                    );
                    report_thermal();
                }
            }
        }

        iteration = iteration.wrapping_add(1);
        Timer::after(Duration::from_millis(ADC_INTERVAL_MS)).await;
    }
}

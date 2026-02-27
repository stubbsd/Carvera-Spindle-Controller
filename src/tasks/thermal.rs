//! MCU temperature monitoring task.
//!
//! Monitors the RP2350's internal temperature sensor and triggers
//! a thermal shutdown if the MCU temperature exceeds the limit.
//! This protects the microcontroller from overheating.

use embassy_rp::adc::{Adc, Channel};
use embassy_time::{Duration, Timer};

use crate::adc_to_temp_c;
use crate::state::{config, report_thermal};

/// Temperature check interval (5 seconds)
const THERMAL_CHECK_INTERVAL_MS: u64 = 5000;

/// MCU temperature monitoring task
#[embassy_executor::task]
pub async fn thermal_monitor_task(
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut temp_channel: Channel<'static>,
) {
    defmt::info!("Thermal monitor task started");

    loop {
        // Read temperature sensor ADC value
        let adc_value = match adc.read(&mut temp_channel).await {
            Ok(v) => v,
            Err(_) => {
                defmt::warn!("Temperature ADC read failed");
                Timer::after(Duration::from_millis(THERMAL_CHECK_INTERVAL_MS)).await;
                continue;
            }
        };

        // Convert ADC reading to temperature using extracted function
        let temp_c = adc_to_temp_c(adc_value);

        defmt::debug!("MCU temperature: {} C", temp_c);

        // Check for thermal shutdown
        if temp_c >= config::THERMAL_SHUTDOWN_C {
            defmt::error!(
                "THERMAL SHUTDOWN: {} C >= {} C",
                temp_c,
                config::THERMAL_SHUTDOWN_C
            );
            report_thermal();
        }

        Timer::after(Duration::from_millis(THERMAL_CHECK_INTERVAL_MS)).await;
    }
}

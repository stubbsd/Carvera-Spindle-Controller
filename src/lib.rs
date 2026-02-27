//! Carvera Spindle Controller - Core Logic
//!
//! Pure functions and data structures that can be tested on the host.

#![cfg_attr(not(test), no_std)]

pub mod adc;
pub mod calibration;
pub mod conversion;
pub mod display;
pub mod filters;
pub mod flash_store;
pub mod lcd;
pub mod speed;
pub mod stabilization;
pub mod stall;
pub mod state;
pub mod temperature;
pub mod threshold;

#[cfg(feature = "embedded")]
pub mod tasks;

// Re-export commonly used types and functions
pub use adc::adc_to_current_ma;
pub use conversion::{
    duty_to_rpm, frequency_to_rpm, motor_rpm_to_output_duty, spindle_to_motor_rpm,
};
pub use filters::CircularBuffer;
pub use speed::{is_valid_period, median_u32, period_us_to_frequency_mhz, periods_to_rpm};
pub use stabilization::{StabilizationStatus, StabilizationTracker};
pub use stall::{StallConfig, StallDetector, StallStatus};
pub use temperature::{
    DEFAULT_ADC_VREF_MV, TEMP_SENSOR_SLOPE_UV_C, TEMP_SENSOR_V27_MV, adc_to_temp_c,
    adc_to_voltage_mv, voltage_to_temp_c,
};
pub use threshold::{ThresholdDetector, ThresholdStatus};

// ============================================================================
// Integration Tests
// ============================================================================
//
// Tests that verify behavior across multiple modules or system-level requirements.
// Domain-specific unit tests live in their respective modules.

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // --- PIO Debounce Timing tests ---
    //
    // These tests verify that PIO debounce timing is compatible with
    // period measurement at various spindle speeds.

    /// Calculate debounce time in microseconds for given clock divider.
    /// Formula: 62 cycles * divider / 125 MHz = debounce_us
    fn debounce_time_us(divider: u32) -> u32 {
        (62 * divider) / 125
    }

    #[test]
    fn test_debounce_time_calculation() {
        // With divider=64: 62 * 64 / 125 = 31.7 us
        let debounce_us = debounce_time_us(64);
        assert_eq!(debounce_us, 31); // ~32 us

        // Verify it's much less than pulse width at max RPM
        // At 13000 RPM, 4 PPR: period = 60_000_000 / (13000 * 4) = 1154 us
        // Pulse HIGH time ~ 577 us (50% duty)
        let max_rpm_period_us = 60_000_000 / (13000 * 4);
        let pulse_high_us = max_rpm_period_us / 2;
        assert!(
            debounce_us < pulse_high_us / 10,
            "Debounce {}us should be <10% of pulse width {}us",
            debounce_us,
            pulse_high_us
        );
    }

    #[test]
    fn test_clean_periods_from_pio() {
        // Simulating what PIO would output: clean, debounced periods
        // At 6100 RPM, 4 PPR: period = 60_000_000 / (6100 * 4) = 2459 us
        let clean_periods = [2459, 2460, 2458, 2461, 2459, 2457, 2460, 2459];
        let rpm = periods_to_rpm(&clean_periods, 4);
        assert!(
            (6090..6110).contains(&rpm),
            "Expected ~6100 RPM, got {}",
            rpm
        );
    }

    #[test]
    fn test_realistic_period_variation() {
        // Real motors have slight speed variations (~1-2%)
        // This should NOT be filtered out - it's real variation
        let periods = [2450, 2470, 2440, 2480, 2455, 2465, 2445, 2475];
        let rpm = periods_to_rpm(&periods, 4);
        // Median of sorted [2440,2445,2450,2455,2465,2470,2475,2480] = (2455+2465)/2 = 2460
        // RPM = 60_000_000 / (2460 * 4) = 6097
        assert!(
            (6050..6150).contains(&rpm),
            "Expected ~6100 RPM, got {}",
            rpm
        );
    }

    #[test]
    fn test_single_outlier_still_filtered() {
        // Even with PIO, one bad period might slip through
        // Median should still handle it
        let periods = [2459, 2459, 2459, 2459, 2459, 2459, 2459, 1500]; // 1 outlier
        let rpm = periods_to_rpm(&periods, 4);
        assert!(
            (6090..6110).contains(&rpm),
            "Single outlier should be filtered by median, got {} RPM",
            rpm
        );
    }

    #[test]
    fn test_rpm_at_various_speeds() {
        // Low speed: 2000 RPM, 4 PPR -> period = 7500 us
        let rpm = periods_to_rpm(&[7500, 7500, 7500, 7500], 4);
        assert_eq!(rpm, 2000);

        // Mid speed: 6000 RPM, 4 PPR -> period = 2500 us
        let rpm = periods_to_rpm(&[2500, 2500, 2500, 2500], 4);
        assert_eq!(rpm, 6000);

        // High speed: 12000 RPM, 4 PPR -> period = 1250 us
        let rpm = periods_to_rpm(&[1250, 1250, 1250, 1250], 4);
        assert_eq!(rpm, 12000);
    }

    #[test]
    fn test_minimum_valid_period() {
        // At 13000 RPM (our max): period = 60_000_000 / (13000 * 4) = 1154 us
        // Debounce time ~32 us is well under half-period (~577 us)
        let periods = [1154, 1154, 1154, 1154, 1154, 1154, 1154, 1154];
        let rpm = periods_to_rpm(&periods, 4);
        assert!(
            (12990..13010).contains(&rpm),
            "Expected ~13000 RPM, got {}",
            rpm
        );
    }

    #[test]
    fn test_clock_divider_recommendations() {
        // At max RPM (13000), pulse high time ~577us
        // Debounce should be <10% of that = <58us
        assert!(
            debounce_time_us(64) < 58,
            "Divider 64 should give <58us debounce"
        );
        assert!(
            debounce_time_us(64) > 20,
            "Divider 64 should give >20us debounce"
        );

        // Divider 8 = ~4us (very aggressive, might miss some noise)
        assert!(debounce_time_us(8) < 10);

        // Divider 256 = ~127us (conservative, might miss fast pulses at high RPM)
        assert!(debounce_time_us(256) > 100);
    }

    // --- Cross-module integration: duty -> calibrated RPM -> display formatting ---

    #[test]
    #[serial]
    fn test_pipeline_raw_duty_to_calibrated_rpm_no_cal() {
        // Without calibration data, duty_to_calibrated_rpm falls back to the
        // standard linear formula: rpm = raw_duty * CARVERA_SPINDLE_MAX_RPM / 10000
        use crate::calibration::{clear_calibration, duty_to_calibrated_rpm};
        use crate::state::CAL_SEQUENCE_ACTIVE;
        use core::sync::atomic::Ordering;

        clear_calibration();
        CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);

        // S10000 -> Carvera sends ~48.9% duty = 4893 in 0-10000 scale
        let raw_duty: u16 = 4893;
        let rpm = duty_to_calibrated_rpm(raw_duty);
        // Expected: 4893 * 20437 / 10000 = ~10001 (with rounding)
        assert!(
            (9950..10050).contains(&rpm),
            "Expected ~10000 RPM from S10000 duty, got {}",
            rpm
        );
    }

    #[test]
    #[serial]
    fn test_pipeline_with_calibration_correction() {
        use crate::calibration::{
            CalibrationPoint, CalibrationTable, apply_calibration, clear_calibration, correct_duty,
            duty_to_calibrated_rpm,
        };
        use crate::state::CAL_SEQUENCE_ACTIVE;
        use core::sync::atomic::Ordering;

        clear_calibration();
        CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);

        // Build a simple 2-point calibration table with a known offset.
        // Point 0: at 750 RPM, measured duty is 377 (slightly higher than ideal 367)
        // Point 1: at 800 RPM, measured duty is 402 (slightly higher than ideal 391)
        let mut table = CalibrationTable::default();
        table.count = 2;
        table.points[0] = CalibrationPoint {
            expected_rpm: 750,
            measured_duty: 377,
        };
        table.points[1] = CalibrationPoint {
            expected_rpm: 800,
            measured_duty: 402,
        };
        apply_calibration(&table);

        // Feed the measured duty for the first cal point
        let rpm = duty_to_calibrated_rpm(377);
        assert!(
            (740..760).contains(&rpm),
            "Expected ~750 RPM from calibrated lookup, got {}",
            rpm
        );

        // correct_duty should remap the raw duty to the ideal duty for that RPM
        let corrected = correct_duty(377);
        // The corrected duty should produce ~750 RPM via the standard formula
        // 750 * 10000 / 20437 = ~367
        assert!(
            (360..380).contains(&corrected),
            "Expected corrected duty ~367, got {}",
            corrected
        );

        // Cleanup
        clear_calibration();
    }

    #[test]
    fn test_debounce_vs_max_rpm_safety_margin() {
        // Ensure debounce time is safe at maximum RPM
        // At 13000 RPM with 4 PPR: period = 1154 us, pulse HIGH = ~577 us
        // Rule: debounce should be <10% of pulse HIGH time
        let divider: u32 = 64;
        let debounce_us = debounce_time_us(divider);
        let max_rpm_pulse_high_us = 60_000_000 / (13000 * 4) / 2;
        let max_allowed_debounce = max_rpm_pulse_high_us / 10;

        assert!(
            debounce_us <= max_allowed_debounce,
            "Debounce {}us exceeds 10% of pulse HIGH {}us at 13000 RPM (max={}us)",
            debounce_us,
            max_rpm_pulse_high_us,
            max_allowed_debounce
        );
    }
}

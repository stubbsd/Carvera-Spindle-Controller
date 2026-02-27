//! Temperature sensor functions for RP2350 internal temperature sensor.
//!
//! Provides conversion from ADC readings to temperature using the RP2350's
//! internal temperature sensor characteristics from the datasheet.

/// ADC reference voltage in millivolts (default for RP2350).
pub const DEFAULT_ADC_VREF_MV: u32 = 3300;

/// Temperature sensor voltage at 27C in millivolts (from RP2350 datasheet).
pub const TEMP_SENSOR_V27_MV: u32 = 706;

/// Temperature sensor slope in uV/C (from RP2350 datasheet: 1.721 mV/C = 1721 uV/C).
pub const TEMP_SENSOR_SLOPE_UV_C: u32 = 1721;

/// Convert 12-bit ADC reading to voltage in millivolts.
///
/// # Arguments
/// * `adc_value` - 12-bit ADC reading (0-4095)
/// * `vref_mv` - ADC reference voltage in millivolts (typically 3300)
///
/// # Returns
/// Voltage in millivolts
///
/// # Examples
/// ```
/// use carvera_spindle::adc_to_voltage_mv;
///
/// // 0 ADC = 0 mV
/// assert_eq!(adc_to_voltage_mv(0, 3300), 0);
///
/// // Full scale ADC = reference voltage
/// assert_eq!(adc_to_voltage_mv(4095, 3300), 3300);
///
/// // Half scale
/// assert_eq!(adc_to_voltage_mv(2048, 3300), 1650);
/// ```
pub fn adc_to_voltage_mv(adc_value: u16, vref_mv: u32) -> u32 {
    let clamped = (adc_value as u32).min(4095);
    (clamped * vref_mv) / 4095
}

/// Convert voltage to temperature using RP2350 internal sensor.
///
/// Uses datasheet values:
/// - V27 = 706 mV (voltage at 27C)
/// - Slope = 1.721 mV/C (negative: higher voltage = lower temperature)
///
/// Formula: T = 27 - (V - 706) / 1.721
///
/// # Arguments
/// * `voltage_mv` - Voltage from temperature sensor in millivolts
///
/// # Returns
/// Temperature in degrees Celsius
///
/// # Examples
/// ```
/// use carvera_spindle::voltage_to_temp_c;
///
/// // At 706 mV = 27C
/// assert_eq!(voltage_to_temp_c(706), 27);
///
/// // Higher voltage = lower temperature
/// // 750 mV: 27 - (750 - 706) / 1.721 = 27 - 25.6 ~ 1C
/// let temp = voltage_to_temp_c(750);
/// assert!(temp < 27 && temp > 0);
///
/// // Lower voltage = higher temperature
/// // 650 mV: 27 + (706 - 650) / 1.721 = 27 + 32.5 ~ 59C
/// let temp = voltage_to_temp_c(650);
/// assert!(temp > 27 && temp < 70);
/// ```
pub fn voltage_to_temp_c(voltage_mv: u32) -> i32 {
    // T = 27 - (V - V27) / slope
    // Using integer math: T = 27 - ((voltage_mv - 706) * 1000) / 1721
    // Note: slope is in uV/C, so we multiply difference by 1000 to match
    if voltage_mv > TEMP_SENSOR_V27_MV {
        27i32 - ((voltage_mv - TEMP_SENSOR_V27_MV) as i32 * 1000 / TEMP_SENSOR_SLOPE_UV_C as i32)
    } else {
        27i32 + ((TEMP_SENSOR_V27_MV - voltage_mv) as i32 * 1000 / TEMP_SENSOR_SLOPE_UV_C as i32)
    }
}

/// Convert 12-bit ADC reading directly to temperature in degrees Celsius.
///
/// Convenience function that combines ADC to voltage and voltage to temperature
/// conversion using the default 3.3V reference voltage.
///
/// # Arguments
/// * `adc_value` - 12-bit ADC reading from temperature sensor (0-4095)
///
/// # Returns
/// Temperature in degrees Celsius
///
/// # Examples
/// ```
/// use carvera_spindle::adc_to_temp_c;
///
/// // ADC value that produces 706 mV at 3.3V reference
/// // 706 mV / 3300 mV * 4095 = 876
/// let temp = adc_to_temp_c(876);
/// assert!((25..=29).contains(&temp), "Expected ~27C, got {}", temp);
/// ```
pub fn adc_to_temp_c(adc_value: u16) -> i32 {
    let voltage_mv = adc_to_voltage_mv(adc_value, DEFAULT_ADC_VREF_MV);
    voltage_to_temp_c(voltage_mv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adc_to_voltage_zero() {
        assert_eq!(adc_to_voltage_mv(0, 3300), 0);
    }

    #[test]
    fn test_adc_to_voltage_full_scale() {
        assert_eq!(adc_to_voltage_mv(4095, 3300), 3300);
    }

    #[test]
    fn test_adc_to_voltage_half_scale() {
        // 2048 / 4095 * 3300 ~ 1650 (integer division may be off by 1)
        let voltage = adc_to_voltage_mv(2048, 3300);
        assert!(
            (1649..=1651).contains(&voltage),
            "Expected ~1650, got {}",
            voltage
        );
    }

    #[test]
    fn test_adc_to_voltage_overflow_protection() {
        // Values > 4095 should clamp
        assert_eq!(adc_to_voltage_mv(5000, 3300), 3300);
        assert_eq!(adc_to_voltage_mv(u16::MAX, 3300), 3300);
    }

    #[test]
    fn test_voltage_to_temp_at_27c() {
        // 706 mV should give exactly 27C
        assert_eq!(voltage_to_temp_c(706), 27);
    }

    #[test]
    fn test_voltage_to_temp_above_27c() {
        // Lower voltage = higher temperature
        // 650 mV: 27 + (706 - 650) * 1000 / 1721 = 27 + 32.5 ~ 59C
        let temp = voltage_to_temp_c(650);
        assert!(temp > 50 && temp < 65, "Expected ~59C, got {}", temp);
    }

    #[test]
    fn test_voltage_to_temp_below_27c() {
        // Higher voltage = lower temperature
        // 750 mV: 27 - (750 - 706) * 1000 / 1721 = 27 - 25.6 ~ 1C
        let temp = voltage_to_temp_c(750);
        assert!(temp >= 0 && temp < 10, "Expected ~1C, got {}", temp);
    }

    #[test]
    fn test_adc_to_temp_integration() {
        // ADC value that produces ~706 mV at 3.3V reference
        // 706 / 3300 * 4095 ~ 876
        let temp = adc_to_temp_c(876);
        // Should be close to 27C (within a few degrees due to integer math)
        assert!((25..=29).contains(&temp), "Expected ~27C, got {}", temp);
    }

    #[test]
    fn test_adc_to_temp_hot() {
        // Lower ADC = lower voltage = higher temp
        // At 60C: V = 706 - (60-27) * 1.721 = 706 - 56.8 ~ 649 mV
        // ADC = 649 / 3300 * 4095 ~ 805
        let temp = adc_to_temp_c(805);
        assert!(temp > 50, "Expected >50C, got {}", temp);
    }

    #[test]
    fn test_adc_to_temp_extreme_low_adc() {
        // ADC = 0: voltage = 0 mV, very low voltage = very high temperature
        // T = 27 + (706 - 0) * 1000 / 1721 = 27 + 410 = 437C
        let temp = adc_to_temp_c(0);
        assert_eq!(
            temp, 437,
            "ADC 0 should give ~437C (unrealistic but tests math path)"
        );
    }

    #[test]
    fn test_adc_to_temp_extreme_high_adc() {
        // ADC = 4095: voltage = 3300 mV, very high voltage = very low temperature
        // T = 27 - (3300 - 706) * 1000 / 1721 = 27 - 1507 = -1480C
        let temp = adc_to_temp_c(4095);
        assert_eq!(
            temp, -1480,
            "ADC 4095 should give ~-1480C (unrealistic but tests math path)"
        );
    }

    #[test]
    fn test_adc_to_temp_cold() {
        // Higher ADC = higher voltage = lower temp
        // At 0C: V = 706 + 27 * 1.721 = 706 + 46.5 ~ 752 mV
        // ADC = 752 / 3300 * 4095 ~ 933
        let temp = adc_to_temp_c(933);
        assert!(temp < 10, "Expected <10C, got {}", temp);
    }
}

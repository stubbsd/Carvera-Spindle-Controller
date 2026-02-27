//! ADC conversion functions for current measurement.
//!
//! Provides conversion from ADC readings to current values for the ESCON
//! motor controller's analog current output.

/// Convert ADC reading to current in milliamps.
///
/// **Hardware Note**: ESCON analog output is 0-4V (for "Actual Current Averaged").
/// This exceeds the RP2350 ADC's 0-3.3V range. A voltage divider is required:
/// - Use 10K + 3.3K resistor divider (0-4V → 0-2.5V)
/// - Or adjust MAX_CURRENT_MA scaling if using different divider ratio
///
/// The RP2350 ADC is 12-bit (0-4095) with 3.3V reference.
///
/// # Arguments
/// * `adc_value` - 12-bit ADC reading (0-4095)
/// * `max_current_ma` - Maximum current in mA corresponding to full scale
///
/// # Returns
/// Current in milliamps
///
/// # Examples
/// ```
/// use carvera_spindle::adc_to_current_ma;
///
/// // ADC 0 = 0 mA
/// assert_eq!(adc_to_current_ma(0, 5000), 0);
///
/// // ADC 4095 = max current
/// assert_eq!(adc_to_current_ma(4095, 5000), 5000);
///
/// // ADC 2048 = ~half current
/// assert_eq!(adc_to_current_ma(2048, 5000), 2500);
/// ```
pub fn adc_to_current_ma(adc_value: u16, max_current_ma: u32) -> u32 {
    // Clamp to 12-bit range
    let clamped = (adc_value as u32).min(4095);
    (clamped * max_current_ma) / 4095
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adc_to_current_zero() {
        assert_eq!(adc_to_current_ma(0, 5000), 0);
    }

    #[test]
    fn test_adc_to_current_max() {
        assert_eq!(adc_to_current_ma(4095, 5000), 5000);
    }

    #[test]
    fn test_adc_to_current_midpoint() {
        // 2048 * 5000 / 4095 = 2500 (integer division)
        assert_eq!(adc_to_current_ma(2048, 5000), 2500);
    }

    #[test]
    fn test_adc_overflow_protection() {
        // Values > 4095 should clamp
        assert_eq!(adc_to_current_ma(5000, 5000), 5000);
        assert_eq!(adc_to_current_ma(u16::MAX, 5000), 5000);
    }
}

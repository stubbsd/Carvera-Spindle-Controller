//! Period-based speed measurement functions (T-Method).
//!
//! This module provides functions for measuring spindle speed using period
//! measurement (time between pulses) rather than frequency counting.
//! The T-Method provides much higher resolution at lower speeds.

/// Calculate median of a slice of u32 values.
///
/// Returns 0 for empty slices. Maximum supported slice length is 16 elements.
/// Uses insertion sort which is efficient for small arrays.
///
/// Median filtering is industry-standard for removing impulse noise (spikes)
/// from measurements without introducing extreme values like mean averaging does.
///
/// # Arguments
/// * `values` - Slice of values to find median of (max 16 elements)
///
/// # Returns
/// Median value, or 0 if slice is empty
///
/// # Examples
/// ```
/// use carvera_spindle::median_u32;
///
/// // Odd number of values - returns middle value
/// assert_eq!(median_u32(&[1, 3, 5, 7, 9]), 5);
///
/// // Even number of values - returns average of two middle values
/// assert_eq!(median_u32(&[1, 3, 5, 7]), 4);
///
/// // Outliers don't affect result
/// assert_eq!(median_u32(&[100, 100, 100, 1]), 100);
///
/// // Empty slice returns 0
/// assert_eq!(median_u32(&[]), 0);
/// ```
pub fn median_u32(values: &[u32]) -> u32 {
    let len = values.len();
    if len == 0 {
        return 0;
    }

    // Copy to local array for sorting (max 16 elements)
    let mut sorted = [0u32; 16];
    let n = len.min(16);
    sorted[..n].copy_from_slice(&values[..n]);

    // Simple insertion sort (efficient for small N)
    for i in 1..n {
        let mut j = i;
        while j > 0 && sorted[j - 1] > sorted[j] {
            sorted.swap(j - 1, j);
            j -= 1;
        }
    }

    let mid = n / 2;
    if n % 2 == 0 {
        // Even: average of two middle values
        (sorted[mid - 1] + sorted[mid]) / 2
    } else {
        // Odd: middle value
        sorted[mid]
    }
}

/// Calculate frequency from period in microseconds.
///
/// Returns frequency in millihertz (mHz) for 0.001 Hz resolution.
///
/// # Arguments
/// * `period_us` - Period between pulses in microseconds
///
/// # Returns
/// Frequency in millihertz (mHz), or 0 if period is 0
///
/// # Examples
/// ```
/// use carvera_spindle::period_us_to_frequency_mhz;
///
/// // 2457 us period = ~407 Hz = 407000 mHz
/// let freq = period_us_to_frequency_mhz(2457);
/// assert!((406000..408000).contains(&freq));
///
/// // 2500 us period = 400 Hz = 400000 mHz
/// assert_eq!(period_us_to_frequency_mhz(2500), 400000);
/// ```
pub fn period_us_to_frequency_mhz(period_us: u32) -> u32 {
    if period_us == 0 {
        return 0;
    }
    // freq_hz = 1_000_000 / period_us
    // freq_mhz = 1_000_000_000 / period_us
    1_000_000_000 / period_us
}

/// Calculate RPM from period measurements using median filtering.
///
/// Uses the T-Method: measures time between pulses for high resolution.
/// Uses median (not mean) to filter impulse noise from the measurements.
///
/// Median filtering is robust against outliers - up to 50% of samples can
/// be corrupted (e.g., noise spikes causing early edge detection) and the
/// result will still be correct.
///
/// # Arguments
/// * `periods` - Slice of period measurements in microseconds
/// * `pulses_per_rev` - Number of pulses per revolution (typically 4)
///
/// # Returns
/// RPM value, or 0 if periods is empty or invalid
///
/// # Examples
/// ```
/// use carvera_spindle::periods_to_rpm;
///
/// // 6000 RPM with 4 PPR = 400 Hz = 2500 us period
/// let periods = [2500, 2500, 2500, 2500];
/// assert_eq!(periods_to_rpm(&periods, 4), 6000);
///
/// // Median filtering ignores outliers (spike at 2008 us)
/// let periods = [2459, 2459, 2459, 2459, 2459, 2459, 2459, 2008];
/// let rpm = periods_to_rpm(&periods, 4);
/// // Median = 2459, so RPM = 60_000_000 / (2459 * 4) = 6100
/// assert!((6090..6110).contains(&rpm), "Expected ~6100, got {}", rpm);
/// ```
pub fn periods_to_rpm(periods: &[u32], pulses_per_rev: u32) -> u32 {
    if periods.is_empty() || pulses_per_rev == 0 {
        return 0;
    }
    // Use median instead of mean to reject impulse noise
    let median_period_us = median_u32(periods);
    if median_period_us == 0 {
        return 0;
    }
    // rpm = 60_000_000 / (median_period_us * pulses_per_rev), rounded to nearest
    let divisor = median_period_us * pulses_per_rev;
    (60_000_000 + divisor / 2) / divisor
}

/// Validate that a period measurement is within expected range.
///
/// Filters outliers and noise by checking if the period corresponds
/// to an RPM within the valid operating range.
///
/// # Arguments
/// * `period_us` - Period between pulses in microseconds
/// * `min_rpm` - Minimum valid RPM
/// * `max_rpm` - Maximum valid RPM
/// * `pulses_per_rev` - Number of pulses per revolution
///
/// # Returns
/// `true` if period is within valid range, `false` otherwise
///
/// # Examples
/// ```
/// use carvera_spindle::is_valid_period;
///
/// // 2500 us at 4 PPR = 6000 RPM - within 1000-12500 range
/// assert!(is_valid_period(2500, 1000, 12500, 4));
///
/// // 1000 us at 4 PPR = 15000 RPM - above max
/// assert!(!is_valid_period(1000, 1000, 12500, 4));
///
/// // 20000 us at 4 PPR = 750 RPM - below min
/// assert!(!is_valid_period(20000, 1000, 12500, 4));
/// ```
pub fn is_valid_period(period_us: u32, min_rpm: u32, max_rpm: u32, pulses_per_rev: u32) -> bool {
    if period_us == 0 || pulses_per_rev == 0 || min_rpm == 0 || max_rpm == 0 {
        return false;
    }
    // At max_rpm: period = 60_000_000 / (max_rpm * pulses_per_rev) (shortest period)
    // At min_rpm: period = 60_000_000 / (min_rpm * pulses_per_rev) (longest period)
    let min_period = 60_000_000 / (max_rpm * pulses_per_rev);
    let max_period = 60_000_000 / (min_rpm * pulses_per_rev);
    period_us >= min_period && period_us <= max_period
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Median Filter tests ---

    #[test]
    fn test_median_empty_slice() {
        assert_eq!(median_u32(&[]), 0);
    }

    #[test]
    fn test_median_single_value() {
        assert_eq!(median_u32(&[42]), 42);
    }

    #[test]
    fn test_median_two_values() {
        // Even count: average of middle two
        assert_eq!(median_u32(&[10, 20]), 15);
    }

    #[test]
    fn test_median_odd_count() {
        // Odd count: middle value
        assert_eq!(median_u32(&[1, 3, 5, 7, 9]), 5);
        assert_eq!(median_u32(&[5, 1, 9, 3, 7]), 5); // Unsorted input
    }

    #[test]
    fn test_median_even_count() {
        // Even count: average of two middle values
        assert_eq!(median_u32(&[1, 3, 5, 7]), 4); // (3 + 5) / 2 = 4
        assert_eq!(median_u32(&[7, 1, 5, 3]), 4); // Same, unsorted
    }

    #[test]
    fn test_median_outlier_rejection() {
        // One outlier in 8 values - median should ignore it
        let values = [100, 100, 100, 100, 100, 100, 100, 1];
        // Sorted: [1, 100, 100, 100, 100, 100, 100, 100]
        // Median = (values[3] + values[4]) / 2 = (100 + 100) / 2 = 100
        assert_eq!(median_u32(&values), 100);
    }

    #[test]
    fn test_median_multiple_outliers() {
        // 3 outliers in 8 values (< 50%) - median still works
        let values = [100, 100, 100, 100, 100, 1, 1, 1];
        // Sorted: [1, 1, 1, 100, 100, 100, 100, 100]
        // Median = (values[3] + values[4]) / 2 = (100 + 100) / 2 = 100
        assert_eq!(median_u32(&values), 100);
    }

    #[test]
    fn test_median_more_than_16_elements() {
        // median_u32 only considers the first 16 elements (let n = len.min(16))
        // Verify that extra elements beyond 16 are ignored
        let mut values = [100u32; 20];
        // First 16 are 100, last 4 are 999 (should be ignored)
        values[16] = 999;
        values[17] = 999;
        values[18] = 999;
        values[19] = 999;
        // Median of first 16 values (all 100) should be 100
        assert_eq!(median_u32(&values), 100);

        // Different test: put low values in first 16, high in last 4
        let mut values2 = [0u32; 20];
        for i in 0..16 {
            values2[i] = 50;
        }
        for i in 16..20 {
            values2[i] = 9999;
        }
        // Should only see the first 16 values (all 50)
        assert_eq!(median_u32(&values2), 50);
    }

    #[test]
    fn test_median_identical_values() {
        assert_eq!(median_u32(&[50, 50, 50, 50]), 50);
    }

    #[test]
    fn test_median_realistic_periods() {
        // Real-world scenario: periods around 2459 us with one spike
        let periods = [2459, 2460, 2458, 2461, 2459, 2457, 2460, 2008];
        // Sorted: [2008, 2457, 2458, 2459, 2459, 2460, 2460, 2461]
        // Median = (2459 + 2459) / 2 = 2459
        assert_eq!(median_u32(&periods), 2459);
    }

    // --- Period Measurement (T-Method) tests ---

    #[test]
    fn test_period_to_frequency_407hz() {
        // 407 Hz = period of ~2457 us
        let freq_mhz = period_us_to_frequency_mhz(2457);
        // Should be ~407000 mHz (407 Hz)
        assert!(
            (406000..408000).contains(&freq_mhz),
            "Expected ~407000 mHz, got {}",
            freq_mhz
        );
    }

    #[test]
    fn test_period_to_frequency_400hz() {
        // 400 Hz = period of 2500 us
        let freq_mhz = period_us_to_frequency_mhz(2500);
        assert_eq!(freq_mhz, 400000); // Exactly 400 Hz = 400000 mHz
    }

    #[test]
    fn test_period_to_frequency_zero() {
        assert_eq!(period_us_to_frequency_mhz(0), 0);
    }

    #[test]
    fn test_periods_to_rpm_6000rpm() {
        // 6000 RPM with 4 PPR = 400 Hz = 2500 us period
        let periods = [2500, 2500, 2500, 2500];
        let rpm = periods_to_rpm(&periods, 4);
        assert_eq!(rpm, 6000);
    }

    #[test]
    fn test_periods_to_rpm_median_filtering() {
        // Slightly varying periods around 2500 us
        // Sorted: [2450, 2500, 2500, 2550], median = (2500+2500)/2 = 2500
        let periods = [2450, 2500, 2550, 2500];
        let rpm = periods_to_rpm(&periods, 4);
        // Median period = 2500, so RPM = 60_000_000 / (2500 * 4) = 6000
        assert_eq!(rpm, 6000);
    }

    #[test]
    fn test_periods_to_rpm_rejects_spike() {
        // Simulates real-world noise: 7 good periods (~6100 RPM) + 1 spike (would be ~7488 RPM)
        // Good period at 6100 RPM, 4 PPR: 60_000_000 / (6100 * 4) = 2459 us
        // Spike period (noise): ~2008 us (would give 7488 RPM if used)
        let periods = [2459, 2459, 2459, 2459, 2459, 2459, 2459, 2008];
        let rpm = periods_to_rpm(&periods, 4);
        // With mean: avg = (2459*7 + 2008) / 8 = 2403 us -> 6234 RPM (wrong)
        // With median: sorted middle values are 2459, median = 2459 -> 6100 RPM (correct)
        assert!(
            (6090..6110).contains(&rpm),
            "Expected ~6100 RPM (median filtering), got {} RPM",
            rpm
        );
    }

    #[test]
    fn test_periods_to_rpm_rejects_multiple_spikes() {
        // 5 good periods + 3 bad periods (still < 50%, median should work)
        let periods = [2459, 2459, 2459, 2459, 2459, 2008, 2008, 2008];
        let rpm = periods_to_rpm(&periods, 4);
        // Sorted: [2008, 2008, 2008, 2459, 2459, 2459, 2459, 2459]
        // Median of 8 values = (values[3] + values[4]) / 2 = (2459 + 2459) / 2 = 2459
        assert!(
            (6090..6110).contains(&rpm),
            "Expected ~6100 RPM with 3 spikes, got {} RPM",
            rpm
        );
    }

    #[test]
    fn test_periods_to_rpm_single_sample() {
        let periods = [2500];
        let rpm = periods_to_rpm(&periods, 4);
        assert_eq!(rpm, 6000);
    }

    #[test]
    fn test_periods_to_rpm_empty() {
        let periods: [u32; 0] = [];
        assert_eq!(periods_to_rpm(&periods, 4), 0);
    }

    #[test]
    fn test_periods_to_rpm_zero_ppr() {
        let periods = [2500, 2500];
        assert_eq!(periods_to_rpm(&periods, 0), 0);
    }

    #[test]
    fn test_is_valid_period_in_range() {
        // 2500 us at 4 PPR = 6000 RPM, within 1000-12500 range
        assert!(is_valid_period(2500, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_at_min_rpm() {
        // At 1000 RPM, 4 PPR: period = 60_000_000 / (1000 * 4) = 15000 us
        assert!(is_valid_period(15000, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_at_max_rpm() {
        // At 12500 RPM, 4 PPR: period = 60_000_000 / (12500 * 4) = 1200 us
        assert!(is_valid_period(1200, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_too_fast() {
        // 1000 us at 4 PPR = 15000 RPM - above max 12500
        assert!(!is_valid_period(1000, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_too_slow() {
        // 20000 us at 4 PPR = 750 RPM - below min 1000
        assert!(!is_valid_period(20000, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_zero_period() {
        assert!(!is_valid_period(0, 1000, 12500, 4));
    }

    #[test]
    fn test_is_valid_period_zero_ppr() {
        assert!(!is_valid_period(2500, 1000, 12500, 0));
    }

    #[test]
    fn test_period_measurement_resolution() {
        // Test that T-method gives better resolution than M-method
        // At 6000 RPM: period = 2500 us
        // Changing by 1 us: 2499 us vs 2500 us
        let rpm_2500 = periods_to_rpm(&[2500], 4);
        let rpm_2499 = periods_to_rpm(&[2499], 4);
        // Should differ by ~2-3 RPM (much better than 375 RPM M-method resolution)
        let diff = rpm_2499.saturating_sub(rpm_2500);
        assert!(diff > 0 && diff < 10, "Expected small diff, got {}", diff);
    }

    // --- Revolution-based measurement tests ---

    #[test]
    fn test_revolution_period_eliminates_asymmetry() {
        // ESCON 4PPR output has pulse-to-pulse asymmetry within each revolution.
        // Per-pulse measurement: 3 long + 1 short pulse, median picks long = low RPM.
        // Revolution measurement: sum of 4 pulses = correct total period.
        //
        // At 6108 motor RPM: total revolution period = 9823 us
        // Asymmetric pulses: 2460, 2460, 2460, 2443 (sum = 9823)
        let pulse_periods = [2460u32, 2460, 2460, 2443];

        // Per-pulse approach: median of individual pulses
        let per_pulse_rpm = periods_to_rpm(&pulse_periods, 4);
        // Median = 2460, RPM = 60_000_000 / (2460 * 4) = 6098 (12 RPM low)

        // Revolution approach: use total period with pulses_per_rev=1
        let rev_period: u32 = pulse_periods.iter().sum();
        let rev_rpm = periods_to_rpm(&[rev_period], 1);
        // rev_period = 9823, RPM = 60_000_000 / 9823 = 6108 (correct!)

        assert_eq!(
            rev_rpm, 6108,
            "Revolution measurement should give exact RPM"
        );
        assert!(
            per_pulse_rpm < rev_rpm,
            "Per-pulse median should read lower due to asymmetry bias"
        );
    }

    #[test]
    fn test_revolution_period_accuracy_at_10000_spindle() {
        // At S10000: motor RPM ~6108, spindle = 6108 * 1635/1000 = 9987
        // Revolution period = 60_000_000 / 6108 = 9823 us
        let rev_period = 9823u32;
        let motor_rpm = periods_to_rpm(&[rev_period], 1);
        // With rounding: (60_000_000 + 4911) / 9823 = 6108
        assert_eq!(motor_rpm, 6108);
        // Belt ratio: 6108 * 1635 / 1000 = 9986 (truncated), 9987 (rounded)
        let spindle_rpm = (motor_rpm * 1635 + 500) / 1000;
        assert!(
            (9985..=9990).contains(&spindle_rpm),
            "Expected ~9987 spindle RPM, got {}",
            spindle_rpm
        );
    }

    #[test]
    fn test_rounding_vs_truncation() {
        // Verify rounding gives closer result than truncation
        // At period 2459 us, 4 PPR: exact = 60_000_000 / 9836 = 6100.04
        assert_eq!(periods_to_rpm(&[2459], 4), 6100);

        // At period 2461 us, 4 PPR: exact = 60_000_000 / 9844 = 6095.08
        assert_eq!(periods_to_rpm(&[2461], 4), 6095);

        // Revolution period at 6108 RPM: 60_000_000 / 6108 = 9823 us
        // 6108 * 9823 = 59,998,884; remainder = 1116; 1116/9823 = 0.11
        // Truncated and rounded both give 6108
        assert_eq!(periods_to_rpm(&[9823], 1), 6108);
    }
}

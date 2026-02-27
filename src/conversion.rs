//! PWM and RPM conversion functions.
//!
//! This module provides functions for converting between PWM duty cycles,
//! RPM values, and encoder frequencies for spindle control.

/// Calculate RPM from output duty cycle.
///
/// Maps the ESCON's 10-90% duty cycle range to MIN_RPM-MAX_RPM.
/// The duty cycle is in 0.01% units (1000 = 10%, 9000 = 90%).
///
/// # Arguments
/// * `output_duty` - Output duty cycle in 0.01% units (1000-9000 range)
/// * `min_rpm` - RPM at 10% duty cycle (ESCON setting)
/// * `max_rpm` - RPM at 90% duty cycle (ESCON setting)
///
/// # Returns
/// RPM value corresponding to the duty cycle
///
/// # Examples
/// ```
/// use carvera_spindle::duty_to_rpm;
///
/// // 10% duty (1000) -> min RPM
/// assert_eq!(duty_to_rpm(1000, 2000, 12500), 2000);
///
/// // 90% duty (9000) -> max RPM
/// assert_eq!(duty_to_rpm(9000, 2000, 12500), 12500);
///
/// // 50% duty (5000) -> midpoint RPM
/// assert_eq!(duty_to_rpm(5000, 2000, 12500), 7250);
/// ```
pub fn duty_to_rpm(output_duty: u32, min_rpm: u32, max_rpm: u32) -> u32 {
    if output_duty <= 1000 {
        return min_rpm;
    }
    if output_duty >= 9000 {
        return max_rpm;
    }
    // Linear interpolation: (duty - 1000) / 8000 * (max - min) + min
    let duty_fraction = output_duty - 1000; // 0-8000 range
    min_rpm + (duty_fraction * (max_rpm - min_rpm) + 4000) / 8000
}

/// Calculate RPM from encoder frequency.
///
/// Converts the frequency of encoder pulses to RPM.
///
/// # Arguments
/// * `frequency_hz` - Encoder pulse frequency in Hz
/// * `pulses_per_rev` - Number of encoder pulses per revolution (typically 4)
///
/// # Returns
/// RPM value
///
/// # Examples
/// ```
/// use carvera_spindle::frequency_to_rpm;
///
/// // 133.3 Hz at 4 ppr = 2000 RPM
/// assert_eq!(frequency_to_rpm(133, 4), 1995);  // Slight rounding
///
/// // 400 Hz at 4 ppr = 6000 RPM
/// assert_eq!(frequency_to_rpm(400, 4), 6000);
/// ```
pub fn frequency_to_rpm(frequency_hz: u32, pulses_per_rev: u32) -> u32 {
    if pulses_per_rev == 0 {
        return 0;
    }
    // RPM = frequency * 60 / pulses_per_rev
    (frequency_hz * 60 + pulses_per_rev / 2) / pulses_per_rev
}

/// Convert spindle RPM to motor RPM using belt ratio.
///
/// Uses u64 intermediate math with round-to-nearest for accuracy.
/// motor_rpm = spindle_rpm * 1000 / belt_ratio_x1000
///
/// # Arguments
/// * `spindle_rpm` - Spindle RPM (after belt ratio)
/// * `belt_ratio_x1000` - Belt ratio multiplied by 1000 (e.g., 1635 for 1.635:1)
///
/// # Returns
/// Motor RPM value
///
/// # Examples
/// ```
/// use carvera_spindle::spindle_to_motor_rpm;
///
/// // 1000 spindle RPM at 1.635:1 ratio = ~612 motor RPM
/// assert_eq!(spindle_to_motor_rpm(1000, 1635), 612);
///
/// // 10000 spindle RPM at 1.635:1 ratio = ~6116 motor RPM
/// assert_eq!(spindle_to_motor_rpm(10000, 1635), 6116);
///
/// // 0 RPM stays 0
/// assert_eq!(spindle_to_motor_rpm(0, 1635), 0);
/// ```
#[inline]
pub fn spindle_to_motor_rpm(spindle_rpm: u32, belt_ratio_x1000: u32) -> u32 {
    if belt_ratio_x1000 == 0 {
        return 0;
    }
    ((spindle_rpm as u64 * 1000 + belt_ratio_x1000 as u64 / 2) / belt_ratio_x1000 as u64) as u32
}

/// Convert motor RPM to ESCON output duty (0-1000 scale).
///
/// Reverse of duty_to_rpm: maps motor RPM to the 10-90% duty range.
/// Uses u64 intermediate math to avoid overflow.
///
/// Formula: duty = 100 + (rpm - min_rpm) * 800 / (max_rpm - min_rpm)
///
/// # Arguments
/// * `motor_rpm` - Motor RPM to convert
/// * `min_rpm` - RPM at 10% duty (ESCON minimum)
/// * `max_rpm` - RPM at 90% duty (ESCON maximum)
///
/// # Returns
/// Output duty in 0-1000 scale (0.1% resolution)
///
/// # Examples
/// ```
/// use carvera_spindle::motor_rpm_to_output_duty;
///
/// // Min RPM -> 10% duty (100)
/// assert_eq!(motor_rpm_to_output_duty(2000, 2000, 12500), 100);
///
/// // Max RPM -> 90% duty (900)
/// assert_eq!(motor_rpm_to_output_duty(12500, 2000, 12500), 900);
///
/// // Midpoint RPM -> 50% duty (500)
/// assert_eq!(motor_rpm_to_output_duty(7250, 2000, 12500), 500);
///
/// // Below min RPM -> 10% duty (100)
/// assert_eq!(motor_rpm_to_output_duty(1000, 2000, 12500), 100);
///
/// // Above max RPM -> 90% duty (900)
/// assert_eq!(motor_rpm_to_output_duty(15000, 2000, 12500), 900);
/// ```
#[inline]
pub fn motor_rpm_to_output_duty(motor_rpm: u32, min_rpm: u32, max_rpm: u32) -> u16 {
    if motor_rpm <= min_rpm {
        return 100; // 10% minimum
    }
    if motor_rpm >= max_rpm {
        return 900; // 90% maximum
    }
    // duty = 100 + (motor_rpm - min_rpm) * 800 / (max_rpm - min_rpm)
    let range = max_rpm - min_rpm;
    let offset = motor_rpm - min_rpm;
    (100 + ((offset as u64 * 800 + range as u64 / 2) / range as u64) as u32).min(900) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy linear scaling function preserved for test coverage.
    /// Not used in the live control path (replaced by calibration-based mapping).
    fn calculate_output(
        input_duty: u16,
        timed_out: bool,
        min_duty: u16,
        max_duty: u16,
    ) -> (u16, bool) {
        if timed_out || input_duty == 0 {
            (min_duty, false)
        } else {
            let range = max_duty - min_duty;
            let scaled = min_duty as u32 + (input_duty as u32 * range as u32) / 1000;
            let clamped = (scaled as u16).min(max_duty);
            (clamped, true)
        }
    }

    // --- calculate_output tests (0-1000 scale = 0.1% resolution) ---
    //
    // The function scales Carvera's 0-100% input to ESCON's 10-90% output:
    //   output = 100 + (input * 800) / 1000
    //
    // Examples:
    //   0% input   → 10% output  (100)
    //   50% input  → 50% output  (100 + 500*800/1000 = 500)
    //   100% input → 90% output  (100 + 1000*800/1000 = 900)

    const TEST_MIN_DUTY: u16 = 100; // 10.0%
    const TEST_MAX_DUTY: u16 = 900; // 90.0%

    #[test]
    fn test_zero_input_returns_min_disabled() {
        // 0% input → 10% output, disabled (spindle off)
        let (duty, enabled) = calculate_output(0, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 100);
        assert!(!enabled);
    }

    #[test]
    fn test_small_input_scales_to_near_min() {
        // Small inputs (1-99) scale to 10.x% output, enabled
        // e.g., 10% input → 100 + 100*800/1000 = 180 (18.0%)
        let (duty, enabled) = calculate_output(100, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 180); // 10% input → 18% output
        assert!(enabled);

        // 1% input → 100 + 10*800/1000 = 108 (10.8%)
        let (duty, enabled) = calculate_output(10, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 108);
        assert!(enabled);
    }

    #[test]
    fn test_scaling_formula() {
        // Verify the scaling formula: output = 100 + (input * 800) / 1000
        let test_cases = [
            (0, 100),    // 0% → 10%
            (125, 200),  // 12.5% → 20% (100 + 125*800/1000 = 200)
            (250, 300),  // 25% → 30%
            (375, 400),  // 37.5% → 40%
            (500, 500),  // 50% → 50%
            (625, 600),  // 62.5% → 60%
            (750, 700),  // 75% → 70%
            (875, 800),  // 87.5% → 80%
            (1000, 900), // 100% → 90%
        ];

        for (input, expected_output) in test_cases {
            let (duty, enabled) = calculate_output(input, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
            assert_eq!(
                duty, expected_output,
                "input {} should scale to {}",
                input, expected_output
            );
            assert!(enabled || input == 0, "input {} should be enabled", input);
        }
    }

    #[test]
    fn test_48_5_percent_scales_correctly() {
        // This is the specific case from the bug report:
        // Carvera sends 48.5% for S10000 command
        // Should scale to: 100 + 485*800/1000 = 100 + 388 = 488 (48.8%)
        let (duty, enabled) = calculate_output(485, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 488);
        assert!(enabled);
    }

    #[test]
    fn test_100_percent_input_gives_max() {
        // 100% input → 90% output
        let (duty, enabled) = calculate_output(1000, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 900);
        assert!(enabled);
    }

    #[test]
    fn test_over_100_percent_clamps_to_max() {
        // Inputs above 1000 should clamp to max (900)
        // 110% input → would be 100 + 1100*800/1000 = 980, clamped to 900
        let (duty, enabled) = calculate_output(1100, false, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 900);
        assert!(enabled);
    }

    #[test]
    fn test_timeout_forces_disabled() {
        let (duty, enabled) = calculate_output(500, true, TEST_MIN_DUTY, TEST_MAX_DUTY);
        assert_eq!(duty, 100); // 10.0%
        assert!(!enabled);
    }

    // --- duty_to_rpm tests (0-10000 scale = 0.01% resolution) ---

    #[test]
    fn test_duty_to_rpm_at_min() {
        // 10% duty (1000 in 0.01% units) -> min RPM
        assert_eq!(duty_to_rpm(1000, 2000, 12500), 2000);
    }

    #[test]
    fn test_duty_to_rpm_at_max() {
        // 90% duty (9000 in 0.01% units) -> max RPM
        assert_eq!(duty_to_rpm(9000, 2000, 12500), 12500);
    }

    #[test]
    fn test_duty_to_rpm_midpoint() {
        // 50% duty (5000 in 0.01% units) -> midpoint
        // (5000 - 1000) / 8000 * (12500 - 2000) + 2000 = 4000/8000 * 10500 + 2000 = 7250
        assert_eq!(duty_to_rpm(5000, 2000, 12500), 7250);
    }

    #[test]
    fn test_duty_to_rpm_below_min() {
        // Below 10% should clamp to min RPM
        assert_eq!(duty_to_rpm(0, 2000, 12500), 2000);
        assert_eq!(duty_to_rpm(500, 2000, 12500), 2000);
        assert_eq!(duty_to_rpm(999, 2000, 12500), 2000);
    }

    #[test]
    fn test_duty_to_rpm_above_max() {
        // Above 90% should clamp to max RPM
        assert_eq!(duty_to_rpm(9001, 2000, 12500), 12500);
        assert_eq!(duty_to_rpm(10000, 2000, 12500), 12500);
    }

    #[test]
    fn test_duty_to_rpm_zero() {
        // 0% duty should return min RPM
        assert_eq!(duty_to_rpm(0, 2000, 12500), 2000);
    }

    // --- spindle_to_motor_rpm tests ---

    #[test]
    fn test_spindle_to_motor_rpm_at_1000() {
        // 1000 spindle RPM at 1.635:1 = round(1000 * 1000 / 1635) = 612
        assert_eq!(spindle_to_motor_rpm(1000, 1635), 612);
    }

    #[test]
    fn test_spindle_to_motor_rpm_at_10000() {
        // 10000 spindle RPM at 1.635:1 = 10000 * 1000 / 1635 = 6116
        assert_eq!(spindle_to_motor_rpm(10000, 1635), 6116);
    }

    #[test]
    fn test_spindle_to_motor_rpm_at_20000() {
        // 20000 spindle RPM at 1.635:1 = 20000 * 1000 / 1635 = 12232
        assert_eq!(spindle_to_motor_rpm(20000, 1635), 12232);
    }

    #[test]
    fn test_spindle_to_motor_rpm_zero() {
        assert_eq!(spindle_to_motor_rpm(0, 1635), 0);
    }

    #[test]
    fn test_spindle_to_motor_rpm_zero_belt_ratio() {
        // Zero belt ratio should return 0 instead of panicking
        assert_eq!(spindle_to_motor_rpm(1000, 0), 0);
        assert_eq!(spindle_to_motor_rpm(0, 0), 0);
    }

    // --- motor_rpm_to_output_duty tests ---

    #[test]
    fn test_motor_rpm_to_output_duty_at_min() {
        // Min RPM -> 10% duty (100)
        assert_eq!(motor_rpm_to_output_duty(2000, 2000, 12500), 100);
    }

    #[test]
    fn test_motor_rpm_to_output_duty_at_max() {
        // Max RPM -> 90% duty (900)
        assert_eq!(motor_rpm_to_output_duty(12500, 2000, 12500), 900);
    }

    #[test]
    fn test_motor_rpm_to_output_duty_at_midpoint() {
        // Midpoint: (2000 + 12500) / 2 = 7250 RPM
        // (7250 - 2000) * 800 / 10500 + 100 = 5250 * 800 / 10500 + 100 = 400 + 100 = 500
        assert_eq!(motor_rpm_to_output_duty(7250, 2000, 12500), 500);
    }

    #[test]
    fn test_motor_rpm_to_output_duty_below_min() {
        // Below min -> clamp to 100
        assert_eq!(motor_rpm_to_output_duty(1000, 2000, 12500), 100);
        assert_eq!(motor_rpm_to_output_duty(0, 2000, 12500), 100);
    }

    #[test]
    fn test_motor_rpm_to_output_duty_above_max() {
        // Above max -> clamp to 900
        assert_eq!(motor_rpm_to_output_duty(15000, 2000, 12500), 900);
    }

    #[test]
    fn test_motor_rpm_to_output_duty_roundtrip() {
        // Verify roundtrip: motor_rpm -> duty -> motor_rpm (should be close)
        // Note: Integer division loses precision, so we allow ±15 RPM tolerance
        // This is acceptable because:
        // - 0-1000 duty scale has 0.1% steps = ~10.5 RPM per step
        // - Roundtrip through integer division accumulates rounding errors
        let original_rpm = 6000u32;
        let duty = motor_rpm_to_output_duty(original_rpm, 2000, 12500);
        // duty is 0-1000 scale, duty_to_rpm expects 0-10000, so multiply by 10
        let recovered_rpm = duty_to_rpm((duty as u32) * 10, 2000, 12500);
        assert!(
            recovered_rpm.abs_diff(original_rpm) <= 15,
            "Roundtrip failed: {} -> duty {} -> {}",
            original_rpm,
            duty,
            recovered_rpm
        );
    }

    // --- frequency_to_rpm tests ---

    #[test]
    fn test_frequency_to_rpm_basic() {
        // 400 Hz at 4 ppr = 6000 RPM
        assert_eq!(frequency_to_rpm(400, 4), 6000);
    }

    #[test]
    fn test_frequency_to_rpm_zero_freq() {
        assert_eq!(frequency_to_rpm(0, 4), 0);
    }

    #[test]
    fn test_frequency_to_rpm_zero_pulses() {
        // Should not panic, return 0
        assert_eq!(frequency_to_rpm(400, 0), 0);
    }
}

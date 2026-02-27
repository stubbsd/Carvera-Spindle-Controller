//! Speed calibration system.
//!
//! Detects a 3-note zigzag start sequence (6000 -> 12000 -> 9000 RPM via speed
//! changes, no M5 stops between notes), then records PWM duty at 386 known
//! speed steps (750-20000 RPM in 50 RPM increments). The resulting correction
//! table enables piecewise-linear interpolation for accurate duty->RPM mapping
//! at runtime.

pub mod recorder;
pub mod sequence;
pub mod table;

// Re-export public API so callers can use crate::calibration::* unchanged
pub use recorder::{CalEvent, CalibrationRecorder};
pub use sequence::SequenceDetector;
pub use table::{
    apply_calibration, calibration_count, clear_calibration, correct_duty, duty_to_calibrated_rpm,
    get_calibration_point, has_calibration, read_calibration_table,
};

use crate::state::config::CARVERA_SPINDLE_MAX_RPM;

// ============================================================================
// Constants
// ============================================================================

/// Number of calibration steps (750 to 20000 RPM in 50 RPM increments)
pub const CAL_STEPS: usize = 386;

/// First calibration RPM
pub const CAL_START_RPM: u16 = 750;

/// RPM increment between steps
pub const CAL_STEP_RPM: u16 = 50;

/// Tolerance for speed matching in the start sequence (+/-12%)
pub(crate) const SEQUENCE_TOLERANCE_PCT: u32 = 12;

/// ON detection threshold (duty > 1.0% = 100 in 0-10000 scale)
const ON_THRESHOLD: u16 = 100;

/// OFF detection threshold (duty < 0.5% = 50 in 0-10000 scale)
const OFF_THRESHOLD: u16 = 50;

/// Time OFF must persist to count as a gap (ms)
const OFF_DEBOUNCE_MS: u64 = 40;

/// Timeout: abort if signal lost during active phase (ms)
const SIGNAL_TIMEOUT_MS: u64 = 10_000;

/// Sequence detector: minimum ON duration for a note (filters glitches)
const SEQ_NOTE_MIN_MS: u64 = 200;

/// Sequence detector: maximum ON duration for a note (8s observed + 50% margin)
const SEQ_NOTE_MAX_MS: u64 = 12_000;

/// Grace period for duty mismatches during note transitions (ms).
/// MEASURED_DUTY batch averaging (32ms windows) produces intermediate values
/// when Carvera changes PWM mid-batch. 100ms covers worst-case 2 batch windows.
const SEQ_TRANSITION_GRACE_MS: u64 = 100;

/// Settle time at start of each step before recording (ms)
const STEP_SETTLE_MS: u64 = 10;

/// Recording window duration per step (ms)
const STEP_RECORD_MS: u64 = 80;

/// Duty change threshold to detect a speed change (new fast path, no M5 needed).
/// ~half the ~24-count difference between adjacent 50 RPM calibration steps.
const SPEED_CHANGE_THRESHOLD: u16 = 12;

/// Settle time after speed-change detection before recording (ms).
/// Accounts for 32ms MEASURED_DUTY batch averaging window + margin.
const SPEED_CHANGE_SETTLE_MS: u64 = 50;

/// Announce phase duration after sequence detected (ms)
const ANNOUNCE_MS: u64 = 200;

// ============================================================================
// Types
// ============================================================================

/// A single calibration data point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalibrationPoint {
    /// Expected RPM for this step
    pub expected_rpm: u16,
    /// Measured PWM duty (0-10000 scale)
    pub measured_duty: u16,
}

/// Full calibration table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalibrationTable {
    pub points: [CalibrationPoint; CAL_STEPS],
    pub count: u16,
}

impl Default for CalibrationTable {
    fn default() -> Self {
        Self {
            points: [CalibrationPoint::default(); CAL_STEPS],
            count: 0,
        }
    }
}

/// Calibration phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum CalPhase {
    /// No calibration active (or no cal data on boot)
    #[default]
    NoCal = 0,
    /// Listening for the 3-note start sequence
    Detecting = 1,
    /// Start sequence detected, announcing
    SequenceDetected = 2,
    /// Recording calibration steps
    Recording = 3,
    /// Calibration complete (temporary flash message)
    Complete = 4,
    /// Calibration aborted (signal lost)
    Aborted = 5,
    /// Calibration data loaded from flash
    Loaded = 6,
    /// Calibration data cleared from flash
    Cleared = 7,
}

/// Status snapshot published to LCD task via Watch channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct CalibrationStatus {
    pub phase: CalPhase,
    pub step: u16,
    pub total_steps: u16,
    pub expected_rpm: u16,
    pub measured_duty: u16,
    pub recording: bool,
}

// ============================================================================
// Helpers
// ============================================================================

/// Convert RPM to expected PWM duty (0-10000 scale) using Carvera's linear formula.
/// duty = (rpm / CARVERA_SPINDLE_MAX_RPM) * 10000
pub fn rpm_to_expected_duty(rpm: u16) -> u16 {
    ((rpm as u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16
}

/// Check if a measured duty matches the expected duty for a given RPM within tolerance.
pub fn duty_matches_speed(duty: u16, rpm: u16) -> bool {
    let expected = rpm_to_expected_duty(rpm);
    if expected == 0 {
        return duty < ON_THRESHOLD;
    }
    let tolerance = (expected as u32 * SEQUENCE_TOLERANCE_PCT) / 100;
    let low = expected.saturating_sub(tolerance as u16);
    let high = expected.saturating_add(tolerance as u16);
    duty >= low && duty <= high
}

/// Check if duty indicates signal is OFF.
fn is_off(duty: u16) -> bool {
    duty < OFF_THRESHOLD
}

/// Check if duty indicates signal is ON.
fn is_on(duty: u16) -> bool {
    duty > ON_THRESHOLD
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpm_to_expected_duty() {
        // 6000 RPM -> 6000/20437 * 10000 = 2936
        let duty = rpm_to_expected_duty(6000);
        assert!(duty >= 2900 && duty <= 2950, "6000 RPM duty={}", duty);

        // 12000 RPM -> 12000/20437 * 10000 = 5872
        let duty = rpm_to_expected_duty(12000);
        assert!(duty >= 5850 && duty <= 5900, "12000 RPM duty={}", duty);

        // 20000 RPM -> 20000/20437 * 10000 = 9786
        let duty = rpm_to_expected_duty(20000);
        assert!(duty >= 9750 && duty <= 9800, "20000 RPM duty={}", duty);

        // 0 RPM -> 0
        assert_eq!(rpm_to_expected_duty(0), 0);
    }

    #[test]
    fn test_duty_matches_speed() {
        let expected = rpm_to_expected_duty(6000);
        assert!(duty_matches_speed(expected, 6000));
        let tolerance = (expected as u32 * 12) / 100;
        assert!(duty_matches_speed(expected + tolerance as u16, 6000));
        assert!(duty_matches_speed(expected - tolerance as u16, 6000));
        assert!(!duty_matches_speed(expected + tolerance as u16 + 1, 6000));
    }

    #[test]
    fn test_sequence_notes_non_overlapping() {
        let d6 = rpm_to_expected_duty(6000) as u32;
        let d9 = rpm_to_expected_duty(9000) as u32;
        let d12 = rpm_to_expected_duty(12000) as u32;

        let t6_high = d6 + d6 * 12 / 100;
        let t9_low = d9 - d9 * 12 / 100;
        let t9_high = d9 + d9 * 12 / 100;
        let t12_low = d12 - d12 * 12 / 100;

        assert!(
            t6_high < t9_low,
            "6000 and 9000 RPM ranges overlap: {} >= {}",
            t6_high,
            t9_low
        );
        assert!(
            t9_high < t12_low,
            "9000 and 12000 RPM ranges overlap: {} >= {}",
            t9_high,
            t12_low
        );
    }

    #[test]
    fn test_cal_step_rpms() {
        assert_eq!(CAL_START_RPM + 0 * CAL_STEP_RPM, 750);
        assert_eq!(CAL_START_RPM + 385 * CAL_STEP_RPM, 20000);
        assert_eq!(CAL_STEPS, 386);
    }

    #[test]
    fn test_is_off_threshold() {
        assert!(is_off(0));
        assert!(is_off(49));
        assert!(!is_off(50));
        assert!(!is_off(100));
    }

    #[test]
    fn test_is_on_threshold() {
        assert!(!is_on(0));
        assert!(!is_on(100));
        assert!(is_on(101));
        assert!(is_on(5000));
    }

    #[test]
    fn test_duty_matches_speed_zero_rpm() {
        assert!(duty_matches_speed(0, 0));
        assert!(duty_matches_speed(50, 0));
        assert!(!duty_matches_speed(200, 0));
    }

    #[test]
    fn test_rpm_to_expected_duty_all_sequence_speeds() {
        let d6 = rpm_to_expected_duty(6000);
        let d9 = rpm_to_expected_duty(9000);
        let d12 = rpm_to_expected_duty(12000);

        assert!(d6 > 0);
        assert!(d9 > d6);
        assert!(d12 > d9);
        assert!(d12 < 10000);
    }

    #[test]
    fn test_calibration_status_default() {
        let status = CalibrationStatus::default();
        assert_eq!(status.phase, CalPhase::NoCal);
        assert_eq!(status.step, 0);
        assert_eq!(status.total_steps, 0);
        assert_eq!(status.expected_rpm, 0);
        assert_eq!(status.measured_duty, 0);
        assert!(!status.recording);
    }

    #[test]
    fn test_calibration_point_default() {
        let point = CalibrationPoint::default();
        assert_eq!(point.expected_rpm, 0);
        assert_eq!(point.measured_duty, 0);
    }

    #[test]
    fn test_calibration_table_default() {
        let table = CalibrationTable::default();
        assert_eq!(table.count, 0);
        for p in &table.points {
            assert_eq!(p.expected_rpm, 0);
            assert_eq!(p.measured_duty, 0);
        }
    }
}

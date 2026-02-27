//! Calibration table storage, lookup, and interpolation.
//!
//! Lock-free static storage for calibration data with piecewise-linear
//! interpolation for accurate duty-to-RPM mapping at runtime.

use core::sync::atomic::{AtomicU16, AtomicU32, Ordering};

use super::{CAL_STEP_RPM, CAL_STEPS, CalibrationPoint, CalibrationTable, OFF_THRESHOLD};
use crate::state::config::CARVERA_SPINDLE_MAX_RPM;

// ============================================================================
// Static Calibration Storage (lock-free, write-once-read-many)
// ============================================================================

/// Packed calibration points: high 16 bits = expected_rpm, low 16 bits = measured_duty
#[allow(clippy::declare_interior_mutable_const)]
static CAL_POINTS: [AtomicU32; CAL_STEPS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; CAL_STEPS]
};

/// Number of valid calibration points (0 = no calibration)
static CAL_COUNT: AtomicU16 = AtomicU16::new(0);

/// Store a calibration table into the static atomic storage.
///
/// Writes all points with `Relaxed` ordering, then publishes the count
/// with `SeqCst` so readers see a consistent snapshot. Called after
/// loading from flash or completing a live calibration run.
///
/// # Arguments
/// * `table` - Table with `count` valid entries in `points`. Each entry
///   pairs an expected RPM with the measured PWM duty at that speed.
pub fn apply_calibration(table: &CalibrationTable) {
    // Write all points first
    for (i, point) in table.points.iter().enumerate().take(table.count as usize) {
        let packed = ((point.expected_rpm as u32) << 16) | (point.measured_duty as u32);
        CAL_POINTS[i].store(packed, Ordering::Relaxed);
    }
    // SeqCst fence ensures all points are visible before count is set
    CAL_COUNT.store(table.count, Ordering::SeqCst);
}

/// Read a calibration point from static storage.
pub fn get_calibration_point(index: usize) -> Option<CalibrationPoint> {
    let count = CAL_COUNT.load(Ordering::SeqCst) as usize;
    if index >= count {
        return None;
    }
    let packed = CAL_POINTS[index].load(Ordering::Relaxed);
    Some(CalibrationPoint {
        expected_rpm: (packed >> 16) as u16,
        measured_duty: (packed & 0xFFFF) as u16,
    })
}

/// Check if calibration data is loaded.
pub fn has_calibration() -> bool {
    CAL_COUNT.load(Ordering::SeqCst) > 0
}

/// Get the number of calibration points.
pub fn calibration_count() -> u16 {
    CAL_COUNT.load(Ordering::SeqCst)
}

/// Clear all runtime calibration data by resetting the point count to zero.
///
/// The underlying atomic point storage is not zeroed (readers check count
/// first), so this is a single atomic store. Use after a user-initiated
/// "clear calibration" command or before loading new data from flash.
pub fn clear_calibration() {
    CAL_COUNT.store(0, Ordering::SeqCst);
}

/// Read the current calibration table back from static atomic storage.
///
/// Reconstructs a [`CalibrationTable`] by unpacking each `AtomicU32`
/// (upper 16 bits = RPM, lower 16 bits = duty). Returns `None` if no
/// calibration data is loaded (`CAL_COUNT == 0`).
///
/// Used by the calibration dump command to serialize the table for
/// RTT output and by tests to verify round-trip storage.
pub fn read_calibration_table() -> Option<CalibrationTable> {
    let count = CAL_COUNT.load(Ordering::SeqCst);
    if count == 0 {
        return None;
    }
    let mut table = CalibrationTable {
        count,
        ..CalibrationTable::default()
    };
    for (i, atomic) in CAL_POINTS.iter().enumerate().take(count as usize) {
        let packed = atomic.load(Ordering::Relaxed);
        table.points[i] = CalibrationPoint {
            expected_rpm: (packed >> 16) as u16,
            measured_duty: (packed & 0xFFFF) as u16,
        };
    }
    Some(table)
}

// ============================================================================
// Runtime Correction -- piecewise linear interpolation
// ============================================================================

/// Interpolate the calibration table to find the true RPM for a raw measured duty.
///
/// Returns `Some(true_rpm)` if calibration data exists and is not bypassed,
/// or `None` if no calibration data or calibration sequence is active.
///
/// Algorithm:
/// 1. Compute approximate RPM from raw duty using standard Carvera formula
/// 2. Use approximate RPM to directly index into the evenly-spaced cal table (O(1))
/// 3. Float interpolation within the correct bracket with clamping
///
/// This avoids the old binary-search-by-duty approach which could land in the
/// wrong bracket when Carvera's actual duty doesn't perfectly track its RPM
/// (e.g., S2501 landing in a compressed duty region -> 62 RPM error per count).
fn interpolate_rpm(raw_duty: u16) -> Option<u32> {
    // Bypass correction during active calibration recording/dump/clear
    if crate::state::CAL_RECORDING.load(Ordering::SeqCst) {
        return None;
    }

    let count = CAL_COUNT.load(Ordering::SeqCst) as usize;
    if count < 2 {
        if count == 1 {
            return Some(CAL_POINTS[0].load(Ordering::Relaxed) >> 16);
        }
        return None;
    }

    // Step 1: Approximate RPM from standard formula (what Carvera intended)
    let approx_rpm = raw_duty as f32 * (CARVERA_SPINDLE_MAX_RPM as f32 / 10000.0);

    // Step 2: Direct index into evenly-spaced cal table (no binary search)
    let first_rpm = (CAL_POINTS[0].load(Ordering::Relaxed) >> 16) as f32;
    let step = CAL_STEP_RPM as f32;
    let raw_idx = (approx_rpm - first_rpm) / step;
    // Truncation toward zero is equivalent to floor for non-negative values.
    // For negative (approx_rpm below first cal point), clamp to 0.
    let idx = if raw_idx < 0.0 {
        0usize
    } else {
        raw_idx as usize
    };
    let lo_idx = idx.min(count - 2);
    let hi_idx = lo_idx + 1;

    // Step 3: Load bracket calibration data
    let lo_packed = CAL_POINTS[lo_idx].load(Ordering::Relaxed);
    let hi_packed = CAL_POINTS[hi_idx].load(Ordering::Relaxed);
    let lo_rpm = (lo_packed >> 16) as f32;
    let lo_duty = (lo_packed & 0xFFFF) as f32;
    let hi_rpm = (hi_packed >> 16) as f32;
    let hi_duty = (hi_packed & 0xFFFF) as f32;

    // Step 4: Float interpolation with clamping
    let duty_range = hi_duty - lo_duty;
    // abs() requires libm in no_std, use conditional instead
    if duty_range > -0.5 && duty_range < 0.5 {
        return Some(lo_rpm as u32); // Degenerate: can't distinguish in this range
    }

    let t = ((raw_duty as f32) - lo_duty) / duty_range;
    let t = t.clamp(0.0, 1.0); // Prevent extrapolation -> prevents amplification

    let true_rpm = lo_rpm + t * (hi_rpm - lo_rpm);
    // round without method (no_std compatible): add 0.5 and truncate
    Some((true_rpm + 0.5) as u32)
}

/// Map a raw measured duty (0-10000) to the corrected duty that Carvera
/// *should* have sent for the true RPM at that operating point.
///
/// Uses [`interpolate_rpm`] to find the true RPM from the calibration
/// table, then converts back to the ideal duty via the standard linear
/// formula. Returns 0 for duties below `OFF_THRESHOLD` (spindle off)
/// and passes through the input unchanged when no calibration data exists.
///
/// This is the primary runtime correction applied in the control loop
/// before the duty value is forwarded to the ESCON output stage.
pub fn correct_duty(raw_duty: u16) -> u16 {
    // Guard: below OFF_THRESHOLD means spindle is off, skip calibration lookup
    if raw_duty < OFF_THRESHOLD {
        return 0;
    }
    match interpolate_rpm(raw_duty) {
        Some(true_rpm) => {
            ((true_rpm * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16
        }
        None => raw_duty,
    }
}

/// Convert a raw measured duty (0-10000) directly to calibrated spindle RPM.
///
/// When calibration data exists, returns the interpolated true RPM from
/// the piecewise-linear calibration table. This avoids the lossy
/// duty->RPM->duty->RPM round-trip that `correct_duty()` + the display
/// formula would produce (up to +/-1 RPM quantization error).
///
/// When no calibration data exists, falls back to the standard linear
/// formula: `rpm = raw_duty * CARVERA_SPINDLE_MAX_RPM / 10000`.
///
/// Used by the display pipeline to show accurate actual RPM on the LCD.
pub fn duty_to_calibrated_rpm(raw_duty: u16) -> u32 {
    match interpolate_rpm(raw_duty) {
        Some(true_rpm) => true_rpm,
        None => (raw_duty as u32 * CARVERA_SPINDLE_MAX_RPM + 5000) / 10000,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::{
        CAL_START_RPM, CAL_STEP_RPM, CAL_STEPS, CalibrationPoint, CalibrationTable,
    };
    use serial_test::serial;

    /// Drop guard that resets shared static calibration state even on panic.
    struct CalTestGuard;

    impl CalTestGuard {
        fn new() -> Self {
            CAL_COUNT.store(0, Ordering::SeqCst);
            crate::state::CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);
            crate::state::CAL_RECORDING.store(false, Ordering::SeqCst);
            Self
        }
    }

    impl Drop for CalTestGuard {
        fn drop(&mut self) {
            CAL_COUNT.store(0, Ordering::SeqCst);
            crate::state::CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);
            crate::state::CAL_RECORDING.store(false, Ordering::SeqCst);
        }
    }

    fn two_point_cal_table(rpm1: u16, duty1: u16, rpm2: u16, duty2: u16) -> CalibrationTable {
        let mut pts = [CalibrationPoint::default(); CAL_STEPS];
        pts[0] = CalibrationPoint {
            expected_rpm: rpm1,
            measured_duty: duty1,
        };
        pts[1] = CalibrationPoint {
            expected_rpm: rpm2,
            measured_duty: duty2,
        };
        CalibrationTable {
            points: pts,
            count: 2,
        }
    }

    fn full_386_point_cal_table(offset: u16) -> CalibrationTable {
        let mut table = CalibrationTable::default();
        table.count = CAL_STEPS as u16;
        for i in 0..CAL_STEPS {
            let rpm = CAL_START_RPM + i as u16 * CAL_STEP_RPM;
            let ideal_duty = ((rpm as u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16;
            table.points[i] = CalibrationPoint {
                expected_rpm: rpm,
                measured_duty: ideal_duty + offset,
            };
        }
        table
    }

    #[test]
    #[serial]
    fn test_correct_duty_no_calibration() {
        let _guard = CalTestGuard::new();
        assert_eq!(correct_duty(5000), 5000);
    }

    #[test]
    #[serial]
    fn test_correct_duty_with_calibration() {
        let _guard = CalTestGuard::new();
        let table = two_point_cal_table(500, 300, 1000, 550);
        apply_calibration(&table);

        let ideal_500 =
            ((500u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert_eq!(correct_duty(300), ideal_500);

        let ideal_1000 =
            ((1000u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert_eq!(correct_duty(550), ideal_1000);

        let corrected = correct_duty(425);
        let ideal_750 =
            ((750u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert_eq!(
            corrected, ideal_750,
            "Expected {}, got {}",
            ideal_750, corrected
        );

        assert_eq!(correct_duty(100), ideal_500);
        assert_eq!(correct_duty(800), ideal_1000);
    }

    #[test]
    #[serial]
    fn test_correct_duty_bypass_during_calibration() {
        let _guard = CalTestGuard::new();
        let table = two_point_cal_table(500, 300, 1000, 550);
        apply_calibration(&table);

        crate::state::CAL_RECORDING.store(false, Ordering::SeqCst);
        let corrected = correct_duty(300);
        let ideal_500 =
            ((500u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert_eq!(corrected, ideal_500);

        crate::state::CAL_RECORDING.store(true, Ordering::SeqCst);
        assert_eq!(correct_duty(300), 300);
        assert_eq!(correct_duty(5000), 5000);
    }

    #[test]
    #[serial]
    fn test_apply_and_get_calibration() {
        let _guard = CalTestGuard::new();
        let mut table = CalibrationTable::default();
        table.points[0] = CalibrationPoint {
            expected_rpm: 500,
            measured_duty: 245,
        };
        table.points[1] = CalibrationPoint {
            expected_rpm: 750,
            measured_duty: 367,
        };
        table.count = 2;

        apply_calibration(&table);
        assert!(has_calibration());
        assert_eq!(calibration_count(), 2);

        let p0 = get_calibration_point(0).unwrap();
        assert_eq!(p0.expected_rpm, 500);
        assert_eq!(p0.measured_duty, 245);

        let p1 = get_calibration_point(1).unwrap();
        assert_eq!(p1.expected_rpm, 750);
        assert_eq!(p1.measured_duty, 367);

        assert!(get_calibration_point(2).is_none());
    }

    #[test]
    #[serial]
    fn test_has_calibration_without_data() {
        let _guard = CalTestGuard::new();
        assert!(!has_calibration());
        assert_eq!(calibration_count(), 0);
    }

    #[test]
    #[serial]
    fn test_get_calibration_point_out_of_bounds() {
        let _guard = CalTestGuard::new();
        assert!(get_calibration_point(0).is_none());
        assert!(get_calibration_point(100).is_none());
    }

    #[test]
    #[serial]
    fn test_correct_duty_identical_measured_duties() {
        let _guard = CalTestGuard::new();
        let table = two_point_cal_table(500, 300, 750, 300);
        apply_calibration(&table);

        let result = correct_duty(300);
        assert!(result > 0);
    }

    #[test]
    #[serial]
    fn test_correct_duty_single_point() {
        let _guard = CalTestGuard::new();
        let mut table = CalibrationTable::default();
        table.points[0] = CalibrationPoint {
            expected_rpm: 5000,
            measured_duty: 2500,
        };
        table.count = 1;
        apply_calibration(&table);

        let ideal =
            ((5000u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert_eq!(correct_duty(1000), ideal);
        assert_eq!(correct_duty(2500), ideal);
        assert_eq!(correct_duty(5000), ideal);
    }

    #[test]
    #[serial]
    fn test_correct_duty_full_range_monotonic() {
        let _guard = CalTestGuard::new();
        let table = full_386_point_cal_table(50);
        apply_calibration(&table);

        let mut prev = correct_duty(table.points[0].measured_duty);
        for i in 1..CAL_STEPS {
            let corrected = correct_duty(table.points[i].measured_duty);
            assert!(
                corrected >= prev,
                "correct_duty not monotonic at step {}: {} < {}",
                i,
                corrected,
                prev
            );
            prev = corrected;
        }
    }

    #[test]
    #[serial]
    fn test_correct_duty_zero_duty_input() {
        let _guard = CalTestGuard::new();
        let table = two_point_cal_table(500, 200, 1000, 450);
        apply_calibration(&table);

        assert_eq!(correct_duty(0), 0);
    }

    #[test]
    #[serial]
    fn test_correct_duty_zero_input_does_not_exceed_enable_threshold() {
        let _guard = CalTestGuard::new();
        let table = two_point_cal_table(CAL_START_RPM, 400, 1500, 800);
        apply_calibration(&table);

        let corrected = correct_duty(0);
        let min_enable_duty = ((crate::state::config::MIN_ENABLE_RPM as u32 * 10000
            + CARVERA_SPINDLE_MAX_RPM / 2)
            / CARVERA_SPINDLE_MAX_RPM) as u16;

        assert!(
            corrected <= min_enable_duty,
            "correct_duty(0) = {} exceeds min_enable_duty = {} -- would falsely enable spindle on startup",
            corrected,
            min_enable_duty,
        );
    }

    #[test]
    #[serial]
    fn test_duty_to_calibrated_rpm_no_calibration() {
        let _guard = CalTestGuard::new();
        for duty in [0u16, 100, 500, 2447, 5000, 9786, 10000] {
            let expected = ((duty as u32 * CARVERA_SPINDLE_MAX_RPM + 5000) / 10000) as u32;
            assert_eq!(
                duty_to_calibrated_rpm(duty),
                expected,
                "duty={} expected={} got={}",
                duty,
                expected,
                duty_to_calibrated_rpm(duty)
            );
        }
    }

    #[test]
    #[serial]
    fn test_duty_to_calibrated_rpm_exact_at_calibration_point() {
        let _guard = CalTestGuard::new();
        let table = CalibrationTable {
            points: {
                let mut pts = [CalibrationPoint::default(); CAL_STEPS];
                pts[0] = CalibrationPoint {
                    expected_rpm: 4750,
                    measured_duty: 2300,
                };
                pts[1] = CalibrationPoint {
                    expected_rpm: 5000,
                    measured_duty: 2447,
                };
                pts[2] = CalibrationPoint {
                    expected_rpm: 5250,
                    measured_duty: 2590,
                };
                pts
            },
            count: 3,
        };
        apply_calibration(&table);

        assert_eq!(
            duty_to_calibrated_rpm(2447),
            5000,
            "Should return exact 5000 RPM at calibration point, not 5001"
        );
        assert_eq!(duty_to_calibrated_rpm(2300), 4750);
        assert_eq!(duty_to_calibrated_rpm(2590), 5250);
    }

    #[test]
    #[serial]
    fn test_duty_to_calibrated_rpm_regression_3500() {
        let _guard = CalTestGuard::new();
        let table = CalibrationTable {
            points: {
                let mut pts = [CalibrationPoint::default(); CAL_STEPS];
                pts[0] = CalibrationPoint {
                    expected_rpm: 3250,
                    measured_duty: 1590,
                };
                pts[1] = CalibrationPoint {
                    expected_rpm: 3500,
                    measured_duty: 1713,
                };
                pts[2] = CalibrationPoint {
                    expected_rpm: 3750,
                    measured_duty: 1835,
                };
                pts
            },
            count: 3,
        };
        apply_calibration(&table);

        assert_eq!(
            duty_to_calibrated_rpm(1713),
            3500,
            "Should return exact 3500 RPM at calibration point"
        );
    }

    #[test]
    #[serial]
    fn test_duty_to_calibrated_rpm_zero_duty_clamped() {
        let _guard = CalTestGuard::new();
        let table = CalibrationTable {
            points: {
                let mut pts = [CalibrationPoint::default(); CAL_STEPS];
                pts[0] = CalibrationPoint {
                    expected_rpm: CAL_START_RPM,
                    measured_duty: 400,
                };
                pts[1] = CalibrationPoint {
                    expected_rpm: 1000,
                    measured_duty: 550,
                };
                pts
            },
            count: 2,
        };
        apply_calibration(&table);

        let rpm = duty_to_calibrated_rpm(0);
        assert_eq!(
            rpm, CAL_START_RPM as u32,
            "duty=0 should clamp to first calibration point RPM ({}), got {}",
            CAL_START_RPM, rpm
        );
    }

    #[test]
    #[serial]
    fn test_interpolation_s2501_no_large_jump() {
        let _guard = CalTestGuard::new();
        let table = CalibrationTable {
            points: {
                let mut pts = [CalibrationPoint::default(); CAL_STEPS];
                pts[0] = CalibrationPoint {
                    expected_rpm: 2450,
                    measured_duty: ((2450u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 5,
                };
                pts[1] = CalibrationPoint {
                    expected_rpm: 2500,
                    measured_duty: ((2500u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 5,
                };
                pts[2] = CalibrationPoint {
                    expected_rpm: 2550,
                    measured_duty: ((2550u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 5,
                };
                pts
            },
            count: 3,
        };
        apply_calibration(&table);

        let duty_s2501 = ((2501u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 5;
        let rpm = duty_to_calibrated_rpm(duty_s2501);
        let error = (rpm as i32 - 2501).unsigned_abs();
        assert!(
            error <= 5,
            "S2501 should give ~2501 RPM, got {} (error={})",
            rpm,
            error
        );

        let duty_2500 = ((2500u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 5;
        assert_eq!(duty_to_calibrated_rpm(duty_2500), 2500);
    }

    #[test]
    #[serial]
    fn test_interpolation_degenerate_bracket() {
        let _guard = CalTestGuard::new();
        let table = CalibrationTable {
            points: {
                let mut pts = [CalibrationPoint::default(); CAL_STEPS];
                pts[0] = CalibrationPoint {
                    expected_rpm: 5000,
                    measured_duty: 2447,
                };
                pts[1] = CalibrationPoint {
                    expected_rpm: 5050,
                    measured_duty: 2447,
                };
                pts[2] = CalibrationPoint {
                    expected_rpm: 5100,
                    measured_duty: 2500,
                };
                pts
            },
            count: 3,
        };
        apply_calibration(&table);

        let rpm = duty_to_calibrated_rpm(2447);
        assert!(
            rpm == 5000 || rpm == 5050,
            "Degenerate bracket should return one of the bounding RPMs, got {}",
            rpm
        );
    }

    #[test]
    #[serial]
    fn test_interpolation_off_step_values() {
        let _guard = CalTestGuard::new();
        let offset: u16 = 20;
        let mut table = CalibrationTable::default();
        table.count = 10;
        for i in 0..10 {
            let rpm = CAL_START_RPM + i as u16 * CAL_STEP_RPM;
            let duty = ((rpm as u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + offset;
            table.points[i] = CalibrationPoint {
                expected_rpm: rpm,
                measured_duty: duty,
            };
        }
        apply_calibration(&table);

        for i in 0..9 {
            let rpm = CAL_START_RPM + i as u16 * CAL_STEP_RPM;
            let target_rpm = rpm as u32 + 1;
            let duty = ((target_rpm * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + offset;
            let result = duty_to_calibrated_rpm(duty);
            let error = (result as i32 - target_rpm as i32).unsigned_abs();
            assert!(
                error <= 10,
                "S{} should give ~{} RPM, got {} (error={})",
                target_rpm,
                target_rpm,
                result,
                error
            );
        }
    }

    #[test]
    #[serial]
    fn test_interpolate_rpm_at_last_calibration_point() {
        let _guard = CalTestGuard::new();
        let mut table = CalibrationTable::default();
        table.count = 5;
        for i in 0..5 {
            let rpm = CAL_START_RPM + i as u16 * CAL_STEP_RPM;
            let duty = ((rpm as u32 * 10000) / CARVERA_SPINDLE_MAX_RPM) as u16 + 20;
            table.points[i] = CalibrationPoint {
                expected_rpm: rpm,
                measured_duty: duty,
            };
        }
        apply_calibration(&table);

        let last_duty = table.points[4].measured_duty;
        let rpm = duty_to_calibrated_rpm(last_duty);
        let expected = CAL_START_RPM as u32 + 4 * CAL_STEP_RPM as u32;
        let error = (rpm as i32 - expected as i32).unsigned_abs();
        assert!(
            error <= 5,
            "At last cal point duty {}, expected ~{} RPM, got {} (error={})",
            last_duty,
            expected,
            rpm,
            error
        );
    }

    #[test]
    #[serial]
    fn test_correct_duty_large_duty_near_10000() {
        let _guard = CalTestGuard::new();
        let table = full_386_point_cal_table(30);
        apply_calibration(&table);

        let corrected = correct_duty(10000);
        let ideal_20000 =
            ((20000u32 * 10000 + CARVERA_SPINDLE_MAX_RPM / 2) / CARVERA_SPINDLE_MAX_RPM) as u16;
        assert!(
            corrected >= ideal_20000 - 50 && corrected <= 10000,
            "correct_duty(10000) = {}, expected near {} (ideal for 20000 RPM)",
            corrected,
            ideal_20000
        );
    }

    #[test]
    #[serial]
    fn test_clear_calibration() {
        let _guard = CalTestGuard::new();
        let mut table = CalibrationTable::default();
        table.count = 2;
        table.points[0] = CalibrationPoint {
            expected_rpm: 500,
            measured_duty: 245,
        };
        table.points[1] = CalibrationPoint {
            expected_rpm: 750,
            measured_duty: 367,
        };
        apply_calibration(&table);
        assert!(has_calibration());
        assert_eq!(calibration_count(), 2);

        clear_calibration();
        assert!(!has_calibration());
        assert_eq!(calibration_count(), 0);
    }

    #[test]
    #[serial]
    fn test_read_calibration_table_none_when_empty() {
        let _guard = CalTestGuard::new();
        assert!(read_calibration_table().is_none());
    }

    #[test]
    #[serial]
    fn test_read_calibration_table_roundtrip() {
        let _guard = CalTestGuard::new();
        let mut table = CalibrationTable::default();
        table.count = 3;
        table.points[0] = CalibrationPoint {
            expected_rpm: 750,
            measured_duty: 400,
        };
        table.points[1] = CalibrationPoint {
            expected_rpm: 800,
            measured_duty: 420,
        };
        table.points[2] = CalibrationPoint {
            expected_rpm: 850,
            measured_duty: 440,
        };
        apply_calibration(&table);

        let readback = read_calibration_table().expect("should have data");
        assert_eq!(readback.count, 3);
        assert_eq!(readback.points[0].expected_rpm, 750);
        assert_eq!(readback.points[0].measured_duty, 400);
        assert_eq!(readback.points[1].expected_rpm, 800);
        assert_eq!(readback.points[1].measured_duty, 420);
        assert_eq!(readback.points[2].expected_rpm, 850);
        assert_eq!(readback.points[2].measured_duty, 440);
    }
}

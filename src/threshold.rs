//! Threshold detection with debounce timing.
//!
//! This module provides generic threshold detection for values that need to exceed
//! a threshold for a minimum time before triggering. Useful for overcurrent detection,
//! thermal limits, or any condition that needs debounce timing.

/// Result of threshold detection check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdStatus {
    /// Value is below threshold
    Below,
    /// Value above threshold, within debounce period
    Rising,
    /// Value above threshold, debounce complete - triggered
    Triggered,
}

/// Detects when a value exceeds a threshold with debounce timing.
///
/// Useful for overcurrent detection, thermal limits, or any condition
/// that needs to persist for a minimum time before triggering.
///
/// # Examples
/// ```
/// use carvera_spindle::{ThresholdDetector, ThresholdStatus};
///
/// let mut detector = ThresholdDetector::new();
///
/// // Below threshold
/// assert_eq!(detector.check(50, 100, 0, 100), ThresholdStatus::Below);
///
/// // Above threshold, starts debounce
/// assert_eq!(detector.check(150, 100, 0, 100), ThresholdStatus::Rising);
///
/// // Still above, during debounce
/// assert_eq!(detector.check(150, 100, 50, 100), ThresholdStatus::Rising);
///
/// // Above threshold, debounce complete
/// assert_eq!(detector.check(150, 100, 150, 100), ThresholdStatus::Triggered);
/// ```
pub struct ThresholdDetector {
    /// Timestamp when value first exceeded threshold (ms), None if below
    start_time: Option<u64>,
}

impl ThresholdDetector {
    /// Create a new threshold detector.
    pub const fn new() -> Self {
        Self { start_time: None }
    }

    /// Check if value exceeds threshold with debounce timing.
    ///
    /// # Arguments
    /// * `value` - Current value to check
    /// * `threshold` - Threshold value (triggers when value > threshold)
    /// * `now_ms` - Current timestamp in milliseconds
    /// * `debounce_ms` - Time value must exceed threshold before triggering
    ///
    /// # Returns
    /// Current threshold status
    pub fn check(
        &mut self,
        value: u32,
        threshold: u32,
        now_ms: u64,
        debounce_ms: u64,
    ) -> ThresholdStatus {
        if value > threshold {
            match self.start_time {
                None => {
                    self.start_time = Some(now_ms);
                    ThresholdStatus::Rising
                }
                Some(start) if now_ms >= start + debounce_ms => ThresholdStatus::Triggered,
                Some(_) => ThresholdStatus::Rising,
            }
        } else {
            self.start_time = None;
            ThresholdStatus::Below
        }
    }

    /// Reset the detector, clearing any in-progress detection.
    pub fn reset(&mut self) {
        self.start_time = None;
    }

    /// Check if detection is currently in progress (above threshold).
    pub fn is_active(&self) -> bool {
        self.start_time.is_some()
    }
}

impl Default for ThresholdDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_below() {
        let mut detector = ThresholdDetector::new();
        let status = detector.check(50, 100, 0, 100);
        assert_eq!(status, ThresholdStatus::Below);
        assert!(!detector.is_active());
    }

    #[test]
    fn test_threshold_rising() {
        let mut detector = ThresholdDetector::new();
        let status = detector.check(150, 100, 0, 100);
        assert_eq!(status, ThresholdStatus::Rising);
        assert!(detector.is_active());
    }

    #[test]
    fn test_threshold_triggered() {
        let mut detector = ThresholdDetector::new();
        detector.check(150, 100, 0, 100); // Start debounce
        detector.check(150, 100, 50, 100); // Still rising
        let status = detector.check(150, 100, 150, 100); // Past debounce
        assert_eq!(status, ThresholdStatus::Triggered);
    }

    #[test]
    fn test_threshold_reset_on_drop() {
        let mut detector = ThresholdDetector::new();
        detector.check(150, 100, 0, 100); // Start debounce
        assert!(detector.is_active());

        // Drop below threshold
        let status = detector.check(50, 100, 50, 100);
        assert_eq!(status, ThresholdStatus::Below);
        assert!(!detector.is_active());
    }

    #[test]
    fn test_threshold_debounce_timing() {
        let mut detector = ThresholdDetector::new();

        // Start at t=1000
        let status = detector.check(150, 100, 1000, 200);
        assert_eq!(status, ThresholdStatus::Rising);

        // At t=1100 (100ms elapsed, debounce is 200ms)
        let status = detector.check(150, 100, 1100, 200);
        assert_eq!(status, ThresholdStatus::Rising);

        // At t=1199 (199ms elapsed, still not triggered)
        let status = detector.check(150, 100, 1199, 200);
        assert_eq!(status, ThresholdStatus::Rising);

        // At t=1200 (200ms elapsed, triggered)
        let status = detector.check(150, 100, 1200, 200);
        assert_eq!(status, ThresholdStatus::Triggered);
    }

    #[test]
    fn test_threshold_manual_reset() {
        let mut detector = ThresholdDetector::new();
        detector.check(150, 100, 0, 100);
        assert!(detector.is_active());

        detector.reset();
        assert!(!detector.is_active());
    }

    #[test]
    fn test_threshold_default() {
        let detector = ThresholdDetector::default();
        assert!(!detector.is_active());
    }

    #[test]
    fn test_threshold_stays_triggered_on_repeated_checks() {
        // After first Triggered, subsequent calls with value > threshold
        // should keep returning Triggered (not Rising or Below)
        let mut detector = ThresholdDetector::new();

        // Start debounce at t=0
        detector.check(150, 100, 0, 100);

        // Triggered at t=100
        let status = detector.check(150, 100, 100, 100);
        assert_eq!(status, ThresholdStatus::Triggered);

        // Continue feeding values above threshold - should stay Triggered
        let status = detector.check(200, 100, 200, 100);
        assert_eq!(
            status,
            ThresholdStatus::Triggered,
            "Should stay Triggered while value remains above threshold"
        );

        let status = detector.check(150, 100, 500, 100);
        assert_eq!(
            status,
            ThresholdStatus::Triggered,
            "Should stay Triggered at any later time while above threshold"
        );

        let status = detector.check(101, 100, 1000, 100);
        assert_eq!(
            status,
            ThresholdStatus::Triggered,
            "Should stay Triggered even with value barely above threshold"
        );
    }
}

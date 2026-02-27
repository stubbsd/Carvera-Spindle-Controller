//! Stabilization tracking for spindle speed.
//!
//! This module provides tracking for how long it takes the spindle to reach
//! target speed after a speed change command. Used for performance monitoring
//! and display feedback.

/// Status of stabilization tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StabilizationStatus {
    /// Spindle is idle (requested RPM = 0)
    Idle,
    /// Spindle is accelerating/decelerating toward target
    Accelerating,
    /// Spindle has reached target speed (within tolerance)
    Stabilized,
    /// Flash duration expired, returning to normal display
    Normal,
}

/// Tracks time from speed change to reaching ±2% of target.
///
/// Used to measure and display how long it takes for the spindle to reach
/// target speed after a speed change command.
///
/// In analog mode, Carvera sends target speed immediately (no incremental feedback loop),
/// so stabilization is reported as soon as actual RPM reaches tolerance.
pub struct StabilizationTracker {
    /// When target changed (ms), None if not tracking
    change_start_time: Option<u64>,
    /// RPM at start of change (for debug logging)
    start_rpm: u32,
    /// Target RPM we're tracking toward
    target_rpm: u32,
    /// Previous requested RPM (for change detection)
    last_requested_rpm: u32,
    /// Timestamp of last RPM reading for rate calculation (ms)
    last_rpm_time: u64,
    /// Most recent stabilization time result (ms)
    last_stabilization_time_ms: Option<u32>,
    /// When stabilization was detected (for flash duration), None if not flashing
    stabilization_detected_time: Option<u64>,
    /// Whether we've reported this stabilization event (for debug log)
    reported: bool,
    /// Whether we've received at least one valid reading
    initialized: bool,
}

impl StabilizationTracker {
    /// Tolerance percentage for considering speed "stabilized" (±2%)
    const TOLERANCE_PCT: u32 = 2;

    /// How long to show stabilization time on display (4 seconds)
    const FLASH_DURATION_MS: u64 = 4000;

    /// Minimum RPM change to consider significant (filters noise)
    const MIN_CHANGE_RPM: u32 = 100;

    /// Minimum rate of change to detect speed change (RPM/second)
    const MIN_RATE_THRESHOLD: u32 = 500;

    /// Create a new stabilization tracker.
    pub const fn new() -> Self {
        Self {
            change_start_time: None,
            start_rpm: 0,
            target_rpm: 0,
            last_requested_rpm: 0,
            last_rpm_time: 0,
            last_stabilization_time_ms: None,
            stabilization_detected_time: None,
            reported: false,
            initialized: false,
        }
    }

    /// Get debug info for logging (start_rpm, target_rpm)
    pub fn get_debug_info(&self) -> (u32, u32) {
        (self.start_rpm, self.target_rpm)
    }

    /// Check stabilization status and return optional time to display.
    ///
    /// # Arguments
    /// * `requested_rpm` - Current requested RPM
    /// * `actual_rpm` - Measured actual RPM
    /// * `now_ms` - Current timestamp in milliseconds
    ///
    /// # Returns
    /// Tuple of (status, optional time_ms to display)
    /// - time_ms is Some during the 2-second flash window after stabilization
    ///
    /// # Stabilization Logic
    /// In analog mode, Carvera sends the target speed immediately without incrementing,
    /// so stabilization is reported as soon as actual RPM reaches ±2% tolerance.
    pub fn check(
        &mut self,
        requested_rpm: u32,
        actual_rpm: u32,
        now_ms: u64,
    ) -> (StabilizationStatus, Option<u32>) {
        // Initialize on first call
        if !self.initialized {
            self.initialized = true;
            self.last_requested_rpm = requested_rpm;
            self.last_rpm_time = now_ms;
            return (StabilizationStatus::Idle, None);
        }

        // Spindle idle - reset tracking
        if requested_rpm == 0 {
            self.change_start_time = None;
            self.stabilization_detected_time = None;
            self.last_stabilization_time_ms = None;
            self.last_requested_rpm = 0;
            self.last_rpm_time = now_ms;
            self.reported = false;
            return (StabilizationStatus::Idle, None);
        }

        // Detect target changes
        if requested_rpm != self.last_requested_rpm {
            let rpm_delta = requested_rpm.abs_diff(self.last_requested_rpm);
            let time_delta_ms = now_ms.saturating_sub(self.last_rpm_time);
            let was_idle = self.last_requested_rpm == 0;
            let rate_per_sec = if time_delta_ms > 0 {
                (rpm_delta as u64 * 1000) / time_delta_ms
            } else {
                0
            };
            let substantial_change =
                rate_per_sec > Self::MIN_RATE_THRESHOLD as u64 && rpm_delta > Self::MIN_CHANGE_RPM;

            if substantial_change {
                // Start new tracking (or restart if target changed)
                self.start_rpm = if was_idle { 0 } else { actual_rpm };
                self.change_start_time = Some(now_ms);
                self.stabilization_detected_time = None;
                self.last_stabilization_time_ms = None;
                self.reported = false;
            }

            // Update target
            self.target_rpm = requested_rpm;
        }

        self.last_rpm_time = now_ms;
        self.last_requested_rpm = requested_rpm;

        // Not tracking anything
        if self.change_start_time.is_none() {
            // Check if we're within flash duration after stabilization
            if let Some(detected_time) = self.stabilization_detected_time {
                if now_ms < detected_time + Self::FLASH_DURATION_MS {
                    return (
                        StabilizationStatus::Stabilized,
                        self.last_stabilization_time_ms,
                    );
                } else {
                    self.stabilization_detected_time = None;
                    return (StabilizationStatus::Normal, None);
                }
            }
            return (StabilizationStatus::Normal, None);
        }

        // Check if we're within flash duration after stabilization
        if let Some(detected_time) = self.stabilization_detected_time {
            if now_ms < detected_time + Self::FLASH_DURATION_MS {
                return (
                    StabilizationStatus::Stabilized,
                    self.last_stabilization_time_ms,
                );
            } else {
                self.stabilization_detected_time = None;
                return (StabilizationStatus::Normal, None);
            }
        }

        // Check if actual is within tolerance of current target
        let tolerance = (self.target_rpm * Self::TOLERANCE_PCT) / 100;
        let lower_bound = self.target_rpm.saturating_sub(tolerance);
        let upper_bound = self.target_rpm.saturating_add(tolerance);
        let within_tolerance = actual_rpm >= lower_bound && actual_rpm <= upper_bound;

        // In analog mode, report stabilization immediately when within tolerance
        if within_tolerance {
            if let Some(start_time) = self.change_start_time {
                let elapsed_ms = now_ms.saturating_sub(start_time) as u32;
                self.last_stabilization_time_ms = Some(elapsed_ms);
                self.stabilization_detected_time = Some(now_ms);
                self.change_start_time = None;
                return (StabilizationStatus::Stabilized, Some(elapsed_ms));
            }
        }

        (StabilizationStatus::Accelerating, None)
    }

    /// Check if this stabilization event has been reported (for debug logging).
    pub fn is_reported(&self) -> bool {
        self.reported
    }

    /// Mark the current stabilization event as reported.
    pub fn mark_reported(&mut self) {
        self.reported = true;
    }
}

impl Default for StabilizationTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stabilization_idle_when_spindle_off() {
        let mut tracker = StabilizationTracker::new();

        // First call initializes
        let (status, time) = tracker.check(0, 0, 0);
        assert_eq!(status, StabilizationStatus::Idle);
        assert!(time.is_none());

        // Spindle remains off
        let (status, time) = tracker.check(0, 0, 1000);
        assert_eq!(status, StabilizationStatus::Idle);
        assert!(time.is_none());
    }

    #[test]
    fn test_stabilization_detects_speed_change() {
        let mut tracker = StabilizationTracker::new();

        // Initialize
        tracker.check(0, 0, 0);

        // Rapid speed change: 0 -> 5000 in 20ms (250000 RPM/s, well above 500 threshold)
        let (status, time) = tracker.check(5000, 0, 20);
        assert_eq!(status, StabilizationStatus::Accelerating);
        assert!(time.is_none());
    }

    #[test]
    fn test_stabilization_detects_within_tolerance() {
        let mut tracker = StabilizationTracker::new();

        // Initialize
        tracker.check(0, 0, 0);

        // Speed change: 0 -> 10000 (starts tracking)
        tracker.check(10000, 0, 20);

        // Still accelerating (actual is 0)
        let (status, _) = tracker.check(10000, 0, 100);
        assert_eq!(status, StabilizationStatus::Accelerating);

        // Actual reaches within 2% of target: 10000 * 0.98 = 9800
        let (status, time) = tracker.check(10000, 9800, 1500);
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert!(time.is_some());

        // Elapsed time should be ~1480ms (1500 - 20)
        let elapsed = time.unwrap();
        assert!(
            elapsed > 1400 && elapsed < 1600,
            "Expected ~1480ms, got {}",
            elapsed
        );
    }

    #[test]
    fn test_stabilization_flash_duration() {
        let mut tracker = StabilizationTracker::new();

        // Initialize and trigger speed change
        tracker.check(0, 0, 0);
        tracker.check(10000, 0, 20);

        // Stabilize at t=1500
        let (status, time) = tracker.check(10000, 10000, 1500);
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert!(time.is_some());
        let stabilization_time = time.unwrap();

        // During flash window (4 seconds), time should still be returned
        let (status, time) = tracker.check(10000, 10000, 4000);
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert_eq!(time, Some(stabilization_time));

        // At 5500ms (1500 + 4000 = flash window expired)
        let (status, time) = tracker.check(10000, 10000, 5500);
        assert_eq!(status, StabilizationStatus::Normal);
        assert!(time.is_none());
    }

    #[test]
    fn test_stabilization_reset_on_new_change() {
        let mut tracker = StabilizationTracker::new();

        // Initialize and complete first stabilization
        tracker.check(0, 0, 0);
        tracker.check(10000, 0, 20);
        tracker.check(10000, 10000, 1500); // Stabilized at 10000 RPM

        // New substantial speed change should reset tracking
        // 10000 -> 15000 in 20ms (250000 RPM/s)
        let (status, time) = tracker.check(15000, 10000, 1520);
        assert_eq!(status, StabilizationStatus::Accelerating);
        assert!(time.is_none());

        // Now stabilize at new target
        let (status, time) = tracker.check(15000, 14700, 3000); // Within 2%
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert!(time.is_some());

        // New elapsed time should be ~1480ms (3000 - 1520)
        let elapsed = time.unwrap();
        assert!(
            elapsed > 1400 && elapsed < 1600,
            "Expected ~1480ms, got {}",
            elapsed
        );
    }

    #[test]
    fn test_stabilization_tolerance_boundary() {
        let mut tracker = StabilizationTracker::new();

        // Initialize and start tracking
        tracker.check(0, 0, 0);
        tracker.check(10000, 0, 20);

        // Just outside 2% tolerance: 10000 * 0.98 - 1 = 9799
        let (status, time) = tracker.check(10000, 9799, 1000);
        assert_eq!(status, StabilizationStatus::Accelerating);
        assert!(time.is_none());

        // Exactly at 2% tolerance: 10000 * 0.98 = 9800
        let (status, time) = tracker.check(10000, 9800, 1500);
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert!(time.is_some());
    }

    #[test]
    fn test_stabilization_debug_info() {
        let mut tracker = StabilizationTracker::new();

        // Initialize
        tracker.check(0, 0, 0);

        // Standing start: from idle (requested=0) to target=10000
        // Even though actual is 500 by detection time, start_rpm should be 0
        // because we know the motor was at rest before the command
        tracker.check(10000, 500, 20);

        let (start_rpm, target_rpm) = tracker.get_debug_info();
        assert_eq!(
            start_rpm, 0,
            "Standing start should use 0 as start RPM, not actual at detection time"
        );
        assert_eq!(target_rpm, 10000, "Target RPM should be requested");

        // In analog mode, target changes restart tracking with new start_rpm
        tracker.check(15000, 10000, 40);
        let (start_rpm, target_rpm) = tracker.get_debug_info();
        assert_eq!(
            start_rpm, 10000,
            "Target change restarts tracking with actual RPM"
        );
        assert_eq!(
            target_rpm, 15000,
            "Target RPM should update to new requested"
        );

        // After stabilization, a new speed change while running uses actual RPM
        tracker.check(15000, 14800, 2000); // Hit tolerance
        // Now start a new tracking from running state
        tracker.check(20000, 15000, 2020);
        let (start_rpm, target_rpm) = tracker.get_debug_info();
        assert_eq!(
            start_rpm, 15000,
            "Speed change after stabilization should use actual RPM"
        );
        assert_eq!(target_rpm, 20000, "Target RPM should be new requested");
    }

    #[test]
    fn test_stabilization_report_flag() {
        let mut tracker = StabilizationTracker::new();

        // Initialize and stabilize
        tracker.check(0, 0, 0);
        tracker.check(10000, 0, 20);
        tracker.check(10000, 10000, 1500);

        // Should not be reported initially
        assert!(!tracker.is_reported());

        // Mark as reported
        tracker.mark_reported();
        assert!(tracker.is_reported());

        // New speed change should reset the flag
        tracker.check(15000, 10000, 1520);
        assert!(!tracker.is_reported());
    }

    #[test]
    fn test_stabilization_spindle_stop_resets_tracking() {
        let mut tracker = StabilizationTracker::new();

        // Initialize and start tracking
        tracker.check(0, 0, 0);
        tracker.check(10000, 0, 20);

        // Spindle stops (requested = 0)
        let (status, time) = tracker.check(0, 5000, 1000);
        assert_eq!(status, StabilizationStatus::Idle);
        assert!(time.is_none());
    }

    #[test]
    fn test_stabilization_reports_immediately_in_analog_mode() {
        let mut tracker = StabilizationTracker::new();
        tracker.check(0, 0, 0);

        // In analog mode, Carvera sends final target immediately
        tracker.check(8000, 0, 100); // Target 8000 RPM

        // Target stable at 8000, but actual not in tolerance yet
        let (status, _) = tracker.check(8000, 7000, 500);
        assert_eq!(status, StabilizationStatus::Accelerating);

        // Still accelerating
        let (status, _) = tracker.check(8000, 7500, 800);
        assert_eq!(status, StabilizationStatus::Accelerating);

        // Actual within 2% - report immediately (no delay in analog mode)
        let (status, time) = tracker.check(8000, 7900, 1000);
        assert_eq!(status, StabilizationStatus::Stabilized);
        assert!(time.is_some());

        // Time should be 1000 - 100 = 900ms
        let elapsed = time.unwrap();
        assert!(
            elapsed >= 800 && elapsed <= 1000,
            "Expected ~900ms, got {}",
            elapsed
        );

        // Subsequent calls show flash
        let (status2, _) = tracker.check(8000, 7920, 1100);
        assert_eq!(status2, StabilizationStatus::Stabilized);
    }

    #[test]
    fn test_stabilization_reports_time_when_tolerance_hit() {
        let mut tracker = StabilizationTracker::new();
        tracker.check(0, 0, 0);

        // Scenario: Single target, report time when we HIT tolerance
        // Timeline:
        // t=100: tracking starts
        // t=500-4000: accelerating, not in tolerance yet
        // t=5000: actual hits tolerance

        tracker.check(10000, 0, 100); // tracking starts

        // Accelerate - not in tolerance yet
        tracker.check(10000, 5000, 500);
        tracker.check(10000, 8000, 2000);

        // At t=5000: actual hits tolerance (9800 is within ±2% of 10000)
        // In analog mode, report immediately
        let (status, time) = tracker.check(10000, 9800, 5000);
        assert_eq!(status, StabilizationStatus::Stabilized);

        let elapsed = time.unwrap();
        // Should report 5000 - 100 = 4900ms
        assert!(
            elapsed >= 4800 && elapsed <= 5000,
            "Expected ~4900ms, got {}ms",
            elapsed
        );
    }

    #[test]
    fn test_stabilization_target_change_restarts_tracking() {
        let mut tracker = StabilizationTracker::new();
        tracker.check(0, 0, 0);

        // Scenario: Target changes, tracking restarts for NEW target
        // Timeline:
        // t=100: start tracking at 5000 RPM
        // t=1000: would hit tolerance for 5000, but reports immediately
        // t=2100: target changes to 8000 - tracking restarts
        // t=3200: hit tolerance for 8000 (7840 within ±2%)
        // Should report: 3200 - 2100 = 1100ms (from when NEW tracking started)

        tracker.check(5000, 0, 100); // start tracking

        // Hit tolerance for 5000 at t=1000 - reports immediately in analog mode
        let (status, time) = tracker.check(5000, 4900, 1000);
        assert_eq!(status, StabilizationStatus::Stabilized);
        let elapsed = time.unwrap();
        assert!(
            elapsed >= 800 && elapsed <= 1000,
            "Expected ~900ms, got {}",
            elapsed
        );

        // Target changes at t=2100 - tracking restarts
        tracker.check(8000, 5000, 2100);

        // At t=3200, actual is within tolerance - reports immediately
        let (status, time) = tracker.check(8000, 7840, 3200);
        assert_eq!(status, StabilizationStatus::Stabilized);

        let elapsed = time.unwrap();
        // Should report 3200 - 2100 = 1100ms (from new tracking start)
        assert!(
            elapsed >= 1000 && elapsed <= 1200,
            "Expected ~1100ms (from new tracking start), got {}ms",
            elapsed
        );
    }

    #[test]
    fn test_stabilization_multiple_target_changes() {
        let mut tracker = StabilizationTracker::new();
        tracker.check(0, 0, 0);

        // Scenario: Multiple target changes (each restarts tracking in analog mode)
        // Timeline:
        // t=100: command 2000 (start tracking)
        // t=1100: command 4000 (tracking restarts)
        // t=2100: command 6000 (tracking restarts)
        // t=3100: command 8000 (final target, tracking restarts)
        // t=3500: actual hits tolerance for 8000 - reports immediately
        // Time: 3500 - 3100 = 400ms (from last tracking start)

        tracker.check(2000, 0, 100);
        tracker.check(4000, 2000, 1100);
        tracker.check(6000, 4000, 2100);
        tracker.check(8000, 6000, 3100); // final target

        // Hit tolerance at t=3500 - reports immediately in analog mode
        let (status, time) = tracker.check(8000, 7850, 3500);
        assert_eq!(status, StabilizationStatus::Stabilized);

        let elapsed = time.unwrap();
        // Time from last tracking start (3100) to tolerance hit (3500) = 400ms
        assert!(
            elapsed >= 300 && elapsed <= 500,
            "Expected ~400ms, got {}ms",
            elapsed
        );
    }

    #[test]
    fn test_stabilization_small_change_below_min_change_rpm() {
        // Changes below MIN_CHANGE_RPM (100) should NOT start tracking
        let mut tracker = StabilizationTracker::new();

        // Initialize at 5000 RPM
        tracker.check(0, 0, 0);
        tracker.check(5000, 0, 20); // Start initial tracking

        // Stabilize at 5000 RPM
        tracker.check(5000, 5000, 1500);

        // Wait for flash to expire
        tracker.check(5000, 5000, 6000);

        // Small change: 5000 -> 5050 (50 RPM delta < 100 MIN_CHANGE_RPM)
        // Even with high rate (50/20ms = 2500 RPM/s), rpm_delta <= 100 so NOT substantial
        let (status, time) = tracker.check(5050, 5000, 6020);
        // Should remain Normal (no new tracking started)
        assert_eq!(
            status,
            StabilizationStatus::Normal,
            "Small RPM change below MIN_CHANGE_RPM should not start tracking"
        );
        assert!(time.is_none());
    }

    #[test]
    fn test_stabilization_reports_first_tolerance_hit() {
        let mut tracker = StabilizationTracker::new();
        tracker.check(0, 0, 0);

        tracker.check(10000, 0, 100);

        // Hit tolerance at t=1000 - reports immediately in analog mode
        let (status, time) = tracker.check(10000, 9800, 1000);
        assert_eq!(status, StabilizationStatus::Stabilized);

        let elapsed = time.unwrap();
        // Should report 1000-100=900ms
        assert!(
            elapsed >= 800 && elapsed <= 1000,
            "Expected ~900ms, got {}ms",
            elapsed
        );

        // Subsequent readings continue flash duration
        let (status, _) = tracker.check(10000, 9900, 1500);
        assert_eq!(status, StabilizationStatus::Stabilized);
    }
}

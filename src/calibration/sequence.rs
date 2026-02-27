//! Sequence detection for calibration start patterns.
//!
//! Detects 3-note zigzag patterns (e.g., 6000 -> 12000 -> 9000 RPM) to trigger
//! calibration, clear, or dump commands.

use super::{
    SEQ_NOTE_MAX_MS, SEQ_NOTE_MIN_MS, SEQ_TRANSITION_GRACE_MS, duty_matches_speed, is_off,
};

/// States for the sequence detector state machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SeqState {
    #[default]
    Idle,
    NoteAOn,
    NoteBOn,
    NoteCOn,
}

/// Detects a 3-note zigzag sequence with configurable RPM values:
///   notes[0] -> notes[1] -> notes[2] -> OFF
///
/// Speed changes happen directly (no M5 stops between notes). The final
/// OFF after note C completes detection. Zigzag patterns prevent false
/// triggers during normal machining ramps.
///
/// Default (calibrate): 6000 -> 12000 -> 9000 RPM
/// Clear: 12000 -> 6000 -> 9000 RPM
/// Dump: 9000 -> 6000 -> 12000 RPM
///
/// Timing: notes 200ms-12s. +/-12% on speed matching.
pub struct SequenceDetector {
    state: SeqState,
    entered_ms: u64,
    mismatch_since_ms: u64, // 0 = no mismatch active
    notes: [u16; 3],
}

impl Default for SequenceDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceDetector {
    pub fn new() -> Self {
        Self::new_with_notes([6000, 12000, 9000])
    }

    pub fn new_with_notes(notes: [u16; 3]) -> Self {
        Self {
            state: SeqState::Idle,
            entered_ms: 0,
            mismatch_since_ms: 0,
            notes,
        }
    }

    /// Feed a duty reading. Returns `true` when the full sequence is detected.
    pub fn update(&mut self, duty: u16, now_ms: u64) -> bool {
        match self.state {
            SeqState::Idle => {
                if duty_matches_speed(duty, self.notes[0]) {
                    self.state = SeqState::NoteAOn;
                    self.entered_ms = now_ms;
                }
            }
            SeqState::NoteAOn => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if elapsed > SEQ_NOTE_MAX_MS {
                    self.reset();
                } else if elapsed >= SEQ_NOTE_MIN_MS && duty_matches_speed(duty, self.notes[1]) {
                    self.state = SeqState::NoteBOn;
                    self.entered_ms = now_ms;
                    self.mismatch_since_ms = 0;
                } else if duty_matches_speed(duty, self.notes[0])
                    || duty_matches_speed(duty, self.notes[1])
                {
                    self.mismatch_since_ms = 0;
                } else {
                    if self.mismatch_since_ms == 0 {
                        self.mismatch_since_ms = now_ms;
                    }
                    if now_ms.saturating_sub(self.mismatch_since_ms) > SEQ_TRANSITION_GRACE_MS {
                        self.reset();
                    }
                }
            }
            SeqState::NoteBOn => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if elapsed > SEQ_NOTE_MAX_MS {
                    self.reset();
                } else if elapsed >= SEQ_NOTE_MIN_MS && duty_matches_speed(duty, self.notes[2]) {
                    self.state = SeqState::NoteCOn;
                    self.entered_ms = now_ms;
                    self.mismatch_since_ms = 0;
                } else if duty_matches_speed(duty, self.notes[1])
                    || duty_matches_speed(duty, self.notes[2])
                {
                    self.mismatch_since_ms = 0;
                } else {
                    if self.mismatch_since_ms == 0 {
                        self.mismatch_since_ms = now_ms;
                    }
                    if now_ms.saturating_sub(self.mismatch_since_ms) > SEQ_TRANSITION_GRACE_MS {
                        self.reset();
                    }
                }
            }
            SeqState::NoteCOn => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if elapsed > SEQ_NOTE_MAX_MS {
                    self.reset();
                } else if elapsed >= SEQ_NOTE_MIN_MS && is_off(duty) {
                    self.reset();
                    return true;
                } else if duty_matches_speed(duty, self.notes[2]) || is_off(duty) {
                    self.mismatch_since_ms = 0;
                } else {
                    if self.mismatch_since_ms == 0 {
                        self.mismatch_since_ms = now_ms;
                    }
                    if now_ms.saturating_sub(self.mismatch_since_ms) > SEQ_TRANSITION_GRACE_MS {
                        self.reset();
                    }
                }
            }
        }
        false
    }

    /// Returns `true` when the detector is partway through matching the sequence
    /// (i.e., past the Idle state). Used to suppress stall detection during
    /// the calibration musical pattern.
    pub fn is_active(&self) -> bool {
        self.state != SeqState::Idle
    }

    fn reset(&mut self) {
        self.state = SeqState::Idle;
        self.entered_ms = 0;
        self.mismatch_since_ms = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calibration::{SEQUENCE_TOLERANCE_PCT, rpm_to_expected_duty};

    /// Helper: run a full speed-change sequence (noteA -> noteB -> noteC -> OFF).
    /// Each note held for `hold_ms` milliseconds at 50Hz (20ms ticks).
    fn run_speed_change_sequence(
        det: &mut SequenceDetector,
        notes: [u16; 3],
        hold_ms: u64,
    ) -> bool {
        let duties: [u16; 3] = [
            rpm_to_expected_duty(notes[0]),
            rpm_to_expected_duty(notes[1]),
            rpm_to_expected_duty(notes[2]),
        ];
        let ticks = (hold_ms / 20) as usize;
        let mut t: u64 = 0;

        // Note A
        for _ in 0..ticks {
            if det.update(duties[0], t) {
                return true;
            }
            t += 20;
        }
        // Note B (speed change)
        for _ in 0..ticks {
            if det.update(duties[1], t) {
                return true;
            }
            t += 20;
        }
        // Note C (speed change)
        for _ in 0..ticks {
            if det.update(duties[2], t) {
                return true;
            }
            t += 20;
        }
        // OFF -> should trigger detection
        det.update(0, t)
    }

    #[test]
    fn test_sequence_detector_full_sequence() {
        let mut det = SequenceDetector::new();
        assert!(run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            1000
        ));
    }

    #[test]
    fn test_sequence_detector_wrong_order_resets() {
        let mut det = SequenceDetector::new();
        let d12 = rpm_to_expected_duty(12000);

        let mut t: u64 = 0;
        for _ in 0..200 {
            assert!(!det.update(d12, t));
            t += 20;
        }
    }

    #[test]
    fn test_sequence_detector_too_short_note() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d12 = rpm_to_expected_duty(12000);

        let mut t: u64 = 0;
        for _ in 0..5 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        assert!(!det.update(d12, t));
    }

    #[test]
    fn test_sequence_detector_note_b_too_short() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d12 = rpm_to_expected_duty(12000);
        let d9 = rpm_to_expected_duty(9000);

        let mut t: u64 = 0;

        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        for _ in 0..5 {
            assert!(!det.update(d12, t));
            t += 20;
        }
        assert!(!det.update(d9, t));
    }

    #[test]
    fn test_sequence_detector_note_too_long() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);

        let mut t: u64 = 0;
        for _ in 0..650 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        let d12 = rpm_to_expected_duty(12000);
        assert!(!det.update(d12, t));
    }

    #[test]
    fn test_sequence_detector_wrong_speed_after_note_a() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d9 = rpm_to_expected_duty(9000);

        let mut t: u64 = 0;
        for _ in 0..50 {
            det.update(d6, t);
            t += 20;
        }
        assert!(!det.update(d9, t));
    }

    #[test]
    fn test_sequence_detector_resets_cleanly_for_retry() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d12 = rpm_to_expected_duty(12000);
        let d9 = rpm_to_expected_duty(9000);

        let mut t: u64 = 0;

        for _ in 0..50 {
            det.update(d6, t);
            t += 20;
        }
        let d3 = rpm_to_expected_duty(3000);
        det.update(d3, t);
        t += 20;

        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        for _ in 0..50 {
            assert!(!det.update(d12, t));
            t += 20;
        }
        for _ in 0..49 {
            assert!(!det.update(d9, t));
            t += 20;
        }
        assert!(det.update(0, t));
    }

    #[test]
    fn test_sequence_detector_real_world_timing() {
        let mut det = SequenceDetector::new();
        assert!(run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            1000
        ));
    }

    #[test]
    fn test_sequence_detector_long_notes() {
        let mut det = SequenceDetector::new();
        assert!(run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            10000
        ));
    }

    #[test]
    fn test_sequence_detector_minimum_note_duration() {
        let mut det = SequenceDetector::new();
        assert!(run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            200
        ));
    }

    #[test]
    fn test_sequence_detector_still_rejects_very_short_note() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);

        let mut t: u64 = 0;
        for _ in 0..8 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        let d3 = rpm_to_expected_duty(3000);
        for _ in 0..7 {
            det.update(d3, t);
            t += 20;
        }

        assert!(!det.is_active());
    }

    #[test]
    fn test_sequence_detector_rejects_note_exceeding_max() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d12 = rpm_to_expected_duty(12000);

        let mut t: u64 = 0;
        for _ in 0..625 {
            det.update(d6, t);
            t += 20;
        }
        assert!(!det.update(d12, t));
    }

    #[test]
    fn test_sequence_detector_normal_operation_no_false_positive() {
        let mut det = SequenceDetector::new();

        let mut t: u64 = 0;

        let d8 = rpm_to_expected_duty(8000);
        for _ in 0..500 {
            assert!(!det.update(d8, t));
            t += 20;
        }

        let d15 = rpm_to_expected_duty(15000);
        for _ in 0..250 {
            assert!(!det.update(d15, t));
            t += 20;
        }

        let d3 = rpm_to_expected_duty(3000);
        for _ in 0..250 {
            assert!(!det.update(d3, t));
            t += 20;
        }

        assert!(!det.update(0, t));
    }

    #[test]
    fn test_sequence_detector_monotonic_ramp_no_false_positive() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d9 = rpm_to_expected_duty(9000);
        let d12 = rpm_to_expected_duty(12000);

        let mut t: u64 = 0;
        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        for _ in 0..50 {
            assert!(!det.update(d9, t));
            t += 20;
        }
        for _ in 0..50 {
            assert!(!det.update(d12, t));
            t += 20;
        }
        assert!(!det.update(0, t));
    }

    #[test]
    fn test_sequence_detector_descending_ramp_no_false_positive() {
        let mut det = SequenceDetector::new();
        let d12 = rpm_to_expected_duty(12000);
        let d9 = rpm_to_expected_duty(9000);
        let d6 = rpm_to_expected_duty(6000);

        let mut t: u64 = 0;
        for _ in 0..50 {
            assert!(!det.update(d12, t));
            t += 20;
        }
        for _ in 0..50 {
            assert!(!det.update(d9, t));
            t += 20;
        }
        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }
        assert!(!det.update(0, t));
    }

    #[test]
    fn test_sequence_detector_default() {
        let det = SequenceDetector::default();
        let mut det = det;
        assert!(!det.update(0, 0));
    }

    #[test]
    fn test_sequence_detector_transition_grace_period() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);
        let d12 = rpm_to_expected_duty(12000);
        let d9 = rpm_to_expected_duty(9000);

        let d6_high = d6 + (d6 as u32 * SEQUENCE_TOLERANCE_PCT / 100) as u16;
        let d12_low = d12 - (d12 as u32 * SEQUENCE_TOLERANCE_PCT / 100) as u16;
        let intermediate = (d6_high + d12_low) / 2;
        assert!(
            !duty_matches_speed(intermediate, 6000),
            "intermediate {} should not match 6000",
            intermediate
        );
        assert!(
            !duty_matches_speed(intermediate, 12000),
            "intermediate {} should not match 12000",
            intermediate
        );

        let mut t: u64 = 0;

        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }

        for _ in 0..3 {
            assert!(!det.update(intermediate, t));
            t += 20;
        }

        for _ in 0..50 {
            assert!(!det.update(d12, t));
            t += 20;
        }

        for _ in 0..49 {
            assert!(!det.update(d9, t));
            t += 20;
        }

        assert!(det.update(0, t));
    }

    #[test]
    fn test_sequence_detector_prolonged_mismatch_resets() {
        let mut det = SequenceDetector::new();
        let d6 = rpm_to_expected_duty(6000);

        let d6_high = d6 + (d6 as u32 * SEQUENCE_TOLERANCE_PCT / 100) as u16;
        let d12 = rpm_to_expected_duty(12000);
        let d12_low = d12 - (d12 as u32 * SEQUENCE_TOLERANCE_PCT / 100) as u16;
        let intermediate = (d6_high + d12_low) / 2;

        let mut t: u64 = 0;

        for _ in 0..50 {
            assert!(!det.update(d6, t));
            t += 20;
        }

        for _ in 0..7 {
            det.update(intermediate, t);
            t += 20;
        }

        assert!(!det.is_active());
    }

    // --- Clear sequence detector tests ---

    #[test]
    fn test_clear_detector_full_sequence() {
        let mut det = SequenceDetector::new_with_notes([12000, 6000, 9000]);
        assert!(run_speed_change_sequence(
            &mut det,
            [12000, 6000, 9000],
            1000
        ));
    }

    // --- Dump sequence detector tests ---

    #[test]
    fn test_dump_detector_full_sequence() {
        let mut det = SequenceDetector::new_with_notes([9000, 6000, 12000]);
        assert!(run_speed_change_sequence(
            &mut det,
            [9000, 6000, 12000],
            1000
        ));
    }

    // --- Cross-detection tests (all 6 combinations) ---

    #[test]
    fn test_cal_does_not_trigger_clear() {
        let mut det = SequenceDetector::new_with_notes([12000, 6000, 9000]);
        assert!(!run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            1000
        ));
    }

    #[test]
    fn test_cal_does_not_trigger_dump() {
        let mut det = SequenceDetector::new_with_notes([9000, 6000, 12000]);
        assert!(!run_speed_change_sequence(
            &mut det,
            [6000, 12000, 9000],
            1000
        ));
    }

    #[test]
    fn test_clear_does_not_trigger_cal() {
        let mut det = SequenceDetector::new();
        assert!(!run_speed_change_sequence(
            &mut det,
            [12000, 6000, 9000],
            1000
        ));
    }

    #[test]
    fn test_clear_does_not_trigger_dump() {
        let mut det = SequenceDetector::new_with_notes([9000, 6000, 12000]);
        assert!(!run_speed_change_sequence(
            &mut det,
            [12000, 6000, 9000],
            1000
        ));
    }

    #[test]
    fn test_dump_does_not_trigger_cal() {
        let mut det = SequenceDetector::new();
        assert!(!run_speed_change_sequence(
            &mut det,
            [9000, 6000, 12000],
            1000
        ));
    }

    #[test]
    fn test_dump_does_not_trigger_clear() {
        let mut det = SequenceDetector::new_with_notes([12000, 6000, 9000]);
        assert!(!run_speed_change_sequence(
            &mut det,
            [9000, 6000, 12000],
            1000
        ));
    }
}

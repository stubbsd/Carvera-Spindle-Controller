//! Step-by-step calibration recording state machine.
//!
//! Records 386 calibration steps sequentially, with support for both
//! OFF-gap-based and speed-change-based step advancement.

use super::{
    ANNOUNCE_MS, CAL_START_RPM, CAL_STEP_RPM, CAL_STEPS, CalibrationPoint, CalibrationTable,
    OFF_DEBOUNCE_MS, SIGNAL_TIMEOUT_MS, SPEED_CHANGE_SETTLE_MS, SPEED_CHANGE_THRESHOLD,
    STEP_RECORD_MS, STEP_SETTLE_MS, is_off, is_on,
};

/// Events emitted by the recorder.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum CalEvent {
    /// No event -- still processing
    None,
    /// A step was recorded (index, expected_rpm, measured_duty)
    StepRecorded(u16, u16, u16),
    /// All steps complete
    Complete(CalibrationTable),
    /// Aborted due to signal loss
    Aborted,
}

/// Internal recorder states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RecState {
    /// Waiting for announce phase to finish
    #[default]
    Announce,
    /// Waiting for step signal to go ON
    WaitForStepOn,
    /// Signal ON, settling before recording
    StepSettling,
    /// Recording duty samples
    StepRecording,
    /// Waiting for the next step (either OFF gap or speed-change detection)
    WaitForNextStep,
}

/// Records 386 calibration steps sequentially.
pub struct CalibrationRecorder {
    state: RecState,
    step_index: u16,
    entered_ms: u64,
    duty_sum: u64,
    sample_count: u32,
    off_since_ms: u64,
    last_recorded_duty: u16,
    from_speed_change: bool,
    table: CalibrationTable,
}

impl CalibrationRecorder {
    pub fn new(start_ms: u64) -> Self {
        Self {
            state: RecState::Announce,
            step_index: 0,
            entered_ms: start_ms,
            duty_sum: 0,
            sample_count: 0,
            off_since_ms: 0,
            last_recorded_duty: 0,
            from_speed_change: false,
            table: CalibrationTable::default(),
        }
    }

    /// Current step index (0-based).
    pub fn step_index(&self) -> u16 {
        self.step_index
    }

    /// Whether we are in the recording sub-phase of a step.
    pub fn is_recording(&self) -> bool {
        self.state == RecState::StepRecording
    }

    /// Expected RPM for the current step.
    pub fn current_expected_rpm(&self) -> u16 {
        CAL_START_RPM + self.step_index * CAL_STEP_RPM
    }

    /// Feed a duty reading. Returns a `CalEvent`.
    pub fn update(&mut self, duty: u16, now_ms: u64) -> CalEvent {
        match self.state {
            RecState::Announce => {
                if now_ms.saturating_sub(self.entered_ms) >= ANNOUNCE_MS {
                    self.state = RecState::WaitForStepOn;
                    self.entered_ms = now_ms;
                }
                CalEvent::None
            }
            RecState::WaitForStepOn => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if is_on(duty) {
                    self.state = RecState::StepSettling;
                    self.entered_ms = now_ms;
                    self.duty_sum = 0;
                    self.sample_count = 0;
                    self.from_speed_change = false;
                    CalEvent::None
                } else if elapsed > SIGNAL_TIMEOUT_MS {
                    CalEvent::Aborted
                } else {
                    CalEvent::None
                }
            }
            RecState::StepSettling => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if is_off(duty) {
                    // Signal dropped during settling -- abort or reset
                    self.state = RecState::WaitForStepOn;
                    self.entered_ms = now_ms;
                    return CalEvent::None;
                }
                let settle_ms = if self.from_speed_change {
                    SPEED_CHANGE_SETTLE_MS
                } else {
                    STEP_SETTLE_MS
                };
                if elapsed >= settle_ms {
                    self.state = RecState::StepRecording;
                    self.entered_ms = now_ms;
                    self.duty_sum = 0;
                    self.sample_count = 0;
                }
                CalEvent::None
            }
            RecState::StepRecording => {
                let elapsed = now_ms.saturating_sub(self.entered_ms);
                if is_off(duty) {
                    // Signal dropped during recording -- use what we have if enough samples
                    if self.sample_count >= 3 {
                        return self.finish_step();
                    }
                    // Not enough samples, go back to waiting
                    self.state = RecState::WaitForStepOn;
                    self.entered_ms = now_ms;
                    return CalEvent::None;
                }
                // Accumulate
                self.duty_sum += duty as u64;
                self.sample_count += 1;
                if elapsed >= STEP_RECORD_MS {
                    self.finish_step()
                } else {
                    CalEvent::None
                }
            }
            RecState::WaitForNextStep => {
                // Check for completion first
                if self.step_index >= CAL_STEPS as u16 {
                    return CalEvent::Complete(self.table.clone());
                }

                if is_off(duty) {
                    // OFF path: backwards compatible with old G-code that uses M5 gaps
                    if self.off_since_ms == 0 {
                        self.off_since_ms = now_ms;
                    }
                    let off_elapsed = now_ms.saturating_sub(self.off_since_ms);
                    if off_elapsed >= OFF_DEBOUNCE_MS {
                        self.state = RecState::WaitForStepOn;
                        self.entered_ms = now_ms;
                        self.off_since_ms = 0;
                    }
                } else {
                    self.off_since_ms = 0;

                    // Speed-change fast path: detect duty shift without requiring OFF
                    let diff = duty.abs_diff(self.last_recorded_duty);
                    if diff > SPEED_CHANGE_THRESHOLD {
                        self.state = RecState::StepSettling;
                        self.entered_ms = now_ms;
                        self.duty_sum = 0;
                        self.sample_count = 0;
                        self.from_speed_change = true;
                    }
                }
                CalEvent::None
            }
        }
    }

    fn finish_step(&mut self) -> CalEvent {
        let avg_duty = if self.sample_count > 0 {
            (self.duty_sum / self.sample_count as u64) as u16
        } else {
            0
        };
        let expected_rpm = self.current_expected_rpm();
        let idx = self.step_index as usize;
        if idx < CAL_STEPS {
            self.table.points[idx] = CalibrationPoint {
                expected_rpm,
                measured_duty: avg_duty,
            };
            self.table.count = (idx + 1) as u16;
        }
        let event = CalEvent::StepRecorded(self.step_index, expected_rpm, avg_duty);
        self.last_recorded_duty = avg_duty;
        self.step_index += 1;
        self.off_since_ms = 0;

        self.state = RecState::WaitForNextStep;
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a recorder through announce+ON+settle+record to complete the first step.
    /// Returns the time after the step is recorded.
    fn record_first_step(rec: &mut CalibrationRecorder, duty: u16) -> u64 {
        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);
        t += 1;
        rec.update(duty, t);
        t += STEP_SETTLE_MS + 1;
        rec.update(duty, t);
        for _ in 0..500 {
            t += 1;
            if matches!(rec.update(duty, t), CalEvent::StepRecorded(..)) {
                return t;
            }
        }
        panic!("record_first_step: step did not complete within 500 iterations");
    }

    #[test]
    fn test_recorder_announce_phase() {
        let mut rec = CalibrationRecorder::new(0);
        assert_eq!(rec.update(0, 100), CalEvent::None);
        assert_eq!(rec.update(0, 400), CalEvent::None);
        assert_eq!(rec.update(0, 501), CalEvent::None);
    }

    #[test]
    fn test_recorder_single_step() {
        let mut rec = CalibrationRecorder::new(0);

        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        t += 1;
        rec.update(500, t);
        assert!(!rec.is_recording());

        t += STEP_SETTLE_MS + 1;
        rec.update(500, t);
        assert!(rec.is_recording());

        let mut got_step = false;
        for _ in 0..500 {
            t += 1;
            let evt = rec.update(500, t);
            if let CalEvent::StepRecorded(idx, rpm, duty) = evt {
                assert_eq!(idx, 0);
                assert_eq!(rpm, CAL_START_RPM);
                assert_eq!(duty, 500);
                got_step = true;
                break;
            }
        }
        assert!(
            got_step,
            "Should have recorded step within recording window"
        );
    }

    #[test]
    fn test_recorder_abort_on_timeout() {
        let mut rec = CalibrationRecorder::new(0);

        let t = ANNOUNCE_MS + 1;
        rec.update(0, t);

        let evt = rec.update(0, t + SIGNAL_TIMEOUT_MS + 1);
        assert_eq!(evt, CalEvent::Aborted);
    }

    #[test]
    fn test_recorder_speed_change_detection_no_off() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t = record_first_step(&mut rec, 500);

        let new_duty: u16 = 530;
        t += 1;
        rec.update(new_duty, t);

        t += SPEED_CHANGE_SETTLE_MS + 1;
        rec.update(new_duty, t);

        let mut step1_done = false;
        for _ in 0..500 {
            t += 1;
            if let CalEvent::StepRecorded(idx, rpm, duty) = rec.update(new_duty, t) {
                assert_eq!(idx, 1);
                assert_eq!(rpm, CAL_START_RPM + CAL_STEP_RPM);
                assert_eq!(duty, new_duty);
                step1_done = true;
                break;
            }
        }
        assert!(
            step1_done,
            "Step 1 should have completed via speed-change path"
        );
    }

    #[test]
    fn test_recorder_speed_change_below_threshold_ignored() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t = record_first_step(&mut rec, 500);

        for _ in 0..100 {
            t += 1;
            let evt = rec.update(510, t);
            assert_eq!(
                evt,
                CalEvent::None,
                "Small duty change should not advance step"
            );
        }

        assert_eq!(rec.step_index(), 1);
    }

    #[test]
    fn test_recorder_backwards_compat_off_path() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t = record_first_step(&mut rec, 500);

        t += 1;
        rec.update(0, t);
        t += OFF_DEBOUNCE_MS + 1;
        rec.update(0, t);

        t += 1;
        rec.update(600, t);
        t += STEP_SETTLE_MS + 1;
        rec.update(600, t);
        let mut step1_done = false;
        for _ in 0..500 {
            t += 1;
            if let CalEvent::StepRecorded(idx, _rpm, duty) = rec.update(600, t) {
                assert_eq!(idx, 1);
                assert_eq!(duty, 600);
                step1_done = true;
                break;
            }
        }
        assert!(step1_done, "Step 1 should complete via OFF path");
    }

    #[test]
    fn test_recorder_mixed_off_and_speed_change() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t = record_first_step(&mut rec, 400);

        t += 1;
        rec.update(430, t);
        t += SPEED_CHANGE_SETTLE_MS + 1;
        rec.update(430, t);
        for _ in 0..500 {
            t += 1;
            if matches!(rec.update(430, t), CalEvent::StepRecorded(..)) {
                break;
            }
        }

        t += 1;
        rec.update(0, t);
        t += OFF_DEBOUNCE_MS + 1;
        rec.update(0, t);

        t += 1;
        rec.update(460, t);
        t += STEP_SETTLE_MS + 1;
        rec.update(460, t);
        let mut step2_done = false;
        for _ in 0..500 {
            t += 1;
            if let CalEvent::StepRecorded(idx, _, _) = rec.update(460, t) {
                assert_eq!(idx, 2);
                step2_done = true;
                break;
            }
        }
        assert!(
            step2_done,
            "Step 2 should complete via OFF path after speed-change step"
        );
    }

    #[test]
    fn test_recorder_completion_via_speed_change() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        let base_duty: u16 = 400;
        for step in 0..CAL_STEPS {
            let duty = base_duty + step as u16 * 25;

            if step == 0 {
                t += 1;
                rec.update(duty, t);
            }

            t += SPEED_CHANGE_SETTLE_MS.max(STEP_SETTLE_MS) + 1;
            rec.update(duty, t);

            let mut recorded = false;
            for _ in 0..500 {
                t += 1;
                match rec.update(duty, t) {
                    CalEvent::StepRecorded(idx, _, _) => {
                        assert_eq!(idx as usize, step);
                        recorded = true;
                        break;
                    }
                    CalEvent::Complete(_) => {
                        panic!("Got Complete too early at step {}", step);
                    }
                    _ => {}
                }
            }
            assert!(recorded, "Step {} should have been recorded", step);

            if step < CAL_STEPS - 1 {
                let next_duty = base_duty + (step as u16 + 1) * 25;
                t += 1;
                rec.update(next_duty, t);
            }
        }

        t += 1;
        let evt = rec.update(base_duty + 386 * 25, t);
        assert!(
            matches!(evt, CalEvent::Complete(_)),
            "Should get Complete after all steps"
        );
    }

    #[test]
    fn test_recorder_signal_drop_during_settling_resets() {
        let mut rec = CalibrationRecorder::new(0);

        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        t += 20;
        rec.update(500, t);

        t += 1000;
        rec.update(500, t);
        t += 20;
        let evt = rec.update(0, t);
        assert_eq!(evt, CalEvent::None);
        assert!(!rec.is_recording());
    }

    #[test]
    fn test_recorder_expected_rpm_progresses() {
        let rec = CalibrationRecorder::new(0);
        assert_eq!(rec.current_expected_rpm(), CAL_START_RPM);

        let rec2 = CalibrationRecorder::new(0);
        assert_eq!(rec2.step_index(), 0);
        assert_eq!(rec2.current_expected_rpm(), CAL_START_RPM);
    }

    #[test]
    fn test_recorder_off_debounce_during_gap() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        t += 20;
        rec.update(500, t);
        t += STEP_SETTLE_MS + 100;
        rec.update(500, t);
        t += STEP_RECORD_MS + 100;
        rec.update(500, t);

        t += 20;
        rec.update(0, t);
        t += 20;
        rec.update(500, t);
        t += 20;
        rec.update(0, t);
        assert_eq!(rec.step_index(), 1);
    }

    #[test]
    fn test_recorder_signal_drop_during_recording_with_enough_samples() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        t += 1;
        rec.update(500, t);
        t += STEP_SETTLE_MS + 1;
        rec.update(500, t);
        assert!(rec.is_recording());

        t += 1;
        rec.update(500, t);
        t += 1;
        rec.update(500, t);
        t += 1;
        rec.update(500, t);

        t += 1;
        let evt = rec.update(0, t);
        assert!(
            matches!(evt, CalEvent::StepRecorded(0, _, 500)),
            "Should finish step with partial average, got {:?}",
            evt
        );
    }

    #[test]
    fn test_recorder_signal_drop_during_recording_too_few_samples() {
        let mut rec = CalibrationRecorder::new(0);
        let mut t: u64 = ANNOUNCE_MS + 1;
        rec.update(0, t);

        t += 1;
        rec.update(500, t);
        t += STEP_SETTLE_MS + 1;
        rec.update(500, t);
        assert!(rec.is_recording());

        t += 1;
        rec.update(500, t);
        t += 1;
        rec.update(500, t);

        t += 1;
        let evt = rec.update(0, t);
        assert_eq!(evt, CalEvent::None, "Should discard and go back to waiting");
        assert!(!rec.is_recording());
        assert_eq!(rec.step_index(), 0, "Step index should not advance");
    }
}

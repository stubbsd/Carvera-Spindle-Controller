//! Calibration Embassy task.
//!
//! Runs at 50Hz, completely decoupled from spindle_control.
//! 1. Boot: reads flash for existing calibration
//! 2. Detection loop: feeds MEASURED_DUTY to SequenceDetectors (4 sequences)
//! 3. Recording loop as triggered
//! 4. On complete: prints table via defmt, writes to flash, applies to static
//! 5. Publishes status to CAL_STATUS Watch channel for LCD task

use core::sync::atomic::Ordering;
use embassy_rp::flash::{Blocking, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_time::{Duration, Instant, Timer};

use crate::calibration::{
    CAL_STEPS, CalEvent, CalPhase, CalibrationRecorder, CalibrationStatus, SequenceDetector,
    apply_calibration, clear_calibration, read_calibration_table,
};
use crate::flash_store::{
    FLASH_SIZE, compute_checksum, erase_calibration, read_calibration, write_calibration,
};
use crate::state::{CAL_RECORDING, CAL_SEQUENCE_ACTIVE, update_cal_status};
use crate::tasks::pwm_input::MEASURED_DUTY;

/// Calibration task loop interval (20ms = 50Hz)
const CAL_INTERVAL_MS: u64 = 20;

/// Duration to show "loaded" or "no cal" message on LCD (ms)
const BOOT_MESSAGE_MS: u64 = 2_000;

/// Duration to show "complete" or "aborted" message on LCD (ms)
const RESULT_MESSAGE_MS: u64 = 5_000;

/// Write a string slice into buf at pos, return new position.
fn write_str(buf: &mut [u8], pos: usize, s: &[u8]) -> usize {
    let end = pos + s.len();
    buf[pos..end].copy_from_slice(s);
    end
}

/// Write a u16 as decimal (no leading zeros) into buf at pos, return new position.
fn write_u16(buf: &mut [u8], pos: usize, val: u16) -> usize {
    if val == 0 {
        buf[pos] = b'0';
        return pos + 1;
    }
    // Max u16 = 65535 (5 digits)
    let mut digits = [0u8; 5];
    let mut n = val;
    let mut i = 5;
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let len = 5 - i;
    buf[pos..pos + len].copy_from_slice(&digits[i..]);
    pos + len
}

/// Write a u16 as 4-char uppercase hex (e.g. "0A3F") into buf at pos, return new position.
fn write_hex_u16(buf: &mut [u8], pos: usize, val: u16) -> usize {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    buf[pos] = HEX[((val >> 12) & 0xF) as usize];
    buf[pos + 1] = HEX[((val >> 8) & 0xF) as usize];
    buf[pos + 2] = HEX[((val >> 4) & 0xF) as usize];
    buf[pos + 3] = HEX[(val & 0xF) as usize];
    pos + 4
}

/// Print calibration table as single-line JSON via defmt.
///
/// Format: `{"v":1,"n":386,"crc":"0xABCD","cal":[[750,1234],[800,1300],...]}`
///
/// Uses a static buffer (called once from single-threaded calibration task).
fn print_cal_table(table: &crate::calibration::CalibrationTable) {
    use core::cell::UnsafeCell;

    // 6144 bytes fits ~386 points at ~13 bytes each plus header/footer
    struct JsonBuf(UnsafeCell<[u8; 6144]>);
    unsafe impl Sync for JsonBuf {}
    static JSON_BUF: JsonBuf = JsonBuf(UnsafeCell::new([0u8; 6144]));

    // Safety: called only from the single-threaded calibration task, never concurrently
    let buf = unsafe { &mut *JSON_BUF.0.get() };

    let crc = compute_checksum(table);

    let mut p = 0;
    p = write_str(buf, p, b"{\"v\":1,\"type\":\"carvera\",\"n\":");
    p = write_u16(buf, p, table.count);
    p = write_str(buf, p, b",\"crc\":\"0x");
    p = write_hex_u16(buf, p, crc);
    p = write_str(buf, p, b"\",\"cal\":[");

    for i in 0..table.count as usize {
        if i > 0 {
            buf[p] = b',';
            p += 1;
        }
        buf[p] = b'[';
        p += 1;
        p = write_u16(buf, p, table.points[i].expected_rpm);
        buf[p] = b',';
        p += 1;
        p = write_u16(buf, p, table.points[i].measured_duty);
        buf[p] = b']';
        p += 1;
    }

    p = write_str(buf, p, b"]}");

    // SAFETY: all bytes written are valid ASCII
    let json_str = unsafe { core::str::from_utf8_unchecked(&buf[..p]) };
    defmt::info!("{=str}", json_str);
}

/// Helper to reset all sequence detectors.
struct AllDetectors {
    cal: SequenceDetector,
    clear: SequenceDetector,
    dump: SequenceDetector,
}

impl AllDetectors {
    fn new() -> Self {
        Self {
            cal: SequenceDetector::new(), // 6000→12000→9000
            clear: SequenceDetector::new_with_notes([12000, 6000, 9000]), // Carvera clear
            dump: SequenceDetector::new_with_notes([9000, 6000, 12000]), // Carvera dump
        }
    }

    fn any_active(&self) -> bool {
        self.cal.is_active() || self.clear.is_active() || self.dump.is_active()
    }
}

#[embassy_executor::task]
pub async fn calibration_task(mut flash: Flash<'static, FLASH, Blocking, FLASH_SIZE>) {
    defmt::info!("Calibration task starting...");

    // === Phase 1: Boot — check flash for existing calibration ===
    let has_cal = match read_calibration(&mut flash) {
        Some(table) => {
            defmt::info!("Carvera cal loaded: {} points, CRC OK", table.count);
            apply_calibration(&table);

            update_cal_status(CalibrationStatus {
                phase: CalPhase::Loaded,
                step: 0,
                total_steps: table.count,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
            Timer::after(Duration::from_millis(BOOT_MESSAGE_MS)).await;
            true
        }
        None => {
            defmt::warn!("WARNING: No Carvera calibration data found in flash");

            update_cal_status(CalibrationStatus {
                phase: CalPhase::NoCal,
                step: 0,
                total_steps: CAL_STEPS as u16,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
            Timer::after(Duration::from_millis(BOOT_MESSAGE_MS)).await;
            false
        }
    };

    // === Phase 2: Detection loop ===
    let mut det = AllDetectors::new();

    update_cal_status(CalibrationStatus {
        phase: CalPhase::Detecting,
        step: 0,
        total_steps: CAL_STEPS as u16,
        expected_rpm: 0,
        measured_duty: 0,
        recording: false,
    });

    if has_cal {
        defmt::info!("Calibration loaded, listening for sequences...");
    } else {
        defmt::info!("No calibration, listening for sequences...");
    }

    loop {
        let duty = MEASURED_DUTY.load(Ordering::SeqCst) as u16;
        let now_ms = Instant::now().as_millis();

        // === Carvera Dump: 9000 → 6000 → 12000 ===
        if det.dump.update(duty, now_ms) {
            defmt::info!("Dump calibration sequence detected!");
            CAL_SEQUENCE_ACTIVE.store(true, Ordering::SeqCst);
            CAL_RECORDING.store(true, Ordering::SeqCst);

            match read_calibration_table() {
                Some(table) => {
                    defmt::info!(
                        "Dumping Carvera calibration table ({} points)...",
                        table.count
                    );
                    print_cal_table(&table);
                }
                None => {
                    defmt::warn!("No Carvera calibration data to dump");
                }
            }

            CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);
            CAL_RECORDING.store(false, Ordering::SeqCst);
            det = AllDetectors::new();
        }
        // === Carvera Clear: 12000 → 6000 → 9000 ===
        else if det.clear.update(duty, now_ms) {
            defmt::info!("Clear calibration sequence detected!");
            CAL_SEQUENCE_ACTIVE.store(true, Ordering::SeqCst);
            CAL_RECORDING.store(true, Ordering::SeqCst);

            match erase_calibration(&mut flash) {
                Ok(()) => defmt::info!("Calibration erased from flash"),
                Err(_e) => defmt::error!("Failed to erase calibration from flash"),
            }

            clear_calibration();

            update_cal_status(CalibrationStatus {
                phase: CalPhase::Cleared,
                step: 0,
                total_steps: 0,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
            Timer::after(Duration::from_millis(RESULT_MESSAGE_MS)).await;

            update_cal_status(CalibrationStatus {
                phase: CalPhase::NoCal,
                step: 0,
                total_steps: CAL_STEPS as u16,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
            Timer::after(Duration::from_millis(BOOT_MESSAGE_MS)).await;

            CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);
            CAL_RECORDING.store(false, Ordering::SeqCst);
            det = AllDetectors::new();
            update_cal_status(CalibrationStatus {
                phase: CalPhase::Detecting,
                step: 0,
                total_steps: CAL_STEPS as u16,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
        }
        // === Carvera Calibrate: 6000 → 12000 → 9000 ===
        else if det.cal.update(duty, now_ms) {
            defmt::info!("Calibration sequence detected!");
            CAL_SEQUENCE_ACTIVE.store(true, Ordering::SeqCst);
            CAL_RECORDING.store(true, Ordering::SeqCst);

            update_cal_status(CalibrationStatus {
                phase: CalPhase::SequenceDetected,
                step: 0,
                total_steps: CAL_STEPS as u16,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });

            let mut recorder = CalibrationRecorder::new(Instant::now().as_millis());

            'recording: loop {
                Timer::after(Duration::from_millis(CAL_INTERVAL_MS)).await;

                let duty = MEASURED_DUTY.load(Ordering::SeqCst) as u16;
                let now_ms = Instant::now().as_millis();

                let event = recorder.update(duty, now_ms);

                update_cal_status(CalibrationStatus {
                    phase: CalPhase::Recording,
                    step: recorder.step_index() + 1,
                    total_steps: CAL_STEPS as u16,
                    expected_rpm: recorder.current_expected_rpm(),
                    measured_duty: duty,
                    recording: recorder.is_recording(),
                });

                match event {
                    CalEvent::None => {}
                    CalEvent::StepRecorded(idx, rpm, measured) => {
                        defmt::info!(
                            "CAL step {}/{}: {} RPM -> duty {}",
                            idx + 1,
                            CAL_STEPS,
                            rpm,
                            measured
                        );
                    }
                    CalEvent::Complete(table) => {
                        defmt::info!("Calibration complete! {} points recorded", table.count);
                        print_cal_table(&table);

                        let flash_ok = match write_calibration(&mut flash, &table) {
                            Ok(()) => {
                                defmt::info!("Calibration saved to flash");
                                true
                            }
                            Err(_e) => {
                                defmt::error!("Failed to write calibration to flash");
                                false
                            }
                        };

                        // Apply calibration to RAM regardless of flash result
                        apply_calibration(&table);

                        if flash_ok {
                            update_cal_status(CalibrationStatus {
                                phase: CalPhase::Complete,
                                step: table.count,
                                total_steps: table.count,
                                expected_rpm: 0,
                                measured_duty: 0,
                                recording: false,
                            });
                        } else {
                            // Flash failed: show aborted so LCD displays error
                            update_cal_status(CalibrationStatus {
                                phase: CalPhase::Aborted,
                                step: table.count,
                                total_steps: table.count,
                                expected_rpm: 0,
                                measured_duty: 0,
                                recording: false,
                            });
                        }
                        defmt::info!("Calibration done. Resuming normal operation.");
                        Timer::after(Duration::from_millis(RESULT_MESSAGE_MS)).await;
                        break 'recording;
                    }
                    CalEvent::Aborted => {
                        defmt::error!("Calibration ABORTED: signal lost");

                        update_cal_status(CalibrationStatus {
                            phase: CalPhase::Aborted,
                            step: recorder.step_index(),
                            total_steps: CAL_STEPS as u16,
                            expected_rpm: 0,
                            measured_duty: 0,
                            recording: false,
                        });
                        Timer::after(Duration::from_millis(RESULT_MESSAGE_MS)).await;
                        break 'recording;
                    }
                }
            }

            CAL_SEQUENCE_ACTIVE.store(false, Ordering::SeqCst);
            CAL_RECORDING.store(false, Ordering::SeqCst);
            det = AllDetectors::new();
            update_cal_status(CalibrationStatus {
                phase: CalPhase::Detecting,
                step: 0,
                total_steps: CAL_STEPS as u16,
                expected_rpm: 0,
                measured_duty: 0,
                recording: false,
            });
        } else {
            // Suppress stall detection while partway through any musical pattern
            CAL_SEQUENCE_ACTIVE.store(det.any_active(), Ordering::SeqCst);
        }

        Timer::after(Duration::from_millis(CAL_INTERVAL_MS)).await;
    }
}

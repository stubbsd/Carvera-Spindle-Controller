//! LCD formatting functions (platform-agnostic).
//!
//! Pure functions for formatting 16x2 LCD display lines and calculating
//! RGB backlight colors. Testable on host without embedded dependencies.

use crate::display::ErrorType;
use crate::state::config;

use super::BacklightColor;

/// Calculate deviation percentage from target
/// Returns (capped_pct, overflow_pct) where overflow_pct is Some if |deviation| > 99%
pub fn calculate_deviation(target: u32, actual: u32) -> (i32, Option<i32>) {
    if target == 0 {
        return (0, None);
    }

    let diff = actual as i64 - target as i64;
    let pct = ((diff * 100) / target as i64) as i32;

    if pct > 99 {
        (99, Some(pct))
    } else if pct < -99 {
        (-99, Some(pct))
    } else {
        (pct, None)
    }
}

/// Format line 1: "i18000 -82% 5.1A" (16 chars)
/// Position 0 is custom char 0 (speed icon)
pub fn format_line1(target_rpm: u32, deviation_pct: i32, current_ma: u32) -> [u8; 16] {
    let mut buf = [b' '; 16];

    // Position 0: Custom character 0 (speed icon)
    buf[0] = 0x00;

    // Position 1-5: Target RPM (right-aligned, 5 digits)
    write_u32_right(&mut buf[1..6], target_rpm, 5);

    // Position 7-10: Deviation % (sign + value + %)
    // Right-align so sign stays next to digit: " +2%" or "-35%"
    let abs_pct = deviation_pct.unsigned_abs();
    if abs_pct >= 10 {
        // Two digits: "-35%"
        if deviation_pct > 0 {
            buf[7] = b'+';
        } else {
            buf[7] = b'-';
        }
        buf[8] = b'0' + ((abs_pct / 10) % 10) as u8;
        buf[9] = b'0' + (abs_pct % 10) as u8;
    } else {
        // One digit: " +2%" or " 0%"
        buf[7] = b' ';
        if deviation_pct > 0 {
            buf[8] = b'+';
        } else if deviation_pct < 0 {
            buf[8] = b'-';
        } else {
            buf[8] = b' ';
        }
        buf[9] = b'0' + (abs_pct % 10) as u8;
    }
    buf[10] = b'%';

    // Position 12-15: Current "X.XA" (position 11 is space)
    let amps = current_ma / 1000;
    let tenths = (current_ma % 1000) / 100;
    buf[12] = b'0' + (amps % 10) as u8;
    buf[13] = b'.';
    buf[14] = b'0' + (tenths % 10) as u8;
    buf[15] = b'A';

    buf
}

/// Status for line 2
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Ok,
    Stall,
    StallCleared, // Stall cleared but history indicator
    Error(ErrorType),
}

/// Format line 2: Status + actual RPM for debugging (16 chars)
/// Format: "OK    ACT:12345" or "STALL!" (no ACT when warning/error)
/// With stabilization time: "OK 1.3s A:12345" (shorter "A:" prefix to fit time)
pub fn format_line2(
    status: Status,
    overflow_pct: Option<i32>,
    enabled: bool,
    actual_rpm: u32,
    stabilization_time_ms: Option<u32>,
) -> [u8; 16] {
    let mut buf = [b' '; 16];

    // Track if we have an error/warning condition
    // Priority order: Stall > Alert > StallCleared > Overflow Warning > OK/OFF
    let has_error_or_warning;

    match status {
        Status::Stall => {
            // Highest priority: stall always shows, even if there's overflow
            has_error_or_warning = true;
            buf[0..6].copy_from_slice(b"STALL!");
        }
        Status::Error(_) => {
            // Latched errors use both lines via format_error_lines(); this is a fallback
            has_error_or_warning = true;
            buf[0..6].copy_from_slice(b"ERROR!");
        }
        Status::StallCleared => {
            has_error_or_warning = true; // Prevent ACT: from showing
            buf[0..13].copy_from_slice(b"OFF - STALLED");
        }
        Status::Ok => {
            // Only show overflow warning when status is Ok
            if let Some(pct) = overflow_pct {
                // "WARN +300%"
                has_error_or_warning = true;
                buf[0..4].copy_from_slice(b"WARN");
                buf[4] = b' ';
                if pct >= 0 {
                    buf[5] = b'+';
                } else {
                    buf[5] = b'-';
                }
                let abs_pct = pct.unsigned_abs();
                if abs_pct >= 100 {
                    buf[6] = b'0' + ((abs_pct / 100) % 10) as u8;
                    buf[7] = b'0' + ((abs_pct / 10) % 10) as u8;
                    buf[8] = b'0' + (abs_pct % 10) as u8;
                    buf[9] = b'%';
                } else if abs_pct >= 10 {
                    buf[6] = b'0' + ((abs_pct / 10) % 10) as u8;
                    buf[7] = b'0' + (abs_pct % 10) as u8;
                    buf[8] = b'%';
                } else {
                    buf[6] = b'0' + (abs_pct % 10) as u8;
                    buf[7] = b'%';
                }
            } else if !enabled {
                has_error_or_warning = false;
                buf[0..3].copy_from_slice(b"OFF");
            } else {
                has_error_or_warning = false;
                buf[0..2].copy_from_slice(b"OK");
            }
        }
    }

    // Only show actual RPM when spindle is running and no error/warning
    if !has_error_or_warning && enabled {
        // Check if we should show stabilization time
        if let Some(time_ms) = stabilization_time_ms {
            // Format: "OK 1.3s A:12345" or "OK12.3s A:12345" for times >= 10s
            let seconds = time_ms / 1000;
            let tenths = (time_ms % 1000) / 100;

            if seconds >= 10 {
                // Two-digit seconds: "OK12.3s A:xxxxx"
                buf[2] = b'0' + ((seconds / 10) % 10) as u8;
                buf[3] = b'0' + (seconds % 10) as u8;
                buf[4] = b'.';
                buf[5] = b'0' + (tenths % 10) as u8;
                buf[6] = b's';
            } else {
                // Single-digit seconds: "OK 1.3s A:xxxxx"
                buf[3] = b'0' + (seconds % 10) as u8;
                buf[4] = b'.';
                buf[5] = b'0' + (tenths % 10) as u8;
                buf[6] = b's';
            }
            // Shorter "A:" prefix to fit time
            buf[8..10].copy_from_slice(b"A:");
            write_u32_right(&mut buf[10..16], actual_rpm, 6);
        } else {
            // Normal format: "OK    ACT:12345"
            buf[6..10].copy_from_slice(b"ACT:");
            write_u32_right(&mut buf[10..16], actual_rpm, 6);
        }
    }

    buf
}

/// Format both LCD lines for a latched error (uses full display).
/// Returns (line1, line2) — each 16 bytes.
pub fn format_error_lines(error_type: ErrorType) -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line2[0..16].copy_from_slice(b"  RESTART REQD  ");
    match error_type {
        ErrorType::EsconAlert => {
            line1[0..16].copy_from_slice(b"!! ESCON ALERT !");
        }
        ErrorType::Overcurrent => {
            line1[0..16].copy_from_slice(b"! OVER CURRENT !");
        }
        ErrorType::Thermal => {
            line1[0..16].copy_from_slice(b"! THERMAL FAULT!");
        }
        _ => {
            // Shouldn't be called for non-latched errors, but handle gracefully
            line1[0..16].copy_from_slice(b"!    ERROR     !");
        }
    }
    (line1, line2)
}

/// Calculate RGB backlight color based on spindle state
pub fn calculate_backlight(
    enabled: bool,
    target_rpm: u32,
    actual_rpm: u32,
    current_ma: u32,
    error: bool,
) -> BacklightColor {
    // Error states force red
    if error {
        return BacklightColor::RED;
    }

    // Spindle off = dim green
    if !enabled {
        return BacklightColor::DIM_GREEN;
    }

    // Calculate severity from current (0-100)
    let current_pct = (current_ma * 100 / config::CURRENT_AT_3V3_MA) as i32;
    let current_severity = match current_pct {
        0..=59 => 0,
        60..=79 => 1,
        80..=89 => 2,
        _ => 3,
    };

    // Calculate severity from deviation
    let (deviation_pct, _) = calculate_deviation(target_rpm, actual_rpm);
    let abs_deviation = deviation_pct.abs();
    let deviation_severity = match abs_deviation {
        0..=19 => 0,
        20..=34 => 1,
        35..=49 => 2,
        _ => 3,
    };

    // Use worst case
    let severity = current_severity.max(deviation_severity);

    match severity {
        0 => BacklightColor::BRIGHT_GREEN,
        1 => BacklightColor::YELLOW,
        2 => BacklightColor::ORANGE,
        _ => BacklightColor::RED,
    }
}

// Helper: write u32 right-aligned into buffer
fn write_u32_right(buf: &mut [u8], val: u32, width: usize) {
    let mut n = val;
    for i in (0..width).rev() {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            buf[..i].fill(b' ');
            break;
        }
    }
}

// ============================================================================
// Calibration Display Formatting
// ============================================================================

/// Format calibration line 1: "C 005/386  5000 " (16 chars)
pub fn format_cal_line1(step: u16, total: u16, rpm: u16) -> [u8; 16] {
    let mut buf = [b' '; 16];
    buf[0] = b'C';
    // Step number (3 digits, zero-padded) at positions 2-4
    buf[2] = b'0' + ((step / 100) % 10) as u8;
    buf[3] = b'0' + ((step / 10) % 10) as u8;
    buf[4] = b'0' + (step % 10) as u8;
    buf[5] = b'/';
    // Total (3 digits, zero-padded) at positions 6-8
    buf[6] = b'0' + ((total / 100) % 10) as u8;
    buf[7] = b'0' + ((total / 10) % 10) as u8;
    buf[8] = b'0' + (total % 10) as u8;
    // RPM right-aligned in positions 11-15
    write_u32_right(&mut buf[11..16], rpm as u32, 5);
    buf
}

/// Format calibration line 2: "IN 24.47%       " (16 chars)
pub fn format_cal_line2(duty: u16) -> [u8; 16] {
    let mut buf = [b' '; 16];
    buf[0..2].copy_from_slice(b"IN");
    // Duty as XX.XX% (positions 3-8)
    let whole = duty / 100;
    let frac = duty % 100;
    if whole >= 100 {
        // 100.00% edge case
        buf[3] = b'1';
        buf[4] = b'0';
        buf[5] = b'0';
        buf[6] = b'.';
        buf[7] = b'0' + ((frac / 10) % 10) as u8;
        buf[8] = b'0' + (frac % 10) as u8;
    } else if whole >= 10 {
        buf[3] = b' ';
        buf[4] = b'0' + ((whole / 10) % 10) as u8;
        buf[5] = b'0' + (whole % 10) as u8;
        buf[6] = b'.';
        buf[7] = b'0' + ((frac / 10) % 10) as u8;
        buf[8] = b'0' + (frac % 10) as u8;
    } else {
        buf[3] = b' ';
        buf[4] = b' ';
        buf[5] = b'0' + (whole % 10) as u8;
        buf[6] = b'.';
        buf[7] = b'0' + ((frac / 10) % 10) as u8;
        buf[8] = b'0' + (frac % 10) as u8;
    }
    buf[9] = b'%';
    buf
}

/// Format calibration detection display (two lines).
/// Returns (line1, line2).
pub fn format_cal_detect() -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line1[1..14].copy_from_slice(b" CALIBRATION ");
    line2[3..13].copy_from_slice(b"DETECTED! ");
    (line1, line2)
}

/// Format "no calibration" warning display.
/// Returns (line1, line2).
pub fn format_no_cal_warning() -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line1[0..16].copy_from_slice(b" NO CALIBRATION ");
    line2[0..16].copy_from_slice(b" RUN CAL GCODE  ");
    (line1, line2)
}

/// Format calibration complete display.
pub fn format_cal_complete() -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line1[0..13].copy_from_slice(b"CAL COMPLETE!");
    line2[0..16].copy_from_slice(b"  RESUMING...   ");
    (line1, line2)
}

/// Format calibration cleared display.
pub fn format_cal_cleared() -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line1[0..12].copy_from_slice(b"CAL CLEARED!");
    line2[0..16].copy_from_slice(b" RUN CAL GCODE  ");
    (line1, line2)
}

/// Format calibration aborted display.
pub fn format_cal_aborted() -> ([u8; 16], [u8; 16]) {
    let mut line1 = [b' '; 16];
    let mut line2 = [b' '; 16];
    line1[0..13].copy_from_slice(b"CAL ABORTED! ");
    line2[0..12].copy_from_slice(b"SIGNAL LOST ");
    (line1, line2)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_deviation_normal() {
        let (pct, overflow) = calculate_deviation(10000, 10000);
        assert_eq!(pct, 0);
        assert!(overflow.is_none());

        let (pct, overflow) = calculate_deviation(10000, 10500);
        assert_eq!(pct, 5);
        assert!(overflow.is_none());

        let (pct, overflow) = calculate_deviation(10000, 9500);
        assert_eq!(pct, -5);
        assert!(overflow.is_none());
    }

    #[test]
    fn test_calculate_deviation_overflow() {
        let (pct, overflow) = calculate_deviation(10000, 30000);
        assert_eq!(pct, 99);
        assert_eq!(overflow, Some(200));

        let (pct, overflow) = calculate_deviation(10000, 0);
        assert_eq!(pct, -99);
        assert_eq!(overflow, Some(-100));
    }

    #[test]
    fn test_format_line1() {
        let line = format_line1(12000, -82, 5100);
        assert_eq!(line[1..6], *b"12000");
        assert_eq!(line[7..11], *b"-82%");
        assert_eq!(line[12..16], *b"5.1A");

        // Single digit deviation
        let line2 = format_line1(12000, 2, 2100);
        assert_eq!(line2[7..11], *b" +2%");
    }

    #[test]
    fn test_format_line2() {
        // Enabled and OK = "OK" with ACT: shown
        let line = format_line2(Status::Ok, None, true, 12000, None);
        assert_eq!(&line[0..2], b"OK");
        assert_eq!(&line[6..10], b"ACT:");

        // Disabled and OK = "OFF" without ACT: (spindle not running)
        let line = format_line2(Status::Ok, None, false, 0, None);
        assert_eq!(&line[0..3], b"OFF");
        assert_eq!(&line[6..10], b"    "); // No ACT: when disabled

        // Stall = "STALL!" without ACT: (error message takes priority)
        let line = format_line2(Status::Stall, None, true, 5000, None);
        assert_eq!(&line[0..6], b"STALL!");
        assert_eq!(&line[6..10], b"    "); // No ACT: - spaces only

        // StallCleared = "OFF - STALLED" without ACT:
        let line = format_line2(Status::StallCleared, None, false, 0, None);
        assert_eq!(&line[0..13], b"OFF - STALLED");
        assert_eq!(&line[13..16], b"   "); // Rest is spaces
    }

    #[test]
    fn test_backlight_colors() {
        // Spindle off = dim green
        let color = calculate_backlight(false, 0, 0, 0, false);
        assert_eq!(color.g, 15);

        // Normal operation = bright green
        let color = calculate_backlight(true, 10000, 10000, 2000, false);
        assert_eq!(color.g, 255);
        assert_eq!(color.r, 0);

        // Error = red
        let color = calculate_backlight(true, 10000, 10000, 2000, true);
        assert_eq!(color.r, 255);
    }

    #[test]
    fn test_calculate_deviation_zero_target() {
        let (pct, overflow) = calculate_deviation(0, 1000);
        assert_eq!(pct, 0);
        assert!(overflow.is_none());
    }

    #[test]
    fn test_format_line1_zero_values() {
        let line = format_line1(0, 0, 0);
        assert_eq!(line[5], b'0');
        assert_eq!(line[8..11], *b" 0%");
        assert_eq!(line[12..16], *b"0.0A");
    }

    #[test]
    fn test_format_line1_max_values() {
        let line = format_line1(99999, 99, 9900);
        assert_eq!(line[1..6], *b"99999");
        assert_eq!(line[7..11], *b"+99%");
        assert_eq!(line[12..16], *b"9.9A");
    }

    #[test]
    fn test_format_line2_overflow_warning() {
        let line = format_line2(Status::Ok, Some(150), true, 10000, None);
        assert_eq!(&line[0..4], b"WARN");
        assert_ne!(&line[6..10], b"ACT:");
    }

    #[test]
    fn test_format_line2_error_fallback() {
        let line = format_line2(
            Status::Error(ErrorType::Overcurrent),
            None,
            true,
            8000,
            None,
        );
        assert_eq!(&line[0..6], b"ERROR!");
        assert_eq!(&line[6..10], b"    ");
    }

    #[test]
    fn test_format_line2_stall_priority_over_overflow() {
        let line = format_line2(Status::Stall, Some(-100), true, 0, None);
        assert_eq!(&line[0..6], b"STALL!");
        assert_ne!(&line[0..4], b"WARN");
    }

    #[test]
    fn test_format_line2_error_priority_over_overflow() {
        let line = format_line2(
            Status::Error(ErrorType::Thermal),
            Some(150),
            true,
            5000,
            None,
        );
        assert_eq!(&line[0..6], b"ERROR!");
        assert_ne!(&line[0..4], b"WARN");
    }

    #[test]
    fn test_format_error_lines_escon_alert() {
        let (line1, line2) = format_error_lines(ErrorType::EsconAlert);
        assert_eq!(&line1, b"!! ESCON ALERT !");
        assert_eq!(&line2, b"  RESTART REQD  ");
    }

    #[test]
    fn test_format_error_lines_overcurrent() {
        let (line1, line2) = format_error_lines(ErrorType::Overcurrent);
        assert_eq!(&line1, b"! OVER CURRENT !");
        assert_eq!(&line2, b"  RESTART REQD  ");
    }

    #[test]
    fn test_format_error_lines_thermal() {
        let (line1, line2) = format_error_lines(ErrorType::Thermal);
        assert_eq!(&line1, b"! THERMAL FAULT!");
        assert_eq!(&line2, b"  RESTART REQD  ");
    }

    #[test]
    fn test_format_error_lines_unexpected_type() {
        // Non-latched error types should still produce a valid display
        let (line1, line2) = format_error_lines(ErrorType::None);
        assert_eq!(&line1, b"!    ERROR     !");
        assert_eq!(&line2, b"  RESTART REQD  ");
    }

    #[test]
    fn test_format_line2_overflow_only_when_ok() {
        let line = format_line2(Status::Ok, Some(-50), true, 5000, None);
        assert_eq!(&line[0..4], b"WARN");
    }

    #[test]
    fn test_format_line2_with_stabilization_time() {
        // Stabilization time < 10s: "OK 1.3s A:12345"
        let line = format_line2(Status::Ok, None, true, 12345, Some(1300));
        assert_eq!(&line[0..2], b"OK");
        assert_eq!(line[3], b'1');
        assert_eq!(line[4], b'.');
        assert_eq!(line[5], b'3');
        assert_eq!(line[6], b's');
        assert_eq!(&line[8..10], b"A:");
        assert_eq!(&line[10..16], b" 12345");

        // Stabilization time >= 10s: "OK12.3s A:xxxxx"
        let line = format_line2(Status::Ok, None, true, 8500, Some(12300));
        assert_eq!(&line[0..2], b"OK");
        assert_eq!(line[2], b'1');
        assert_eq!(line[3], b'2');
        assert_eq!(line[4], b'.');
        assert_eq!(line[5], b'3');
        assert_eq!(line[6], b's');
        assert_eq!(&line[8..10], b"A:");

        // Stabilization time with sub-second (0.5s)
        let line = format_line2(Status::Ok, None, true, 10000, Some(500));
        assert_eq!(line[3], b'0');
        assert_eq!(line[5], b'5');

        // No stabilization time during error
        let line = format_line2(Status::Stall, None, true, 5000, Some(1000));
        assert_eq!(&line[0..6], b"STALL!");
    }

    #[test]
    fn test_backlight_warning_levels() {
        // High current (60-80%) = yellow
        let color = calculate_backlight(true, 10000, 10000, 3500, false);
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 255);
        assert_eq!(color.b, 0);

        // Very high current (80-90%) = orange
        let color = calculate_backlight(true, 10000, 10000, 4250, false);
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 128);
        assert_eq!(color.b, 0);

        // Large deviation (20-35%) = yellow
        let color = calculate_backlight(true, 10000, 7500, 2000, false);
        assert_eq!(color.r, 255);
        assert_eq!(color.g, 255);
    }

    // --- Calibration format tests ---

    #[test]
    fn test_format_cal_line1() {
        let line = format_cal_line1(5, 386, 5000);
        assert_eq!(line[0], b'C');
        assert_eq!(line[2], b'0');
        assert_eq!(line[3], b'0');
        assert_eq!(line[4], b'5');
        assert_eq!(line[5], b'/');
        assert_eq!(line[6], b'3');
        assert_eq!(line[7], b'8');
        assert_eq!(line[8], b'6');
        assert_eq!(&line[11..16], b" 5000");
    }

    #[test]
    fn test_format_cal_line1_large_step() {
        let line = format_cal_line1(386, 386, 20000);
        assert_eq!(line[2], b'3');
        assert_eq!(line[3], b'8');
        assert_eq!(line[4], b'6');
        assert_eq!(&line[11..16], b"20000");
    }

    #[test]
    fn test_format_cal_line2() {
        let line = format_cal_line2(2447);
        assert_eq!(&line[0..2], b"IN");
        assert_eq!(line[4], b'2');
        assert_eq!(line[5], b'4');
        assert_eq!(line[6], b'.');
        assert_eq!(line[7], b'4');
        assert_eq!(line[8], b'7');
        assert_eq!(line[9], b'%');
        assert_eq!(&line[10..16], b"      ");
    }

    #[test]
    fn test_format_cal_detect() {
        let (line1, line2) = format_cal_detect();
        assert!(line1.windows(11).any(|w| w == b"CALIBRATION"));
        assert!(line2.windows(9).any(|w| w == b"DETECTED!"));
    }

    #[test]
    fn test_format_no_cal_warning() {
        let (line1, line2) = format_no_cal_warning();
        assert!(line1.windows(14).any(|w| w == b"NO CALIBRATION"));
        assert!(line2.windows(12).any(|w| w == b"RUN CAL GCOD"));
    }

    #[test]
    fn test_format_cal_complete() {
        let (line1, line2) = format_cal_complete();
        assert!(line1.windows(12).any(|w| w == b"CAL COMPLETE"));
        assert!(line2.windows(10).any(|w| w == b"RESUMING.."));
    }

    #[test]
    fn test_format_cal_cleared() {
        let (line1, line2) = format_cal_cleared();
        assert!(line1.windows(11).any(|w| w == b"CAL CLEARED"));
        assert!(line2.windows(12).any(|w| w == b"RUN CAL GCOD"));
    }

    #[test]
    fn test_format_cal_aborted() {
        let (line1, line2) = format_cal_aborted();
        assert!(line1.windows(11).any(|w| w == b"CAL ABORTED"));
        assert!(line2.windows(11).any(|w| w == b"SIGNAL LOST"));
    }

    #[test]
    fn test_format_cal_line2_duty_zero() {
        let line = format_cal_line2(0);
        assert_eq!(&line[0..2], b"IN");
        assert_eq!(line[5], b'0');
        assert_eq!(line[6], b'.');
        assert_eq!(line[7], b'0');
        assert_eq!(line[8], b'0');
        assert_eq!(line[9], b'%');
    }

    #[test]
    fn test_format_cal_line2_duty_10000() {
        let line = format_cal_line2(10000);
        assert_eq!(&line[0..2], b"IN");
        assert_eq!(line[3], b'1');
        assert_eq!(line[4], b'0');
        assert_eq!(line[5], b'0');
        assert_eq!(line[6], b'.');
        assert_eq!(line[7], b'0');
        assert_eq!(line[8], b'0');
        assert_eq!(line[9], b'%');
    }

    #[test]
    fn test_format_line1_current_above_10a() {
        let line = format_line1(10000, 0, 12300);
        assert_eq!(line[12], b'2');
        assert_eq!(line[13], b'.');
        assert_eq!(line[14], b'3');
        assert_eq!(line[15], b'A');
    }

    #[test]
    fn test_write_u32_right_via_format_line1_zero_rpm() {
        let line = format_line1(0, 0, 0);
        assert_eq!(line[1], b' ');
        assert_eq!(line[2], b' ');
        assert_eq!(line[3], b' ');
        assert_eq!(line[4], b' ');
        assert_eq!(line[5], b'0');
    }

    #[test]
    fn test_format_line1_deviation_negative_99() {
        let line = format_line1(10000, -99, 1000);
        assert_eq!(line[7], b'-');
        assert_eq!(line[8], b'9');
        assert_eq!(line[9], b'9');
        assert_eq!(line[10], b'%');
    }

    #[test]
    fn test_backlight_calibration_colors() {
        assert_eq!(BacklightColor::BLUE.b, 255);
        assert_eq!(BacklightColor::BLUE.r, 0);
        assert_eq!(BacklightColor::CYAN.g, 255);
        assert_eq!(BacklightColor::CYAN.b, 255);
        assert_eq!(BacklightColor::MAGENTA.r, 255);
        assert_eq!(BacklightColor::MAGENTA.b, 255);
        assert_eq!(BacklightColor::DIM_BLUE.b, 30);
    }
}

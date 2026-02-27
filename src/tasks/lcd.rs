//! LCD display task for HD44780 16x2 with RGB backlight.
//!
//! Subscribes to DISPLAY_DATA channel and updates the LCD with spindle status.
//! Uses the shared driver from `crate::lcd`.

use embassy_rp::gpio::Output;
use embassy_rp::pwm::Pwm;
use embassy_time::{Duration, Timer};

use crate::calibration::CalPhase;
use crate::display::ErrorType;
use crate::lcd::{
    BacklightColor, Hd44780, RgbBacklight, Status, calculate_backlight, calculate_deviation,
    format_cal_aborted, format_cal_cleared, format_cal_complete, format_cal_detect,
    format_cal_line1, format_cal_line2, format_error_lines, format_line1, format_line2,
    format_no_cal_warning,
};
use crate::state::{CAL_STATUS, DISPLAY_DATA, pins};

/// LCD display update interval (500ms = 2Hz)
/// HD44780 has ~350ms internal refresh cycle; slower updates are easier to read
const LCD_UPDATE_INTERVAL_MS: u64 = 500;

/// Peripheral bundle for LCD task
/// Takes pre-constructed Output and Pwm objects from main
pub struct LcdPeripherals<'a> {
    pub rs: Output<'a>,
    pub e: Output<'a>,
    pub d4: Output<'a>,
    pub d5: Output<'a>,
    pub d6: Output<'a>,
    pub d7: Output<'a>,
    pub red_pwm: Pwm<'a>,
    pub green_blue_pwm: Pwm<'a>,
}

/// LCD display task
///
/// Initializes the HD44780 LCD and RGB backlight, then continuously
/// updates the display with spindle status from DISPLAY_DATA channel.
#[embassy_executor::task]
pub async fn lcd_task(peripherals: LcdPeripherals<'static>) {
    // Wait for display power to stabilize (HD44780 needs 40-50ms; 100ms is safe)
    Timer::after(Duration::from_millis(100)).await;

    // Create backlight controller from pre-constructed PWMs
    let mut backlight = RgbBacklight::new(peripherals.red_pwm, peripherals.green_blue_pwm);
    backlight.set_color(&BacklightColor::DIM_GREEN); // Dim during init

    // Create LCD driver from pre-constructed Outputs
    let mut lcd = Hd44780::new(
        peripherals.rs,
        peripherals.e,
        peripherals.d4,
        peripherals.d5,
        peripherals.d6,
        peripherals.d7,
    );
    lcd.init().await;

    // Get receiver for display data
    let Some(mut receiver) = DISPLAY_DATA.receiver() else {
        defmt::error!("LCD task: No display data receiver available - task exiting");
        return;
    };

    defmt::info!(
        "LCD ready: RS={} E={} D4-D7={},{},{},{} RGB={},{},{}",
        pins::LCD_RS,
        pins::LCD_E,
        pins::LCD_D4,
        pins::LCD_D5,
        pins::LCD_D6,
        pins::LCD_D7,
        pins::LCD_RED,
        pins::LCD_GREEN,
        pins::LCD_BLUE
    );

    // Get receiver for calibration status
    let mut cal_receiver = CAL_STATUS.receiver().unwrap();

    // Periodic reset state: re-init LCD every ~10 seconds to recover from power glitches
    let mut reset_counter: u8 = 0;
    let mut last_line1 = [b' '; 16];
    let mut last_line2 = [b' '; 16];

    // Track current calibration phase persistently across loop iterations.
    // Starts as Detecting (passive) so normal display runs until calibration task reports in.
    let mut current_cal_phase = CalPhase::Detecting;

    loop {
        // Check calibration status first (takes priority over normal display)
        if let Some(cal_status) = cal_receiver.try_changed() {
            current_cal_phase = cal_status.phase;

            let (line1, line2, color) = match cal_status.phase {
                CalPhase::NoCal => {
                    let (l1, l2) = format_no_cal_warning();
                    (l1, l2, BacklightColor::YELLOW)
                }
                CalPhase::Detecting => {
                    // Don't override normal display during detection (passive listening)
                    ([b' '; 16], [b' '; 16], BacklightColor::DIM_BLUE)
                }
                CalPhase::SequenceDetected => {
                    let (l1, l2) = format_cal_detect();
                    (l1, l2, BacklightColor::MAGENTA)
                }
                CalPhase::Recording => {
                    let l1 = format_cal_line1(
                        cal_status.step,
                        cal_status.total_steps,
                        cal_status.expected_rpm,
                    );
                    let l2 = format_cal_line2(cal_status.measured_duty);
                    (l1, l2, BacklightColor::CYAN)
                }
                CalPhase::Complete => {
                    let (l1, l2) = format_cal_complete();
                    (l1, l2, BacklightColor::BRIGHT_GREEN)
                }
                CalPhase::Cleared => {
                    let (l1, l2) = format_cal_cleared();
                    (l1, l2, BacklightColor::YELLOW)
                }
                CalPhase::Aborted => {
                    let (l1, l2) = format_cal_aborted();
                    (l1, l2, BacklightColor::RED)
                }
                CalPhase::Loaded => {
                    // Brief "loaded" message — don't override normal display for long
                    ([b' '; 16], [b' '; 16], BacklightColor::DIM_GREEN)
                }
            };

            // Only render calibration-specific screens (not Detecting/Loaded which are passive)
            if matches!(
                cal_status.phase,
                CalPhase::NoCal
                    | CalPhase::SequenceDetected
                    | CalPhase::Recording
                    | CalPhase::Complete
                    | CalPhase::Aborted
                    | CalPhase::Cleared
            ) {
                last_line1 = line1;
                last_line2 = line2;
                backlight.set_color(&color);
                lcd.write_line(0, &line1).await;
                lcd.write_line(1, &line2).await;
            }
        }

        // Determine if calibration is actively using the display (persists across iterations)
        let cal_active = matches!(
            current_cal_phase,
            CalPhase::NoCal
                | CalPhase::SequenceDetected
                | CalPhase::Recording
                | CalPhase::Complete
                | CalPhase::Aborted
                | CalPhase::Cleared
        );

        // Check for new display data (non-blocking) — skip if calibration is actively displaying
        if !cal_active {
            if let Some(status) = receiver.try_changed() {
                // Map ErrorType to LCD Status
                let lcd_status = match status.error_type {
                    ErrorType::None => Status::Ok,
                    ErrorType::StallCleared => Status::StallCleared,
                    ErrorType::Stall => Status::Stall,
                    ErrorType::Overcurrent | ErrorType::EsconAlert | ErrorType::Thermal => {
                        Status::Error(status.error_type)
                    }
                };

                // Latched errors use both lines; normal status uses line1+line2
                let (line1, line2) = if let Status::Error(e) = lcd_status {
                    format_error_lines(e)
                } else {
                    let (deviation_pct, overflow) =
                        calculate_deviation(status.requested_rpm, status.actual_rpm);
                    let l1 = format_line1(status.requested_rpm, deviation_pct, status.current_ma);
                    let l2 = format_line2(
                        lcd_status,
                        overflow,
                        status.enabled,
                        status.actual_rpm,
                        status.stabilization_time_ms,
                    );
                    (l1, l2)
                };

                let color = calculate_backlight(
                    status.enabled,
                    status.requested_rpm,
                    status.actual_rpm,
                    status.current_ma,
                    status.error,
                );

                // Store for potential re-display after reset
                last_line1 = line1;
                last_line2 = line2;

                // Update display
                backlight.set_color(&color);
                lcd.write_line(0, &line1).await;
                lcd.write_line(1, &line2).await;
            }
        } // end if !cal_active

        // Periodic LCD reset every ~5 seconds
        // Normal: 10 iterations * 500ms; Recording: 33 iterations * 150ms
        reset_counter = reset_counter.wrapping_add(1);
        let reset_threshold: u8 = if matches!(current_cal_phase, CalPhase::Recording) {
            33
        } else {
            10
        };
        if reset_counter >= reset_threshold {
            reset_counter = 0;
            defmt::trace!("LCD periodic reset");
            lcd.init().await;
            lcd.write_line(0, &last_line1).await;
            lcd.write_line(1, &last_line2).await;
        }

        // Faster refresh during calibration recording for smooth step counter
        let interval = if matches!(current_cal_phase, CalPhase::Recording) {
            150
        } else {
            LCD_UPDATE_INTERVAL_MS
        };
        Timer::after(Duration::from_millis(interval)).await;
    }
}

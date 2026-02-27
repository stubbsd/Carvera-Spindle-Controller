//! Embassy async tasks for the spindle controller.
//!
//! Each task runs concurrently and handles one aspect of the system:
//! - `spindle_control`: Combined PWM input/output, feedback generation, and stall detection
//! - `speed_measure`: High-resolution speed measurement using T-Method (period measurement)
//! - `current_monitor`: ADC reading and overcurrent protection
//! - `lcd`: HD44780 LCD display with RGB backlight
//! - `led`: Status LED blinking patterns
//! - `watchdog`: Hardware watchdog feeder
//! - `thermal`: MCU temperature monitoring

pub mod calibration;
pub mod current_monitor;
pub mod lcd;
pub mod led;
pub mod pwm_input;
pub mod speed_measure;
pub mod spindle_control;
pub mod thermal;
pub mod watchdog;

pub use calibration::calibration_task;
pub use current_monitor::current_monitor_task;
pub use lcd::LcdPeripherals;
pub use lcd::lcd_task;
pub use led::led_task;
pub use pwm_input::{MEASURED_DUTY, pwm_input_task, setup_pwm_input_pio};
pub use speed_measure::{
    MEASURED_FREQ_MHZ, MEASURED_RPM, setup_speed_measure_pio, speed_measure_task,
};
pub use spindle_control::spindle_control_task;
pub use thermal::thermal_monitor_task;
pub use watchdog::watchdog_task;

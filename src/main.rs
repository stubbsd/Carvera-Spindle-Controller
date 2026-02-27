//! Carvera Spindle Controller - Firmware Entry Point
//!
//! Embassy async firmware that interfaces between Carvera CNC and ESCON 50/5 motor controller.
//! Uses multiple concurrent tasks for different responsibilities.

#![no_std]
#![no_main]

use defmt::{Debug2Format, *};
use embassy_executor::Spawner;
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcInterruptHandler};
use embassy_rp::bind_interrupts;
use embassy_rp::flash::Flash;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_rp::watchdog::Watchdog;

bind_interrupts!(struct AdcIrqs {
    ADC_IRQ_FIFO => AdcInterruptHandler;
});

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => PioInterruptHandler<embassy_rp::peripherals::PIO0>;
});
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

use carvera_spindle::state::{config, init_display_data, pins};
use carvera_spindle::tasks::{
    calibration_task, current_monitor_task, lcd_task, led_task, pwm_input_task,
    setup_pwm_input_pio, setup_speed_measure_pio, speed_measure_task, spindle_control_task,
    watchdog_task,
};

// ============================================================================
// Hardware Configuration
// ============================================================================

/// PWM clock divider for output (150MHz / 150 = 1MHz timer clock)
const PWM_OUT_DIVIDER: u8 = (config::CLOCK_HZ / 1_000_000) as u8;

/// PWM output TOP value (1MHz / PWM_OUTPUT_FREQ_HZ - 1)
/// With 1MHz clock and 1kHz output: TOP = 999
const PWM_OUT_TOP: u16 = (1_000_000 / config::PWM_OUTPUT_FREQ_HZ - 1) as u16;

// ============================================================================
// Main Entry Point
// ============================================================================

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // CRITICAL: Initialize error output HIGH immediately (active-low signaling).
    // Carvera expects: HIGH = OK, LOW = fault (fail-safe design with pull-down).
    let error_out = Output::new(p.PIN_10, Level::High);
    info!(
        "Error output configured on GPIO{} (active-low, initial=HIGH)",
        pins::ERROR_OUTPUT
    );

    info!("Carvera Spindle Controller starting...");

    // Safe startup delay
    Timer::after(Duration::from_millis(config::STARTUP_DELAY_MS)).await;
    info!("Startup delay complete ({}ms)", config::STARTUP_DELAY_MS);

    // Initialize display data channel
    init_display_data();
    info!("Display data channel initialized");

    // === Initialize Watchdog ===
    let watchdog = Watchdog::new(p.WATCHDOG);
    info!("Watchdog initialized");

    // === PWM Output Setup (GPIO4 -> ESCON DIN1) ===
    let mut out_cfg = PwmConfig::default();
    out_cfg.top = PWM_OUT_TOP;
    out_cfg.divider = PWM_OUT_DIVIDER.into();
    out_cfg.compare_a = 100; // 10% initial
    out_cfg.enable = true;
    let pwm_out = Pwm::new_output_a(p.PWM_SLICE2, p.PIN_4, out_cfg);
    info!(
        "PWM output configured on GPIO{} (freq={}Hz range={}-{}%)",
        pins::PWM_OUTPUT,
        config::PWM_OUTPUT_FREQ_HZ,
        config::PWM_MIN_DUTY / 10,
        config::PWM_MAX_DUTY / 10
    );

    // === Enable Pin Setup (GPIO5 -> ESCON DIN2) ===
    let enable_pin = Output::new(p.PIN_5, Level::Low);
    info!("Enable pin configured on GPIO{}", pins::ENABLE);

    // === ESCON Alert Input (GPIO8) ===
    let escon_alert = Input::new(p.PIN_8, Pull::Up);
    info!("ESCON alert input configured on GPIO{}", pins::ESCON_ALERT);

    // === PIO Setup for Speed Input (GPIO9) and PWM Input (GPIO3) ===
    // Using PIO state machines for:
    // - SM0: Speed measurement (ESCON DOUT, hardware debounced)
    // - SM1: PWM duty cycle measurement (Carvera input, cycle-aligned)
    let Pio {
        common: mut pio_common,
        sm0: mut pio_sm0,
        sm1: pio_sm1,
        irq0: pio_irq0,
        ..
    } = Pio::new(p.PIO0, PioIrqs);

    // Setup speed measurement (ESCON DOUT is open-drain: needs Pull::Up)
    setup_speed_measure_pio(&mut pio_common, &mut pio_sm0, p.PIN_9);
    info!(
        "Speed input configured on GPIO{} (PIO hardware debounced)",
        pins::SPEED_INPUT
    );

    // Setup PWM input measurement (Carvera PWM)
    let mut pio_sm1 = pio_sm1;
    setup_pwm_input_pio(&mut pio_common, &mut pio_sm1, p.PIN_3);
    info!(
        "PWM input configured on GPIO{} (PIO cycle-aligned)",
        pins::PWM_INPUT
    );

    // Note: error_out (GPIO10) was initialized at startup before delay

    // === LCD Display Setup (HD44780 16x2 with RGB backlight) ===
    // Construct GPIO outputs for LCD data/control
    let lcd_rs = Output::new(p.PIN_16, Level::Low);
    let lcd_e = Output::new(p.PIN_17, Level::Low);
    let lcd_d4 = Output::new(p.PIN_18, Level::Low);
    let lcd_d5 = Output::new(p.PIN_22, Level::Low); // Moved from GPIO 19 (was stuck HIGH)
    let lcd_d6 = Output::new(p.PIN_20, Level::Low);
    let lcd_d7 = Output::new(p.PIN_21, Level::Low);

    // Construct PWM for RGB backlight (common anode - inverted)
    let lcd_red_pwm = Pwm::new_output_a(p.PWM_SLICE7, p.PIN_14, Default::default());
    let lcd_gb_pwm = Pwm::new_output_ab(p.PWM_SLICE6, p.PIN_12, p.PIN_13, Default::default());

    info!(
        "LCD configured: RS={} E={} D4-D7={},{},{},{} RGB={},{},{}",
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

    // === ADC Setup for Current Monitoring (GPIO26) + Temperature Sensor ===
    let adc = Adc::new(p.ADC, AdcIrqs, AdcConfig::default());
    let current_channel = Channel::new_pin(p.PIN_26, Pull::None);
    let temp_channel = Channel::new_temp_sensor(p.ADC_TEMP_SENSOR);
    info!(
        "ADC configured for current monitoring on GPIO{} + internal temp sensor",
        pins::CURRENT_ADC
    );

    // === Flash for Calibration Storage ===
    let flash: Flash<'_, _, embassy_rp::flash::Blocking, { 4 * 1024 * 1024 }> =
        Flash::new_blocking(p.FLASH);
    info!("Flash configured for calibration storage (4MB, blocking)");

    // === Status LED (GPIO25) ===
    let led = Output::new(p.PIN_25, Level::Low);
    info!("Status LED configured on GPIO{}", pins::STATUS_LED);

    info!(
        "Motor config: max_rpm={} belt_ratio={}.{} carvera_max_rpm={} ppr={}",
        config::MAX_RPM,
        config::BELT_RATIO_X1000 / 1000,
        config::BELT_RATIO_X1000 % 1000,
        config::CARVERA_SPINDLE_MAX_RPM,
        config::ESCON_PULSES_PER_REV
    );

    // === Spawn Tasks ===
    info!("Spawning tasks...");

    // Speed measurement task (PIO hardware debounced period measurement)
    match speed_measure_task(pio_sm0, pio_irq0) {
        Ok(token) => {
            spawner.spawn(token);
            info!("Speed measurement task spawned (PIO debounced)");
        }
        Err(e) => error!("speed_measure_task unavailable: {:?}", Debug2Format(&e)),
    }

    // PWM input measurement task (PIO cycle-aligned duty measurement)
    match pwm_input_task(pio_sm1) {
        Ok(token) => {
            spawner.spawn(token);
            info!("PWM input task spawned (PIO cycle-aligned)");
        }
        Err(e) => error!("pwm_input_task unavailable: {:?}", Debug2Format(&e)),
    }

    // Spindle control task (reads duty from MEASURED_DUTY, RPM from MEASURED_RPM)
    match spindle_control_task(pwm_out, enable_pin, error_out, escon_alert) {
        Ok(token) => {
            spawner.spawn(token);
            info!("Spindle control task spawned");
        }
        Err(e) => error!("spindle_control_task unavailable: {:?}", Debug2Format(&e)),
    }

    match watchdog_task(watchdog) {
        Ok(token) => {
            spawner.spawn(token);
            info!("Watchdog task spawned");
        }
        Err(e) => error!("watchdog_task unavailable: {:?}", Debug2Format(&e)),
    }

    // Current monitor task (reads ESCON analog output via ADC + internal temp sensor)
    match current_monitor_task(adc, current_channel, temp_channel) {
        Ok(token) => {
            spawner.spawn(token);
            info!("Current monitor task spawned");
        }
        Err(e) => error!("current_monitor_task unavailable: {:?}", Debug2Format(&e)),
    }

    // LCD display task
    use carvera_spindle::tasks::LcdPeripherals;
    let lcd_peripherals = LcdPeripherals {
        rs: lcd_rs,
        e: lcd_e,
        d4: lcd_d4,
        d5: lcd_d5,
        d6: lcd_d6,
        d7: lcd_d7,
        red_pwm: lcd_red_pwm,
        green_blue_pwm: lcd_gb_pwm,
    };
    match lcd_task(lcd_peripherals) {
        Ok(token) => {
            spawner.spawn(token);
            info!("LCD task spawned");
        }
        Err(e) => error!("lcd_task unavailable: {:?}", Debug2Format(&e)),
    }

    // Calibration task (flash-based calibration storage and sequence detection)
    match calibration_task(flash) {
        Ok(token) => {
            spawner.spawn(token);
            info!("Calibration task spawned");
        }
        Err(e) => error!("calibration_task unavailable: {:?}", Debug2Format(&e)),
    }

    match led_task(led) {
        Ok(token) => {
            spawner.spawn(token);
            info!("LED task spawned");
        }
        Err(e) => error!("led_task unavailable: {:?}", Debug2Format(&e)),
    }

    info!("All tasks spawned. System running.");
}

//! HD44780 LCD hardware driver and RGB backlight controller (embedded only).
//!
//! Async driver for 4-bit parallel interface HD44780 LCD with common-anode RGB backlight.

use embassy_rp::gpio::Output;
use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_time::{Duration, Timer};

use super::{BacklightColor, SPEED_ICON};

// ============================================================================
// LCD Driver
// ============================================================================

pub struct Hd44780<'a> {
    rs: Output<'a>,
    e: Output<'a>,
    d4: Output<'a>,
    d5: Output<'a>,
    d6: Output<'a>,
    d7: Output<'a>,
}

impl<'a> Hd44780<'a> {
    /// Create a new HD44780 LCD driver
    pub fn new(
        rs: Output<'a>,
        e: Output<'a>,
        d4: Output<'a>,
        d5: Output<'a>,
        d6: Output<'a>,
        d7: Output<'a>,
    ) -> Self {
        Self {
            rs,
            e,
            d4,
            d5,
            d6,
            d7,
        }
    }

    /// Initialize the LCD in 4-bit mode (per HD44780 datasheet)
    pub async fn init(&mut self) {
        // Power-up delay (HD44780 datasheet: 40ms; 50ms provides margin)
        Timer::after(Duration::from_millis(50)).await;

        // Reset sequence: 0x03 three times
        self.write_nibble(0x03).await;
        Timer::after(Duration::from_millis(5)).await;
        self.write_nibble(0x03).await;
        Timer::after(Duration::from_micros(150)).await;
        self.write_nibble(0x03).await;
        Timer::after(Duration::from_micros(150)).await;

        // Switch to 4-bit mode (datasheet: ~40us; 100us provides margin)
        self.write_nibble(0x02).await;
        Timer::after(Duration::from_micros(100)).await;

        // Configure: 4-bit, 2-line, 5x8 font
        self.command(0x28).await;
        self.command(0x08).await; // Display OFF
        self.command(0x01).await; // Clear
        Timer::after(Duration::from_millis(2)).await;
        self.command(0x06).await; // Entry mode: increment
        self.command(0x0C).await; // Display ON, cursor off

        // Define custom character 0 (speed icon)
        self.command(0x40).await; // CGRAM address 0
        for &row in SPEED_ICON.iter() {
            self.data(row).await;
        }
        self.command(0x80).await; // Return to DDRAM
    }

    /// Write a command byte
    pub async fn command(&mut self, cmd: u8) {
        self.rs.set_low();
        Timer::after(Duration::from_micros(1)).await;
        self.write_byte(cmd).await;
    }

    /// Write a data byte
    pub async fn data(&mut self, data: u8) {
        self.rs.set_high();
        Timer::after(Duration::from_micros(1)).await;
        self.write_byte(data).await;
    }

    /// Write a line of text at the specified row (0 or 1)
    pub async fn write_line(&mut self, row: u8, text: &[u8]) {
        let addr = if row == 0 { 0x80 } else { 0xC0 };
        self.command(addr).await;
        for &c in text.iter().take(16) {
            self.data(c).await;
        }
    }

    /// Clear the display
    pub async fn clear(&mut self) {
        self.command(0x01).await;
        Timer::after(Duration::from_millis(2)).await;
    }

    async fn write_byte(&mut self, byte: u8) {
        self.write_nibble(byte >> 4).await;
        self.write_nibble(byte & 0x0F).await;
    }

    async fn write_nibble(&mut self, nibble: u8) {
        // Set data lines
        if nibble & 0x01 != 0 {
            self.d4.set_high();
        } else {
            self.d4.set_low();
        }
        if nibble & 0x02 != 0 {
            self.d5.set_high();
        } else {
            self.d5.set_low();
        }
        if nibble & 0x04 != 0 {
            self.d6.set_high();
        } else {
            self.d6.set_low();
        }
        if nibble & 0x08 != 0 {
            self.d7.set_high();
        } else {
            self.d7.set_low();
        }

        // Pulse enable (datasheet: 37us typical; 50us provides margin)
        Timer::after(Duration::from_micros(1)).await;
        self.e.set_high();
        Timer::after(Duration::from_micros(1)).await;
        self.e.set_low();
        Timer::after(Duration::from_micros(50)).await;
    }
}

// ============================================================================
// RGB Backlight Controller
// ============================================================================

pub struct RgbBacklight<'a> {
    red: Pwm<'a>,
    green_blue: Pwm<'a>,
}

impl<'a> RgbBacklight<'a> {
    /// Create a new RGB backlight controller
    /// - red: PWM slice for red LED (channel A)
    /// - green_blue: PWM slice for green (channel A) and blue (channel B)
    ///
    /// Initializes with backlight OFF to prevent bright flash on startup.
    /// (Common anode: default PWM duty of 0 = full brightness)
    pub fn new(red: Pwm<'a>, green_blue: Pwm<'a>) -> Self {
        let mut backlight = Self { red, green_blue };
        backlight.set_color(&BacklightColor::OFF);
        backlight
    }

    /// Set backlight color
    /// NOTE: Common anode LCD - PWM is INVERTED (low duty = bright, high duty = off)
    pub fn set_color(&mut self, color: &BacklightColor) {
        // Convert 0-255 to PWM duty (0-65535), then INVERT for common anode
        let r_duty = 65535 - (color.r as u16) * 257;
        let g_duty = 65535 - (color.g as u16) * 257;
        let b_duty = 65535 - (color.b as u16) * 257;

        // Red is on SLICE7 channel A
        let mut red_cfg = PwmConfig::default();
        red_cfg.top = 65535;
        red_cfg.compare_a = r_duty;
        self.red.set_config(&red_cfg);

        // Green (A) and Blue (B) share SLICE6
        let mut gb_cfg = PwmConfig::default();
        gb_cfg.top = 65535;
        gb_cfg.compare_a = g_duty;
        gb_cfg.compare_b = b_duty;
        self.green_blue.set_config(&gb_cfg);
    }
}

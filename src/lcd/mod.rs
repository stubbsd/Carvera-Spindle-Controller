//! HD44780 16x2 LCD driver with RGB backlight.
//!
//! Split into:
//! - `format` - Pure formatting functions (host-testable)
//! - `driver` - HD44780 hardware driver and RGB backlight (embedded-only)

#[cfg(feature = "embedded")]
pub mod driver;
pub mod format;

// Re-export public API so callers can use crate::lcd::* unchanged
#[cfg(feature = "embedded")]
pub use driver::{Hd44780, RgbBacklight};
pub use format::{
    Status, calculate_backlight, calculate_deviation, format_cal_aborted, format_cal_cleared,
    format_cal_complete, format_cal_detect, format_cal_line1, format_cal_line2, format_error_lines,
    format_line1, format_line2, format_no_cal_warning,
};

// ============================================================================
// RGB Backlight Color
// ============================================================================

/// RGB backlight color (0-255 per channel)
#[derive(Clone, Copy, Debug, Default)]
pub struct BacklightColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl BacklightColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const OFF: Self = Self::new(0, 0, 0);
    pub const DIM_GREEN: Self = Self::new(0, 15, 0);
    pub const BRIGHT_GREEN: Self = Self::new(0, 255, 0);
    pub const YELLOW: Self = Self::new(255, 255, 0);
    pub const ORANGE: Self = Self::new(255, 128, 0);
    pub const RED: Self = Self::new(255, 0, 0);
    pub const BLUE: Self = Self::new(0, 0, 255);
    pub const CYAN: Self = Self::new(0, 255, 255);
    pub const MAGENTA: Self = Self::new(255, 0, 255);
    pub const DIM_BLUE: Self = Self::new(0, 0, 30);
    pub const WHITE: Self = Self::new(255, 255, 255);
}

// ============================================================================
// Custom Character - Speed Icon
// ============================================================================

/// Speed icon for HD44780 custom character slot 0
/// Tachometer/gauge shape (5x8 pixels)
pub const SPEED_ICON: [u8; 8] = [
    0b11111, // Row 0
    0b11111, // Row 1
    0b01110, // Row 2
    0b00100, // Row 3
    0b00100, // Row 4
    0b00100, // Row 5
    0b00000, // Row 6
    0b00000, // Row 7
];

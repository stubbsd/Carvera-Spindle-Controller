//! Flash persistence for calibration data.
//!
//! Stores and retrieves calibration tables in the last 4KB sector of flash.
//! Serialization/deserialization functions are host-testable (pure, no-std).
//! Flash I/O functions are embedded-only (embassy-rp Flash API).
//!
//! ## Flash Layout v2 (4096 bytes)
//!
//! ```text
//! Bytes 0-3:   Magic (0x43414C32 = "CAL2")
//! Byte  4:     Version (2)
//! Byte  5:     Reserved (0)
//! Bytes 6-7:   Count as u16 LE (1-386)
//! Bytes 8-9:   CRC-16 checksum of points data
//! Bytes 10+:   Points array (count × 4 bytes, packed: rpm_le16 | duty_le16)
//! ```

use crate::calibration::{CAL_STEPS, CalibrationPoint, CalibrationTable};

// ============================================================================
// Constants
// ============================================================================

/// Magic bytes: "CAL2" as ASCII
const MAGIC_BYTES: [u8; 4] = [0x43, 0x41, 0x4C, 0x32]; // "CAL2"

/// Current format version
const VERSION: u8 = 2;

/// Header size in bytes (magic:4 + version:1 + reserved:1 + count:2 + crc:2 = 10)
const HEADER_SIZE: usize = 10;

/// Flash sector size
pub const SECTOR_SIZE: usize = 4096;

/// Offset within 4MB flash for calibration storage (last sector)
pub const FLASH_OFFSET: u32 = 0x3F_F000;

// ============================================================================
// CRC-16 (CCITT) — small, no-dependency implementation
// ============================================================================

/// Compute CRC-16/CCITT over a byte slice.
/// Polynomial: 0x1021, Init: 0xFFFF
pub fn compute_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

// ============================================================================
// Serialization (pure, host-testable)
// ============================================================================

/// Serialize a calibration table into a 4096-byte flash buffer.
pub fn serialize_calibration(table: &CalibrationTable, buf: &mut [u8; SECTOR_SIZE]) {
    // Clear buffer
    buf.fill(0xFF); // Flash erased state

    // Magic
    buf[0..4].copy_from_slice(&MAGIC_BYTES);

    // Version
    buf[4] = VERSION;

    // Reserved
    buf[5] = 0;

    // Count as u16 LE
    let count_bytes = table.count.to_le_bytes();
    buf[6] = count_bytes[0];
    buf[7] = count_bytes[1];

    // Points data (after header, starting at byte 10)
    let points_data = &mut buf[HEADER_SIZE..];
    for i in 0..table.count as usize {
        let offset = i * 4;
        let rpm_bytes = table.points[i].expected_rpm.to_le_bytes();
        let duty_bytes = table.points[i].measured_duty.to_le_bytes();
        points_data[offset] = rpm_bytes[0];
        points_data[offset + 1] = rpm_bytes[1];
        points_data[offset + 2] = duty_bytes[0];
        points_data[offset + 3] = duty_bytes[1];
    }

    // CRC-16 over points data only
    let data_len = table.count as usize * 4;
    let crc = compute_crc16(&buf[HEADER_SIZE..HEADER_SIZE + data_len]);
    let crc_bytes = crc.to_le_bytes();
    buf[8] = crc_bytes[0];
    buf[9] = crc_bytes[1];
}

/// Deserialize a calibration table from a 4096-byte flash buffer.
///
/// Validates magic, version, count range, and CRC-16 checksum.
/// Returns `None` if any validation fails (including old "CAL1" format).
pub fn deserialize_calibration(buf: &[u8; SECTOR_SIZE]) -> Option<CalibrationTable> {
    // Check magic (rejects old "CAL1" format gracefully)
    if buf[0..4] != MAGIC_BYTES {
        return None;
    }

    // Check version
    if buf[4] != VERSION {
        return None;
    }

    // Check count (u16 LE at bytes 6-7)
    let count = u16::from_le_bytes([buf[6], buf[7]]);
    if count == 0 || count as usize > CAL_STEPS {
        return None;
    }

    // Verify CRC (at bytes 8-9)
    let data_len = count as usize * 4;
    let stored_crc = u16::from_le_bytes([buf[8], buf[9]]);
    let computed_crc = compute_crc16(&buf[HEADER_SIZE..HEADER_SIZE + data_len]);
    if stored_crc != computed_crc {
        return None;
    }

    // Parse points
    let mut table = CalibrationTable {
        count,
        ..CalibrationTable::default()
    };

    let points_data = &buf[HEADER_SIZE..];
    for i in 0..count as usize {
        let offset = i * 4;
        let expected_rpm = u16::from_le_bytes([points_data[offset], points_data[offset + 1]]);
        let measured_duty = u16::from_le_bytes([points_data[offset + 2], points_data[offset + 3]]);
        table.points[i] = CalibrationPoint {
            expected_rpm,
            measured_duty,
        };
    }

    Some(table)
}

/// Compute checksum for display/logging.
pub fn compute_checksum(table: &CalibrationTable) -> u16 {
    let mut data = [0u8; CAL_STEPS * 4];
    for i in 0..table.count as usize {
        let offset = i * 4;
        let rpm_bytes = table.points[i].expected_rpm.to_le_bytes();
        let duty_bytes = table.points[i].measured_duty.to_le_bytes();
        data[offset] = rpm_bytes[0];
        data[offset + 1] = rpm_bytes[1];
        data[offset + 2] = duty_bytes[0];
        data[offset + 3] = duty_bytes[1];
    }
    compute_crc16(&data[..table.count as usize * 4])
}

// ============================================================================
// Flash I/O (embedded-only)
// ============================================================================

#[cfg(feature = "embedded")]
pub use embedded::*;

#[cfg(feature = "embedded")]
mod embedded {
    use super::*;
    use core::cell::UnsafeCell;
    use embassy_rp::flash::{self, Blocking, Flash};
    use embassy_rp::peripherals::FLASH;

    /// 4MB flash size for RP2350
    pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

    /// Wrapper for static buffer requiring interior mutability.
    /// Safety: only accessed from the calibration task (single writer, never concurrent).
    struct FlashBuf(UnsafeCell<[u8; SECTOR_SIZE]>);
    unsafe impl Sync for FlashBuf {}

    static FLASH_BUF: FlashBuf = FlashBuf(UnsafeCell::new([0u8; SECTOR_SIZE]));

    /// Read calibration table from flash.
    pub fn read_calibration(
        flash: &mut Flash<'_, FLASH, Blocking, FLASH_SIZE>,
    ) -> Option<CalibrationTable> {
        // Safety: single-writer access from calibration task only
        let buf = unsafe { &mut *FLASH_BUF.0.get() };

        if flash.blocking_read(FLASH_OFFSET, &mut buf[..]).is_err() {
            defmt::error!("Flash read failed at offset 0x{:X}", FLASH_OFFSET);
            return None;
        }

        deserialize_calibration(buf)
    }

    /// Erase calibration data from flash (writes all 0xFF).
    pub fn erase_calibration(
        flash: &mut Flash<'_, FLASH, Blocking, FLASH_SIZE>,
    ) -> Result<(), flash::Error> {
        flash.blocking_erase(FLASH_OFFSET, FLASH_OFFSET + SECTOR_SIZE as u32)?;
        defmt::info!(
            "Calibration erased from flash at offset 0x{:X}",
            FLASH_OFFSET
        );
        Ok(())
    }

    /// Write calibration table to flash.
    pub fn write_calibration(
        flash: &mut Flash<'_, FLASH, Blocking, FLASH_SIZE>,
        table: &CalibrationTable,
    ) -> Result<(), flash::Error> {
        // Safety: single-writer access from calibration task only
        let buf = unsafe { &mut *FLASH_BUF.0.get() };

        serialize_calibration(table, buf);

        // Erase sector first
        flash.blocking_erase(FLASH_OFFSET, FLASH_OFFSET + SECTOR_SIZE as u32)?;

        // Write data
        flash.blocking_write(FLASH_OFFSET, &buf[..])?;

        defmt::info!(
            "Calibration written to flash at offset 0x{:X}",
            FLASH_OFFSET
        );
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_table() -> CalibrationTable {
        let mut table = CalibrationTable::default();
        table.count = 3;
        table.points[0] = CalibrationPoint {
            expected_rpm: 500,
            measured_duty: 245,
        };
        table.points[1] = CalibrationPoint {
            expected_rpm: 750,
            measured_duty: 367,
        };
        table.points[2] = CalibrationPoint {
            expected_rpm: 1000,
            measured_duty: 489,
        };
        table
    }

    #[test]
    fn test_crc16_basic() {
        let data = b"Hello";
        let crc = compute_crc16(data);
        // Just verify it's deterministic and non-zero
        assert_ne!(crc, 0);
        assert_eq!(crc, compute_crc16(data));
    }

    #[test]
    fn test_crc16_different_data() {
        let crc1 = compute_crc16(b"Hello");
        let crc2 = compute_crc16(b"World");
        assert_ne!(crc1, crc2);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let table = make_test_table();
        let mut buf = [0u8; SECTOR_SIZE];

        serialize_calibration(&table, &mut buf);
        let result = deserialize_calibration(&buf).expect("Deserialization should succeed");

        assert_eq!(result.count, 3);
        assert_eq!(result.points[0].expected_rpm, 500);
        assert_eq!(result.points[0].measured_duty, 245);
        assert_eq!(result.points[1].expected_rpm, 750);
        assert_eq!(result.points[1].measured_duty, 367);
        assert_eq!(result.points[2].expected_rpm, 1000);
        assert_eq!(result.points[2].measured_duty, 489);
    }

    #[test]
    fn test_deserialize_bad_magic() {
        let table = make_test_table();
        let mut buf = [0u8; SECTOR_SIZE];
        serialize_calibration(&table, &mut buf);

        // Corrupt magic
        buf[0] = 0x00;
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_deserialize_bad_version() {
        let table = make_test_table();
        let mut buf = [0u8; SECTOR_SIZE];
        serialize_calibration(&table, &mut buf);

        // Wrong version
        buf[4] = 99;
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_deserialize_bad_crc() {
        let table = make_test_table();
        let mut buf = [0u8; SECTOR_SIZE];
        serialize_calibration(&table, &mut buf);

        // Corrupt a data byte
        buf[HEADER_SIZE] ^= 0xFF;
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_deserialize_zero_count() {
        let mut buf = [0u8; SECTOR_SIZE];
        buf[0..4].copy_from_slice(&MAGIC_BYTES);
        buf[4] = VERSION;
        buf[5] = 0; // reserved
        buf[6..8].copy_from_slice(&0u16.to_le_bytes()); // Zero count should fail
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_deserialize_count_too_high() {
        let mut buf = [0u8; SECTOR_SIZE];
        buf[0..4].copy_from_slice(&MAGIC_BYTES);
        buf[4] = VERSION;
        buf[5] = 0; // reserved
        buf[6..8].copy_from_slice(&400u16.to_le_bytes()); // > CAL_STEPS should fail
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_deserialize_erased_flash() {
        // Erased flash is all 0xFF
        let buf = [0xFF; SECTOR_SIZE];
        assert!(deserialize_calibration(&buf).is_none());
    }

    #[test]
    fn test_compute_checksum_matches() {
        let table = make_test_table();
        let mut buf = [0u8; SECTOR_SIZE];
        serialize_calibration(&table, &mut buf);

        let stored_crc = u16::from_le_bytes([buf[8], buf[9]]);
        let computed_crc = compute_checksum(&table);
        assert_eq!(stored_crc, computed_crc);
    }

    #[test]
    fn test_full_386_point_roundtrip() {
        let mut table = CalibrationTable::default();
        table.count = CAL_STEPS as u16;
        for i in 0..CAL_STEPS {
            let rpm = 750 + i as u16 * 50;
            let duty = ((rpm as u32 * 10000) / 20437) as u16;
            table.points[i] = CalibrationPoint {
                expected_rpm: rpm,
                measured_duty: duty,
            };
        }

        let mut buf = [0u8; SECTOR_SIZE];
        serialize_calibration(&table, &mut buf);
        let result = deserialize_calibration(&buf).expect("386-point roundtrip should work");

        assert_eq!(result.count, CAL_STEPS as u16);
        for i in 0..CAL_STEPS {
            assert_eq!(result.points[i].expected_rpm, table.points[i].expected_rpm);
            assert_eq!(
                result.points[i].measured_duty,
                table.points[i].measured_duty
            );
        }

        // Verify fits in sector: header(10) + 386*4 = 1554 < 4096
        let total = HEADER_SIZE + CAL_STEPS * 4;
        assert!(
            total <= SECTOR_SIZE,
            "Data size {} exceeds sector {}",
            total,
            SECTOR_SIZE
        );
    }

    #[test]
    fn test_rejects_old_cal1_format() {
        // Old CAL1 magic should be rejected
        let mut buf = [0u8; SECTOR_SIZE];
        buf[0..4].copy_from_slice(&[0x43, 0x41, 0x4C, 0x31]); // "CAL1"
        buf[4] = 1; // old version
        buf[5] = 3; // old count field
        assert!(deserialize_calibration(&buf).is_none());
    }
}

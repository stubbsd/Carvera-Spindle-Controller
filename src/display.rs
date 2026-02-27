//! Display types for inter-task communication.
//!
//! Shared data structures used by the spindle control task to publish
//! status updates and by the LCD display task to render them.

/// Error type for display and fault reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorType {
    None,
    Stall,
    /// Stall was detected but alert has been released (spindle stopped for 2s+)
    StallCleared,
    Overcurrent,
    EsconAlert,
    Thermal,
}

/// Display status data
#[derive(Clone, Copy)]
pub struct DisplayStatus {
    pub requested_rpm: u32,
    pub actual_rpm: u32,
    pub current_ma: u32,
    pub error: bool,
    pub error_type: ErrorType,
    pub enabled: bool,
    /// Time in ms to reach stable speed (shown briefly after stabilization)
    pub stabilization_time_ms: Option<u32>,
}

impl Default for DisplayStatus {
    fn default() -> Self {
        Self {
            requested_rpm: 0,
            actual_rpm: 0,
            current_ma: 0,
            error: false,
            error_type: ErrorType::None,
            enabled: false,
            stabilization_time_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_status_default() {
        // Documents the contract: default DisplayStatus represents idle spindle
        let status = DisplayStatus::default();
        assert_eq!(status.requested_rpm, 0);
        assert_eq!(status.actual_rpm, 0);
        assert_eq!(status.current_ma, 0);
        assert!(!status.error);
        assert_eq!(status.error_type, ErrorType::None);
        assert!(!status.enabled);
        assert!(status.stabilization_time_ms.is_none());
    }
}

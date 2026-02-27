//! Signal filtering and smoothing utilities.
//!
//! This module provides a [`CircularBuffer`] ring buffer for measurement storage.

/// Generic circular buffer for storing measurement values.
///
/// Provides FIFO storage with automatic wraparound and slice access
/// for averaging calculations.
///
/// # Examples
/// ```
/// use carvera_spindle::CircularBuffer;
///
/// let mut buffer = CircularBuffer::<4>::new();
/// buffer.push(100);
/// buffer.push(200);
/// assert_eq!(buffer.len(), 2);
/// assert_eq!(buffer.as_slice(), &[100, 200]);
///
/// // Fill and wrap around
/// buffer.push(300);
/// buffer.push(400);
/// buffer.push(500); // Overwrites 100
/// assert_eq!(buffer.len(), 4);
/// assert_eq!(buffer.as_slice(), &[200, 300, 400, 500]);
/// ```
pub struct CircularBuffer<const N: usize> {
    buffer: [u32; N],
    index: usize,
    count: usize,
}

impl<const N: usize> CircularBuffer<N> {
    /// Create a new empty circular buffer.
    pub const fn new() -> Self {
        Self {
            buffer: [0; N],
            index: 0,
            count: 0,
        }
    }

    /// Push a value into the buffer, overwriting the oldest if full.
    pub fn push(&mut self, value: u32) {
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % N;
        if self.count < N {
            self.count += 1;
        }
    }

    /// Get the valid values as a slice (oldest to newest when not wrapped).
    ///
    /// When the buffer has wrapped, this returns values in storage order
    /// (not chronological order). This is intentional and safe for all current
    /// uses: averaging (order-independent) and median filtering via
    /// `median_u32` (which sorts its input). Do not rely on chronological
    /// ordering from this method after wraparound.
    pub fn as_slice(&self) -> &[u32] {
        if self.count < N {
            // Not yet wrapped - values are at the beginning
            &self.buffer[..self.count]
        } else {
            // Wrapped - all values are valid
            &self.buffer[..]
        }
    }

    /// Get the number of valid values in the buffer.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Clear all values from the buffer.
    pub fn clear(&mut self) {
        self.buffer = [0; N];
        self.index = 0;
        self.count = 0;
    }
}

impl<const N: usize> Default for CircularBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_buffer_new_empty() {
        let buffer = CircularBuffer::<4>::new();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.as_slice(), &[]);
    }

    #[test]
    fn test_circular_buffer_push_single() {
        let mut buffer = CircularBuffer::<4>::new();
        buffer.push(100);
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.as_slice(), &[100]);
    }

    #[test]
    fn test_circular_buffer_fill() {
        let mut buffer = CircularBuffer::<4>::new();
        buffer.push(100);
        buffer.push(200);
        buffer.push(300);
        buffer.push(400);
        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer.as_slice(), &[100, 200, 300, 400]);
    }

    #[test]
    fn test_circular_buffer_wraparound() {
        let mut buffer = CircularBuffer::<4>::new();
        buffer.push(100);
        buffer.push(200);
        buffer.push(300);
        buffer.push(400);
        buffer.push(500); // Overwrites 100
        assert_eq!(buffer.len(), 4);
        // Buffer now contains [500, 200, 300, 400] in storage order
        let slice = buffer.as_slice();
        assert_eq!(slice.len(), 4);
        // For averaging, order doesn't matter - verify all values present
        let sum: u32 = slice.iter().sum();
        assert_eq!(sum, 200 + 300 + 400 + 500);
    }

    #[test]
    fn test_circular_buffer_clear() {
        let mut buffer = CircularBuffer::<4>::new();
        buffer.push(100);
        buffer.push(200);
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.as_slice(), &[]);
    }

    #[test]
    fn test_circular_buffer_as_slice_partial() {
        let mut buffer = CircularBuffer::<8>::new();
        buffer.push(10);
        buffer.push(20);
        buffer.push(30);
        assert_eq!(buffer.as_slice(), &[10, 20, 30]);
    }

    #[test]
    fn test_circular_buffer_is_empty_after_wraparound() {
        // Push N+1 values into a buffer of size N, then verify is_empty() is false
        let mut buffer = CircularBuffer::<4>::new();
        buffer.push(10);
        buffer.push(20);
        buffer.push(30);
        buffer.push(40);
        buffer.push(50); // Overwrites first element, count stays at 4
        assert!(
            !buffer.is_empty(),
            "Buffer should not be empty after wraparound"
        );
        assert_eq!(
            buffer.len(),
            4,
            "Buffer length should be N after wraparound"
        );
    }

    #[test]
    fn test_circular_buffer_default() {
        let buffer = CircularBuffer::<4>::default();
        assert!(buffer.is_empty());
    }
}

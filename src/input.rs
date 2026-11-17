//! Turning received bytes into lines.
//!
//! The serial port is polled, so bytes arrive one at a time and the edge cases
//! (Enter, backspace, a line longer than the buffer) are decided here, away from
//! the hardware. That also makes them testable without a terminal.

/// Longest line kept. Further bytes are dropped until the next line ends, so a
/// pasted paragraph cannot grow the buffer at run time.
pub const LINE_CAPACITY: usize = 64;

/// The bytes typed since the last Enter.
pub struct Line {
    buf: [u8; LINE_CAPACITY],
    len: usize,
}

impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}

impl Line {
    pub const fn new() -> Self {
        Self {
            buf: [0; LINE_CAPACITY],
            len: 0,
        }
    }

    /// Feeds one received byte.
    ///
    /// Returns the finished line, without the Enter, when `byte` ends it. An
    /// empty line returns `None` too: there is nothing to hand over.
    pub fn push(&mut self, byte: u8) -> Option<&[u8]> {
        match byte {
            b'\r' | b'\n' => {
                if self.len == 0 {
                    return None;
                }
                let text = &self.buf[..self.len];
                self.len = 0;
                Some(text)
            }
            // Backspace and delete take one byte off the end.
            0x08 | 0x7f => {
                self.len = self.len.saturating_sub(1);
                None
            }
            _ => {
                if self.len < LINE_CAPACITY {
                    self.buf[self.len] = byte;
                    self.len += 1;
                }
                None
            }
        }
    }
}

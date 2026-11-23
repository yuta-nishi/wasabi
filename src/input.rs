/// Longest line kept. Further bytes are dropped until the next line ends, so a
/// pasted paragraph cannot grow the buffer at run time.
pub const LINE_CAPACITY: usize = 64;

/// The bytes typed since the last Enter.
pub struct Line {
    bytes: [u8; LINE_CAPACITY],
    length: usize,
}

impl Default for Line {
    fn default() -> Self {
        Self::new()
    }
}

impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; LINE_CAPACITY],
            length: 0,
        }
    }

    /// Feeds one received byte.
    ///
    /// Returns the finished line, without the Enter, when `byte` ends it. An
    /// empty line returns `None` too: there is nothing to hand over.
    pub fn push(&mut self, byte: u8) -> Option<&[u8]> {
        match byte {
            b'\r' | b'\n' => {
                if self.length == 0 {
                    return None;
                }
                let text = &self.bytes[..self.length];
                self.length = 0;
                Some(text)
            }
            // Backspace and delete take one byte off the end.
            0x08 | 0x7f => {
                self.length = self.length.saturating_sub(1);
                None
            }
            _ => {
                if self.length < LINE_CAPACITY {
                    self.bytes[self.length] = byte;
                    self.length += 1;
                }
                None
            }
        }
    }
}

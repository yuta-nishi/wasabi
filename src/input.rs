use crate::executor;
use crate::info;
use crate::mutex::Mutex;
use crate::println;
use crate::serial::Serial;
use crate::timer;
use core::time::Duration;

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

/// The line both input sources feed.
static CONSOLE: Mutex<Line> = Mutex::new(Line::new());

/// Adds one byte to the console line; a finished line is logged.
pub fn push(byte: u8) {
    let mut finished = [0u8; LINE_CAPACITY];
    let mut length = 0;
    {
        let mut line = CONSOLE.lock();
        if let Some(text) = line.push(byte) {
            length = text.len();
            finished[..length].copy_from_slice(text);
        }
    }
    if length > 0 {
        let text = core::str::from_utf8(&finished[..length]).unwrap_or("<not utf-8>");
        info!("input: \"{text}\"");
    } else if byte == b'\r' || byte == b'\n' {
        // An empty line still gets its own newline.
        println!();
    }
}

/// Polls the serial port and logs each line once Enter arrives.
///
/// `Serial::try_read` only touches the data register when the line status says a
/// byte is waiting, so this task polls with `sleep` and the other tasks keep
/// their turns.
pub fn spawn_console() {
    executor::spawn(async {
        let mut serial = Serial;
        // QEMU's window and the terminal both take keys: only the terminal's
        // reach COM1, and only while no key is held by the window.
        println!("serial console: type in this terminal, then press Enter");
        loop {
            while let Some(byte) = serial.try_read() {
                push(byte);
            }
            timer::sleep(Duration::from_millis(10)).await;
        }
    });
}

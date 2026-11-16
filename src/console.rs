//! Where `print!` and `println!` go: the serial port, and the screen once it is
//! available.
//!
//! Formatting happens once; the resulting string chunks are forwarded to both
//! sinks, so the two outputs never disagree.

use crate::graphics::draw_font_fg;
use crate::mutex::Mutex;
use crate::serial::Serial;
use crate::uefi::VramBufferInfo;
use core::fmt;
use core::fmt::Write;

/// The screen and its cursor, once `init_screen` has installed it. Before that,
/// printing only reaches the serial port. The framebuffer is kept by value, so
/// nothing has to be allocated before the heap exists.
struct Screen {
    buffer: VramBufferInfo,
    cursor_x: i64,
    cursor_y: i64,
}

// SAFETY: the screen is a fixed framebuffer address the CPU writes through; on
// this single-CPU OS nothing moves it between cores.
unsafe impl Send for Screen {}

static SCREEN: Mutex<Option<Screen>> = Mutex::new(None);

/// Makes `buffer` the screen `print!` writes to.
pub fn install_screen(buffer: VramBufferInfo) {
    *SCREEN.lock() = Some(Screen {
        buffer,
        cursor_x: 0,
        cursor_y: 0,
    });
}

fn screen_write(s: &str) {
    let mut screen = SCREEN.lock();
    let Some(screen) = screen.as_mut() else {
        return;
    };
    for c in s.chars() {
        if c == '\n' {
            screen.cursor_y += 16;
            screen.cursor_x = 0;
            continue;
        }
        draw_font_fg(
            &mut screen.buffer,
            screen.cursor_x,
            screen.cursor_y,
            0xffffff,
            c,
        );
        screen.cursor_x += 8;
    }
}

/// Forwards every chunk to the serial port and, if present, the screen.
struct Tee;

impl fmt::Write for Tee {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = Serial.write_str(s);
        screen_write(s);
        Ok(())
    }
}

/// Writes `s` without a trailing newline.
pub fn print_str(s: &str) {
    let _ = Tee.write_str(s);
}

/// Writes formatted text without a trailing newline.
pub fn print_fmt(args: fmt::Arguments) {
    let _ = Tee.write_fmt(args);
}

/// Prints to the serial port and the screen.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::print_fmt(format_args!($($arg)*))
    };
}

/// Prints to the serial port and the screen, with a trailing newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::console::print_str("\n")
    };
    ($($arg:tt)*) => {{
        $crate::console::print_fmt(format_args!($($arg)*));
        $crate::console::print_str("\n");
    }};
}

/// Logs an informational message with its source location.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::print!("[INFO]  {}:{:<3}: {}\n", file!(), line!(), format_args!($($arg)*))
    };
}

/// Logs a warning with its source location.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::print!("[WARN]  {}:{:<3}: {}\n", file!(), line!(), format_args!($($arg)*))
    };
}

/// Logs an error with its source location.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::print!("[ERROR] {}:{:<3}: {}\n", file!(), line!(), format_args!($($arg)*))
    };
}

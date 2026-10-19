//! Serial port (COM1) output for debugging.
//!
//! `scripts/launch_qemu.sh` starts QEMU with `-serial stdio`, so everything
//! written here shows up in the terminal that runs `cargo test` or `cargo run`.

use crate::x86::read_io_port_u8;
use crate::x86::write_io_port_u8;
use core::fmt;
use core::fmt::Write;

const COM1: u16 = 0x3f8;
/// The line status register; bit 5 is set while the transmitter can take a byte.
const COM1_LINE_STATUS: u16 = COM1 + 5;
const TRANSMITTER_EMPTY: u8 = 1 << 5;

/// A writer for COM1. It carries no state (the port itself is the writer), so a
/// fresh value can be used anywhere without sharing or locking.
pub struct Serial;

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            while read_io_port_u8(COM1_LINE_STATUS) & TRANSMITTER_EMPTY == 0 {
                core::hint::spin_loop();
            }
            write_io_port_u8(COM1, byte);
        }
        Ok(())
    }
}

/// Writes `s` to the serial port without a trailing newline.
pub fn print_str(s: &str) {
    let _ = Serial.write_str(s);
}

/// Writes formatted text to the serial port without a trailing newline.
pub fn print_fmt(args: fmt::Arguments) {
    let _ = Serial.write_fmt(args);
}

/// Prints to the serial port.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::serial::print_fmt(format_args!($($arg)*))
    };
}

/// Prints to the serial port with a trailing newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::serial::print_str("\n")
    };
    ($($arg:tt)*) => {{
        $crate::serial::print_fmt(format_args!($($arg)*));
        $crate::serial::print_str("\n");
    }};
}

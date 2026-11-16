//! Serial port (COM1) output for debugging.
//!
//! `scripts/launch_qemu.sh` starts QEMU with `-serial stdio`, so everything
//! written here shows up in the terminal that runs `cargo test` or `cargo run`.

use crate::x86::read_io_port_u8;
use crate::x86::write_io_port_u8;
use core::fmt;

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

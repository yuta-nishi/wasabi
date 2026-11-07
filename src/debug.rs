//! Debug printing helpers.

use crate::serial::Serial;
use core::fmt;

/// Formats `data` as 16 bytes per line: offset, hex bytes, then ASCII.
pub fn write_hexdump(out: &mut impl fmt::Write, data: &[u8]) -> fmt::Result {
    for (line, chunk) in data.chunks(16).enumerate() {
        write!(out, "{:08x}  ", line * 16)?;
        for byte in chunk {
            write!(out, "{byte:02x} ")?;
        }
        for _ in chunk.len()..16 {
            out.write_str("   ")?;
        }
        out.write_str(" |")?;
        for byte in chunk {
            let c = if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            };
            write!(out, "{c}")?;
        }
        writeln!(out, "|")?;
    }
    Ok(())
}

/// Prints `data` to the serial port as a hexdump.
pub fn hexdump(data: &[u8]) {
    let _ = write_hexdump(&mut Serial, data);
}

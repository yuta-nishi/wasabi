//! Tests for the debug printing helpers.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use wasabi::debug::write_hexdump;

pub fn hexdump_prints_16_bytes_per_line() {
    let mut out = String::new();
    write_hexdump(&mut out, b"0123456789abcdef").expect("hexdump failed");
    assert_eq!(
        out,
        concat!(
            "00000000  30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66  ",
            "|0123456789abcdef|\n"
        )
    );
}

pub fn hexdump_pads_the_last_line() {
    let mut out = String::new();
    write_hexdump(&mut out, b"Hi").expect("hexdump failed");
    assert_eq!(
        out,
        format!("00000000  48 69 {} |Hi|\n", "   ".repeat(14))
    );
}

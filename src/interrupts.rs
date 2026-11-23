pub mod stubs;

use crate::x86::write_io_port_u8;
use core::sync::atomic::AtomicU64;

const PIC1: u16 = 0x20;
const PIC1_COMMAND: u16 = PIC1;
const PIC1_DATA: u16 = PIC1 + 1;
const PIC2_COMMAND: u16 = 0xa0;
const PIC2_DATA: u16 = 0xa1;
/// Where IRQ0..7 and IRQ8..15 land in the IDT after remapping.
const PIC1_OFFSET: u8 = 32;
const PIC2_OFFSET: u8 = PIC1_OFFSET + 8;
/// End-of-interrupt command.
const EOI: u8 = 0x20;
/// IRQ0 is the only line we want: every other bit in the mask disables it.
const UNMASK_ONLY_IRQ0: u8 = 0b1111_1110;
const MASK_ALL: u8 = 0xff;

/// The PIT's input clock: 1.193182 MHz.
const PIT_FREQUENCY: u32 = 1_193_182;
const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;

/// Timer interrupts since [`start`].
pub static TICKS: AtomicU64 = AtomicU64::new(0);

fn pic_settle() {
    // The PIC needs a moment between the initialization commands; the classic
    // remedy is an I/O write to an unused port.
    write_io_port_u8(0x80, 0);
}

/// Moves IRQ0..15 to vectors 32..47 and unmasks only the timer.
pub fn remap_pic() {
    // ICW1: start initialization, ICW4 will follow.
    write_io_port_u8(PIC1_COMMAND, 0x11);
    write_io_port_u8(PIC2_COMMAND, 0x11);
    pic_settle();
    // ICW2: vector offsets.
    write_io_port_u8(PIC1_DATA, PIC1_OFFSET);
    write_io_port_u8(PIC2_DATA, PIC2_OFFSET);
    pic_settle();
    // ICW3: the slave hangs off IRQ2 of the master.
    write_io_port_u8(PIC1_DATA, 0x04);
    write_io_port_u8(PIC2_DATA, 0x02);
    pic_settle();
    // ICW4: 8086 mode.
    write_io_port_u8(PIC1_DATA, 0x01);
    write_io_port_u8(PIC2_DATA, 0x01);
    pic_settle();
    // Only IRQ0 (the timer) is allowed through.
    write_io_port_u8(PIC1_DATA, UNMASK_ONLY_IRQ0);
    write_io_port_u8(PIC2_DATA, MASK_ALL);
}

/// Programs PIT channel 0 to fire IRQ0 `hz` times per second.
pub fn start_pit(hz: u32) {
    let divisor = (PIT_FREQUENCY / hz) as u16;
    // Channel 0, lobyte/hibyte, mode 3 (square wave).
    write_io_port_u8(PIT_COMMAND, 0x36);
    write_io_port_u8(PIT_CHANNEL0, divisor as u8);
    write_io_port_u8(PIT_CHANNEL0, (divisor >> 8) as u8);
}

/// Tells the PIC that the current interrupt has been handled.
pub fn send_eoi() {
    write_io_port_u8(PIC1_COMMAND, EOI);
}

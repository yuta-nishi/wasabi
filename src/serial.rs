use crate::result::Result;
use crate::x86::read_io_port_u8;
use crate::x86::write_io_port_u8;
use core::fmt;

const COM1: u16 = 0x3f8;
/// Interrupt enable register; also the high byte of the divisor while DLAB is set.
const COM1_INTERRUPT_ENABLE: u16 = COM1 + 1;
/// FIFO control register.
const COM1_FIFO_CONTROL: u16 = COM1 + 2;
/// Line control register; bit 7 (DLAB) exposes the divisor.
const COM1_LINE_CONTROL: u16 = COM1 + 3;
/// Modem control register.
const COM1_MODEM_CONTROL: u16 = COM1 + 4;
/// The line status register; bit 5 is set while the transmitter can take a byte.
const COM1_LINE_STATUS: u16 = COM1 + 5;
const TRANSMITTER_EMPTY: u8 = 1 << 5;
/// Bit 0 of the line status register: a received byte is waiting in `COM1`.
const DATA_READY: u8 = 1 << 0;

/// 115200 baud on a 1.8432 MHz clock: the divisor is 16.
const BAUD_DIVISOR: u16 = 1;
/// DTR and RTS asserted, OUT2 on: the line is ready and the IRQ line is wired.
const MODEM_READY: u8 = 0x0b;
/// [`MODEM_READY`] plus loopback: transmitted bytes come straight back.
const MODEM_LOOPBACK: u8 = 0x1e;

/// A writer for COM1. It carries no state (the port itself is the writer), so a
/// fresh value can be used anywhere without sharing or locking.
pub struct Serial;

impl Serial {
    /// Programs COM1: 8N1 at 115200 baud, FIFO on, no interrupts.
    ///
    /// The firmware leaves the port working, but it may also leave its own
    /// divisor or receive interrupts in place. Setting everything here makes the
    /// port behave the same whatever the firmware chose, and leaves it quiet:
    /// input is polled from the executor, not delivered by interrupt.
    pub fn init(&mut self) {
        write_io_port_u8(COM1_INTERRUPT_ENABLE, 0x00);
        // DLAB on: the two low registers now carry the baud rate divisor.
        write_io_port_u8(COM1_LINE_CONTROL, 0x80);
        write_io_port_u8(COM1, (BAUD_DIVISOR & 0xff) as u8);
        write_io_port_u8(COM1_INTERRUPT_ENABLE, (BAUD_DIVISOR >> 8) as u8);
        // DLAB off: 8 data bits, no parity, one stop bit.
        write_io_port_u8(COM1_LINE_CONTROL, 0x03);
        // FIFO on and cleared, with a 14-byte threshold.
        write_io_port_u8(COM1_FIFO_CONTROL, 0xc7);
        write_io_port_u8(COM1_MODEM_CONTROL, MODEM_READY);
    }

    /// Sends a byte with the port in loopback mode and reads it back, proving
    /// that the transmit and receive paths both work.
    ///
    /// The modem control register is restored on the way out even when the test
    /// fails, so a broken port cannot also be left deaf.
    pub fn loopback_test(&mut self) -> Result<()> {
        write_io_port_u8(COM1_MODEM_CONTROL, MODEM_LOOPBACK);
        let byte = b'T';
        while read_io_port_u8(COM1_LINE_STATUS) & TRANSMITTER_EMPTY == 0 {
            core::hint::spin_loop();
        }
        write_io_port_u8(COM1, byte);
        // In loopback the byte comes back immediately, but the register updates
        // a clock or two later, so a few tries are allowed before giving up.
        let mut outcome = Err("loopback_test failed: no response");
        for _ in 0..1_000 {
            if let Some(received) = self.try_read() {
                outcome = if received == byte {
                    Ok(())
                } else {
                    Err("loopback_test failed: wrong data received")
                };
                break;
            }
            core::hint::spin_loop();
        }
        write_io_port_u8(COM1_MODEM_CONTROL, MODEM_READY);
        outcome
    }

    /// The next byte the host has sent, or `None` when none has arrived yet.
    ///
    /// The port itself is not touched until the line status says a byte is
    /// waiting, so this never blocks and a task can poll it from the executor.
    pub fn try_read(&mut self) -> Option<u8> {
        if read_io_port_u8(COM1_LINE_STATUS) & DATA_READY == 0 {
            None
        } else {
            Some(read_io_port_u8(COM1))
        }
    }
}

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

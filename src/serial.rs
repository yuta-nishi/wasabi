use crate::bitfield;
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
/// The line status register.
const COM1_LINE_STATUS: u16 = COM1 + 5;

/// 115200 baud on a 1.8432 MHz clock: the divisor is 16.
const BAUD_DIVISOR: u16 = 1;

bitfield! {
    /// The line control register: how bits are framed on the wire.
    LineControl: u8 {
        word_length: 1, 0 -> set_word_length,
        stop_bits: 2, 2 -> set_stop_bits,
        parity: 5, 3 -> set_parity,
        divisor_latch: 7, 7 -> set_divisor_latch,
    }
}

bitfield! {
    /// The FIFO control register: whether the 16-byte buffers are used.
    FifoControl: u8 {
        enable: 0, 0 -> set_enable,
        clear_received: 1, 1 -> set_clear_received,
        clear_transmitted: 2, 2 -> set_clear_transmitted,
        trigger: 7, 6 -> set_trigger,
    }
}

bitfield! {
    /// The modem control register: the lines the port drives.
    ModemControl: u8 {
        dtr: 0, 0 -> set_dtr,
        rts: 1, 1 -> set_rts,
        out1: 2, 2 -> set_out1,
        out2: 3, 3 -> set_out2,
        loopback: 4, 4 -> set_loopback,
    }
}

bitfield! {
    /// The line status register: what the port is doing.
    LineStatus: u8 {
        data_ready: 0, 0,
        overrun: 1, 1,
        transmitter_empty: 5, 5,
    }
}

/// DTR and RTS asserted, OUT2 on: the line is ready and the IRQ line is wired.
fn modem_ready() -> ModemControl {
    let mut control = ModemControl::new(0);
    control.set_dtr(1);
    control.set_rts(1);
    control.set_out2(1);
    control
}

/// [`modem_ready`] plus loopback: transmitted bytes come straight back.
fn modem_loopback() -> ModemControl {
    let mut control = modem_ready();
    control.set_loopback(1);
    control
}

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

        // The divisor lives in the two low registers while the latch is open.
        let mut control = LineControl::new(0);
        control.set_divisor_latch(1);
        write_io_port_u8(COM1_LINE_CONTROL, control.raw());
        write_io_port_u8(COM1, (BAUD_DIVISOR & 0xff) as u8);
        write_io_port_u8(COM1_INTERRUPT_ENABLE, (BAUD_DIVISOR >> 8) as u8);

        // Eight data bits, no parity, one stop bit.
        let mut control = LineControl::new(0);
        control.set_word_length(0b11);
        write_io_port_u8(COM1_LINE_CONTROL, control.raw());

        let mut fifo = FifoControl::new(0);
        fifo.set_enable(1);
        fifo.set_clear_received(1);
        fifo.set_clear_transmitted(1);
        fifo.set_trigger(0b11);
        write_io_port_u8(COM1_FIFO_CONTROL, fifo.raw());

        write_io_port_u8(COM1_MODEM_CONTROL, modem_ready().raw());
    }

    /// Sends a byte with the port in loopback mode and reads it back, proving
    /// that the transmit and receive paths both work.
    ///
    /// The modem control register is restored on the way out even when the test
    /// fails, so a broken port cannot also be left deaf.
    pub fn loopback_test(&mut self) -> Result<()> {
        write_io_port_u8(COM1_MODEM_CONTROL, modem_loopback().raw());
        let byte = b'T';
        while LineStatus::new(read_io_port_u8(COM1_LINE_STATUS)).transmitter_empty() == 0
        {
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
        write_io_port_u8(COM1_MODEM_CONTROL, modem_ready().raw());
        outcome
    }

    /// The next byte the host has sent, or `None` when none has arrived yet.
    ///
    /// The port itself is not touched until the line status says a byte is
    /// waiting, so this never blocks and a task can poll it from the executor.
    pub fn try_read(&mut self) -> Option<u8> {
        if LineStatus::new(read_io_port_u8(COM1_LINE_STATUS)).data_ready() == 0 {
            None
        } else {
            Some(read_io_port_u8(COM1))
        }
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            while LineStatus::new(read_io_port_u8(COM1_LINE_STATUS)).transmitter_empty()
                == 0
            {
                core::hint::spin_loop();
            }
            write_io_port_u8(COM1, byte);
        }
        Ok(())
    }
}

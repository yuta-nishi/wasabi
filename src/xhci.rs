//! The xHCI controller: its register windows, and the rings it reads from memory.
//!
//! The controller is reached through one memory window (its BAR0) that holds four
//! register groups at offsets the controller itself reports: the capability
//! registers at 0, the operational registers at `CAPLENGTH`, the runtime registers
//! at `RTSOFF`, and the doorbell array at `DBOFF`.
//!
//! Work is handed over through rings in physical memory: the driver writes a
//! transfer request block (TRB) into the command ring and rings the doorbell, and
//! the controller writes events into the event ring.

use crate::iobox::IoBox;
use crate::iobox::allocate_uncached;
use crate::mmio::Mmio;
use crate::paging::PageTables;
use crate::result::Result;
use crate::timer;
use core::ptr::read_volatile;
use core::ptr::write_volatile;

/// Generic capability register offsets.
const CAPLENGTH: usize = 0x00;
const HCSPARAMS1: usize = 0x04;
const HCSPARAMS2: usize = 0x08;
const DBOFF: usize = 0x14;
const RTSOFF: usize = 0x18;

/// Operational register offsets, relative to `CAPLENGTH`.
const USBCMD: usize = 0x00;
const USBSTS: usize = 0x04;
const PAGESIZE: usize = 0x08;
const CRCR: usize = 0x18;
const DCBAAP: usize = 0x30;
const CONFIG: usize = 0x38;

/// Runtime registers of interrupter 0, relative to `RTSOFF`.
const INTERRUPTER0: usize = 0x20;
const ERSTSZ: usize = 0x08;
const ERSTBA: usize = 0x10;
const ERDP: usize = 0x18;

const USBCMD_RUN: u32 = 1 << 0;
const USBCMD_RESET: u32 = 1 << 1;
const USBSTS_HALTED: u32 = 1 << 0;
const USBSTS_NOT_READY: u32 = 1 << 11;

/// How long to wait for the controller to reach a state, in milliseconds.
const TIMEOUT_MS: u64 = 100;
/// TRBs in the command ring and in the event ring segment.
const RING_TRBS: usize = 256;

/// A transfer request block: 16 bytes the controller and the driver hand to each
/// other through a ring.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Trb {
    dword0: u32,
    dword1: u32,
    dword2: u32,
    dword3: u32,
}

/// `No Op Command` (TRB type 23; type 8 is the transfer-ring `No Op`): the
/// controller completes it without doing anything, which makes it a check that
/// the rings work.
const TRB_NO_OP_COMMAND: u32 = 23;
/// `Link` (TRB type 6): jumps back to the start of the ring.
const TRB_LINK: u32 = 6;
/// `Command Completion Event` (TRB type 33): the controller's answer to a command.
const TRB_COMMAND_COMPLETION: u32 = 33;
/// Completion code 1: success.
const COMPLETION_SUCCESS: u32 = 1;

fn trb_type(trb: &Trb) -> u32 {
    (trb.dword3 >> 10) & 0x3f
}

fn completion_code(trb: &Trb) -> u32 {
    (trb.dword2 >> 24) & 0xff
}

impl Trb {
    fn no_op_command(cycle: bool) -> Self {
        Self {
            dword3: (TRB_NO_OP_COMMAND << 10) | u32::from(cycle),
            ..Self::default()
        }
    }

    /// Points the ring's last slot back at `ring` and flips the cycle state, so
    /// the controller can tell a fresh lap from the previous one.
    fn link(ring: u64, cycle: bool) -> Self {
        Self {
            dword0: ring as u32,
            dword1: (ring >> 32) as u32,
            dword2: 0,
            dword3: (TRB_LINK << 10) | (1 << 1) | u32::from(cycle),
        }
    }
}

/// One event ring segment table entry: where a segment is and how big it is.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ErstEntry {
    base: u64,
    size: u32,
    _reserved: u32,
}

/// The four register groups, as addresses inside the controller's window.
pub struct Registers {
    window: u64,
    caplength: u64,
    rtsoff: u64,
    dboff: u64,
}

impl Registers {
    /// Locates the register groups; the window itself must already be mapped.
    pub fn new(window: u64) -> Result<Self> {
        // SAFETY: `window` is the controller's register window, mapped by the
        // caller; the offsets come from the controller's own registers.
        let caplength =
            u64::from(unsafe { Mmio::<u8>::at(window + CAPLENGTH as u64) }.read());
        if caplength == 0 {
            return Err("the xHCI controller reports a zero CAPLENGTH");
        }
        let dboff = u64::from(unsafe { Mmio::<u32>::at(window + DBOFF as u64) }.read());
        let rtsoff = u64::from(unsafe { Mmio::<u32>::at(window + RTSOFF as u64) }.read());
        Ok(Self {
            window,
            caplength,
            rtsoff,
            dboff,
        })
    }

    /// One of the capability registers, `offset` bytes into the window.
    fn capability<T: Copy>(&self, offset: usize) -> Mmio<T> {
        // SAFETY: the window is device memory and `offset` stays inside it.
        unsafe { Mmio::at(self.window + offset as u64) }
    }

    /// One of the operational registers, relative to `CAPLENGTH`.
    fn operational<T: Copy>(&self, offset: usize) -> Mmio<T> {
        // SAFETY: as above; `CAPLENGTH` was read from the controller.
        unsafe { Mmio::at(self.window + self.caplength + offset as u64) }
    }

    /// One of interrupter 0's runtime registers, relative to `RTSOFF`.
    fn runtime<T: Copy>(&self, offset: usize) -> Mmio<T> {
        // SAFETY: as above; `RTSOFF` was read from the controller.
        unsafe {
            Mmio::at(self.window + self.rtsoff + INTERRUPTER0 as u64 + offset as u64)
        }
    }

    /// One entry of the doorbell array, relative to `DBOFF`.
    fn doorbell(&self, slot: u32) -> Mmio<u32> {
        // SAFETY: as above; `DBOFF` was read from the controller.
        unsafe { Mmio::at(self.window + self.dboff + u64::from(slot) * 4) }
    }

    /// The version the controller reports, in BCD (`0x0100` is 1.0).
    pub fn version(&self) -> u16 {
        // The capability dword holds CAPLENGTH, a reserved byte, then the
        // version, so read the whole dword and shift the version out.
        (self.read_cap(CAPLENGTH) >> 16) as u16
    }

    fn read_cap(&self, offset: usize) -> u32 {
        self.capability::<u32>(offset).read()
    }

    fn read_op(&self, offset: usize) -> u32 {
        self.operational::<u32>(offset).read()
    }

    fn write_op(&self, offset: usize, value: u32) {
        self.operational::<u32>(offset).write(value);
    }

    fn write_op64(&self, offset: usize, value: u64) {
        self.operational::<u64>(offset).write(value);
    }

    fn write_rt(&self, offset: usize, value: u32) {
        self.runtime::<u32>(offset).write(value);
    }

    fn write_rt64(&self, offset: usize, value: u64) {
        self.runtime::<u64>(offset).write(value);
    }

    fn ring_doorbell(&self, slot: u32, target: u32) {
        self.doorbell(slot).write((target << 16) | slot);
    }

    /// Waits until `mask` is clear in `USBSTS`, or gives up.
    fn wait_status_clear(&self, mask: u32) -> Result<()> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        while self.read_op(USBSTS) & mask != 0 {
            if timer::nanoseconds() >= deadline {
                return Err("the xHCI controller did not reach the expected state");
            }
            core::hint::spin_loop();
        }
        Ok(())
    }

    fn wait_status_set(&self, mask: u32) -> Result<()> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        while self.read_op(USBSTS) & mask == 0 {
            if timer::nanoseconds() >= deadline {
                return Err("the xHCI controller did not reach the expected state");
            }
            core::hint::spin_loop();
        }
        Ok(())
    }
}

/// A controller that has been reset, given rings, and started.
pub struct Controller {
    registers: Registers,
    command_ring: IoBox<[Trb; RING_TRBS]>,
    command_index: usize,
    command_cycle: bool,
    event_ring: IoBox<[Trb; RING_TRBS]>,
    event_index: usize,
    event_cycle: bool,
    _erst: IoBox<[ErstEntry; 1]>,
    _dcbaa: u64,
}

impl Controller {
    /// Takes the controller over: halt it, reset it, give it its rings and start
    /// it, then prove the whole chain with a no-op command.
    pub fn init(registers: Registers, tables: &mut PageTables) -> Result<Self> {
        let hcsparams1 = registers.read_cap(HCSPARAMS1);
        let max_slots = hcsparams1 & 0xff;
        let max_interrupters = (hcsparams1 >> 8) & 0x7ff;
        let max_ports = (hcsparams1 >> 24) & 0xff;
        let scratchpad = scratchpad_count(registers.read_cap(HCSPARAMS2));
        crate::info!(
            "xhci: {max_slots} slots, {max_interrupters} interrupters, {max_ports} ports, \
             {scratchpad} scratchpad buffers"
        );

        // The firmware may have left the controller running with its own rings;
        // stop it before taking them away.
        registers.write_op(USBCMD, registers.read_op(USBCMD) & !USBCMD_RUN);
        registers.wait_status_set(USBSTS_HALTED)?;
        registers.write_op(USBCMD, registers.read_op(USBCMD) | USBCMD_RESET);
        registers.wait_status_clear(USBCMD_RESET)?;
        registers.wait_status_clear(USBSTS_NOT_READY)?;

        // 4 KiB pages: the only size this OS allocates.
        registers.write_op(PAGESIZE, 1);

        // SAFETY: an all-zero TRB is a valid, unused block.
        let command_ring = unsafe { IoBox::<[Trb; RING_TRBS]>::new(tables) };
        // SAFETY: as above.
        let event_ring = unsafe { IoBox::<[Trb; RING_TRBS]>::new(tables) };
        // SAFETY: an all-zero segment table entry describes nothing yet.
        let erst = unsafe { IoBox::<[ErstEntry; 1]>::new(tables) };
        let mut controller = Self {
            registers,
            command_ring,
            command_index: 0,
            command_cycle: true,
            event_ring,
            event_index: 0,
            event_cycle: true,
            _erst: erst,
            _dcbaa: 0,
        };
        controller.register_rings(tables, max_slots, scratchpad)?;
        controller.start()?;
        controller.check_with_no_op()?;
        Ok(controller)
    }

    /// Tells the controller where its device context array and both rings are.
    fn register_rings(
        &mut self,
        tables: &mut PageTables,
        max_slots: u32,
        scratchpad: u32,
    ) -> Result<()> {
        let registers = &self.registers;

        // Device context array: one entry per slot, plus the scratchpad pointer.
        let dcbaa_entries = max_slots as usize + 1;
        let dcbaa = allocate_uncached(tables, dcbaa_entries * 8)?;
        if scratchpad > 0 {
            // The first entry points at an array of scratchpad buffers, which the
            // controller uses when it needs a scratch area of its own.
            let buffers = allocate_uncached(tables, scratchpad as usize * 8)? as *mut u64;
            for index in 0..scratchpad as usize {
                let buffer = allocate_uncached(tables, 4096)?;
                // SAFETY: `index` is inside the array just allocated.
                unsafe { write_volatile(buffers.add(index), buffer) };
            }
            // SAFETY: entry 0 exists, since `dcbaa_entries` is at least 1.
            unsafe { write_volatile(dcbaa as *mut u64, buffers as u64) };
        }
        self._dcbaa = dcbaa;
        registers.write_op64(DCBAAP, dcbaa);
        registers.write_op(CONFIG, max_slots & 0xff);

        // Event ring segment table: one segment of `RING_TRBS` TRBs.
        // SAFETY: the table holds exactly one entry and nothing else reads it.
        unsafe {
            *self._erst.as_mut() = [ErstEntry {
                base: self.event_ring.address(),
                size: RING_TRBS as u32,
                _reserved: 0,
            }]
        };
        registers.write_rt(ERSTSZ, 1);
        registers.write_rt64(ERSTBA, self._erst.address());
        registers.write_rt64(ERDP, self.event_ring.address());

        // Command ring: the last TRB links back to the first.
        let link = Trb::link(self.command_ring.address(), true);
        // SAFETY: the ring has `RING_TRBS` slots and the link goes in the last.
        unsafe { write_volatile(self.command_ring_ptr().add(RING_TRBS - 1), link) };
        registers.write_op64(CRCR, self.command_ring.address() | 1);
        Ok(())
    }

    fn command_ring_ptr(&self) -> *mut Trb {
        self.command_ring.address() as *mut Trb
    }

    fn event_ring_ptr(&self) -> *mut Trb {
        self.event_ring.address() as *mut Trb
    }

    fn start(&mut self) -> Result<()> {
        let registers = &self.registers;
        registers.write_op(USBCMD, registers.read_op(USBCMD) | USBCMD_RUN);
        registers.wait_status_clear(USBSTS_HALTED)
    }

    /// Sends a no-op command and waits for its completion event: if that arrives,
    /// the doorbell, the command ring, the event ring and the completion path all
    /// work.
    fn check_with_no_op(&mut self) -> Result<()> {
        self.command(Trb::no_op_command(self.command_cycle))?;
        let event = self.next_event()?;
        if trb_type(&event) != TRB_COMMAND_COMPLETION {
            return Err("the xHCI controller sent an unexpected event");
        }
        let code = completion_code(&event);
        if code != COMPLETION_SUCCESS {
            return Err("the no-op command did not complete successfully");
        }
        crate::info!("xhci: started, and the no-op command completed");
        Ok(())
    }

    /// The version the controller reports, in BCD.
    pub fn version(&self) -> u16 {
        self.registers.version()
    }

    /// Hands one command TRB to the controller.
    fn command(&mut self, trb: Trb) -> Result<()> {
        if self.command_index == RING_TRBS - 1 {
            return Err("the command ring needs a lap it cannot do yet");
        }
        // SAFETY: `command_index` is below the link entry at the end.
        unsafe { write_volatile(self.command_ring_ptr().add(self.command_index), trb) };
        self.command_index += 1;
        self.registers.ring_doorbell(0, 0);
        Ok(())
    }

    /// Waits for the next event the controller writes into the event ring.
    fn next_event(&mut self) -> Result<Trb> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            // SAFETY: `event_index` is inside the ring segment.
            let event =
                unsafe { read_volatile(self.event_ring_ptr().add(self.event_index)) };
            if (event.dword3 & 1 != 0) == self.event_cycle {
                self.event_index += 1;
                if self.event_index == RING_TRBS {
                    self.event_index = 0;
                    self.event_cycle = !self.event_cycle;
                }
                // Tell the controller how far the driver has read.
                self.registers.write_rt64(
                    ERDP,
                    self.event_ring.address() + (self.event_index * 16) as u64,
                );
                return Ok(event);
            }
            if timer::nanoseconds() >= deadline {
                return Err("the xHCI controller sent no event");
            }
            core::hint::spin_loop();
        }
    }
}

/// `HCSPARAMS2` holds the scratchpad buffer count in two 5-bit halves.
fn scratchpad_count(hcsparams2: u32) -> u32 {
    let low = (hcsparams2 >> 21) & 0x1f;
    let high = (hcsparams2 >> 27) & 0x1f;
    (high << 5) | low
}

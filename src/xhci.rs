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
//!
//! Register and TRB layouts follow the xHCI specification:
//! <https://www.intel.com/content/www/us/en/standards/xhci-specification.html>.

use crate::iobox::IoBox;
use crate::mmio::Mmio;
use crate::mmio::Window;
use crate::paging::PageTables;
use crate::result::Result;
use crate::timer;

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
/// Most device slots the controller may report (`MaxSlots` is one byte).
const MAX_SLOTS: usize = 256;
/// Most scratchpad buffers the controller may report (the count is ten bits).
const MAX_SCRATCHPAD: usize = 1023;

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

// SAFETY: a TRB is four integers, so every bit pattern is valid.
unsafe impl crate::iobox::Zeroable for Trb {}

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

// SAFETY: a segment table entry is two integers and a reserved word.
unsafe impl crate::iobox::Zeroable for ErstEntry {}

// SAFETY: a byte has no invalid bit pattern.
unsafe impl crate::iobox::Zeroable for [u8; 4096] {}

/// The four register groups, as addresses inside the controller's window.
pub struct Registers {
    window: Window,
    caplength: u64,
    rtsoff: u64,
    dboff: u64,
}

impl Registers {
    /// Locates the register groups; the window itself must already be mapped.
    pub fn new(window: u64) -> Result<Self> {
        // SAFETY: `window` is the controller's register window, mapped uncached
        // by the caller.
        let window = unsafe { Window::new(window) };
        let caplength = u64::from(window.at::<u8>(CAPLENGTH).read());
        if caplength == 0 {
            return Err("the xHCI controller reports a zero CAPLENGTH");
        }
        let dboff = u64::from(window.at::<u32>(DBOFF).read());
        let rtsoff = u64::from(window.at::<u32>(RTSOFF).read());
        Ok(Self {
            window,
            caplength,
            rtsoff,
            dboff,
        })
    }

    /// One of the capability registers, `offset` bytes into the window.
    fn capability<T: Copy>(&self, offset: usize) -> Mmio<T> {
        self.window.at(offset)
    }

    /// One of the operational registers, relative to `CAPLENGTH`.
    fn operational<T: Copy>(&self, offset: usize) -> Mmio<T> {
        self.window.at(self.caplength as usize + offset)
    }

    /// One of interrupter 0's runtime registers, relative to `RTSOFF`.
    fn runtime<T: Copy>(&self, offset: usize) -> Mmio<T> {
        self.window.at(self.rtsoff as usize + INTERRUPTER0 + offset)
    }

    /// One entry of the doorbell array, relative to `DBOFF`.
    fn doorbell(&self, slot: u32) -> Mmio<u32> {
        self.window.at(self.dboff as usize + slot as usize * 4)
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

/// A ring of TRBs that the controller walks at its own pace.
///
/// The driver writes entries at `index`; the cycle bit on each entry marks the
/// lap it belongs to, so both sides can tell a fresh entry from one left over
/// from an earlier lap.
struct Ring<const N: usize> {
    entries: IoBox<[Trb; N]>,
    index: usize,
    cycle: bool,
}

impl<const N: usize> Ring<N> {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            entries: IoBox::new(tables),
            index: 0,
            cycle: true,
        }
    }

    fn address(&self) -> u64 {
        self.entries.address()
    }

    /// Where the next entry sits, in bytes from the ring's address.
    fn offset(&self) -> u64 {
        (self.index * core::mem::size_of::<Trb>()) as u64
    }

    /// Hands over one entry. The last slot is the link, so `N - 1` are usable and
    /// a ring that fills up needs a lap the driver cannot start yet.
    fn push(&mut self, trb: Trb) -> Result<()> {
        if self.index + 1 >= N {
            return Err("the command ring is full");
        }
        self.entries
            .write_slot(self.index, trb)
            .ok_or("the command ring is full")?;
        self.index += 1;
        Ok(())
    }

    /// The next entry whose cycle bit says it belongs to this lap.
    fn take(&mut self) -> Option<Trb> {
        let entry = self.entries.read_slot(self.index)?;
        if (entry.dword3 & 1 != 0) != self.cycle {
            return None;
        }
        self.index += 1;
        if self.index == N {
            self.index = 0;
            self.cycle = !self.cycle;
        }
        Some(entry)
    }

    /// Points the last slot back at the first, closing the ring.
    fn link_to_start(&mut self) {
        let link = Trb::link(self.entries.address(), self.cycle);
        self.entries.write_slot(N - 1, link);
    }
}

/// The scratchpad buffers: the array of their addresses, and the pages it names.
type Scratchpad = (
    IoBox<[u64; MAX_SCRATCHPAD]>,
    alloc::vec::Vec<IoBox<[u8; 4096]>>,
);

/// A controller that has been reset, given rings, and started.
pub struct Controller {
    registers: Registers,
    command_ring: Ring<RING_TRBS>,
    event_ring: Ring<RING_TRBS>,
    erst: IoBox<[ErstEntry; 1]>,
    dcbaa: IoBox<[u64; MAX_SLOTS]>,
    _scratchpad: Option<Scratchpad>,
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

        let mut controller = Self {
            registers,
            command_ring: Ring::new(tables),
            event_ring: Ring::new(tables),
            erst: IoBox::new(tables),
            dcbaa: IoBox::new(tables),
            _scratchpad: None,
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
        if max_slots as usize >= MAX_SLOTS {
            return Err("the controller has more slots than this driver supports");
        }
        if scratchpad as usize > MAX_SCRATCHPAD {
            return Err("the controller wants more scratchpad buffers than supported");
        }
        let registers = &self.registers;

        if scratchpad > 0 {
            // Entry 0 of the device context array is where the controller looks
            // for the scratchpad buffers it needs for its own work.
            let mut buffers = IoBox::<[u64; MAX_SCRATCHPAD]>::new(tables);
            let mut pages = alloc::vec::Vec::new();
            for index in 0..scratchpad as usize {
                let page = IoBox::<[u8; 4096]>::new(tables);
                buffers
                    .write_slot(index, page.address())
                    .ok_or("the scratchpad array is full")?;
                pages.push(page);
            }
            self.dcbaa
                .write_slot(0, buffers.address())
                .ok_or("the device context array is full")?;
            self._scratchpad = Some((buffers, pages));
        }
        registers.write_op64(DCBAAP, self.dcbaa.address());
        registers.write_op(CONFIG, max_slots & 0xff);

        self.erst
            .write_slot(
                0,
                ErstEntry {
                    base: self.event_ring.address(),
                    size: RING_TRBS as u32,
                    _reserved: 0,
                },
            )
            .ok_or("the segment table has no room")?;
        registers.write_rt(ERSTSZ, 1);
        registers.write_rt64(ERSTBA, self.erst.address());
        registers.write_rt64(ERDP, self.event_ring.address());

        self.command_ring.link_to_start();
        registers.write_op64(CRCR, self.command_ring.address() | 1);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let registers = &self.registers;
        registers.write_op(USBCMD, registers.read_op(USBCMD) | USBCMD_RUN);
        registers.wait_status_clear(USBSTS_HALTED)
    }

    /// The version the controller reports, in BCD.
    pub fn version(&self) -> u16 {
        self.registers.version()
    }

    /// Sends a no-op command and waits for its completion event: if that arrives,
    /// the doorbell, the command ring, the event ring and the completion path all
    /// work.
    fn check_with_no_op(&mut self) -> Result<()> {
        self.command(Trb::no_op_command(self.command_ring.cycle))?;
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

    /// Hands one command TRB to the controller.
    fn command(&mut self, trb: Trb) -> Result<()> {
        self.command_ring.push(trb)?;
        self.registers.ring_doorbell(0, 0);
        Ok(())
    }

    /// Waits for the next event the controller writes into the event ring.
    fn next_event(&mut self) -> Result<Trb> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            if let Some(event) = self.event_ring.take() {
                // Tell the controller how far the driver has read; the address is
                // 16-byte aligned, so this also clears the busy bit.
                let dequeue = self.event_ring.address() + self.event_ring.offset();
                self.registers.write_rt64(ERDP, dequeue);
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

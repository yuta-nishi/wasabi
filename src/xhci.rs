use crate::bitfield;
use crate::info;
use crate::iobox::IoBox;
use crate::iobox::Zeroable;
use crate::mmio::Mmio;
use crate::mmio::Window;
use crate::paging::PageTables;
use crate::result::Result;
use crate::timer;
use alloc::vec::Vec;

/// Generic capability register offsets.
const CAPLENGTH: usize = 0x00;
const HCSPARAMS1: usize = 0x04;
const HCSPARAMS2: usize = 0x08;
const HCCPARAMS1: usize = 0x10;
const DBOFF: usize = 0x14;
const RTSOFF: usize = 0x18;

/// Operational register offsets, relative to `CAPLENGTH`.
const USBCMD: usize = 0x00;
const USBSTS: usize = 0x04;
const PAGESIZE: usize = 0x08;
const CRCR: usize = 0x18;
const DCBAAP: usize = 0x30;
const CONFIG: usize = 0x38;

/// Port register set, relative to `CAPLENGTH`; each port owns 0x10 bytes.
const PORT_REGISTERS: usize = 0x400;
const PORT_SIZE: usize = 0x10;
bitfield! {
    /// `PORTSC`: a port's status and control word.
    Portsc {
        connected: 0, 0,
        enabled: 1, 1,
        reset: 4, 4,
        link_state: 8, 5,
        power: 9, 9,
        speed: 13, 10,
        connect_changed: 17, 17,
        reset_changed: 21, 21,
    }
}

impl Portsc {
    /// The change bits: writing 1 to one clears it.
    fn changes() -> u32 {
        Self::mask(17, 17)
            | Self::mask(18, 18)
            | Self::mask(19, 19)
            | Self::mask(20, 20)
            | Self::mask(21, 21)
            | Self::mask(22, 22)
            | Self::mask(23, 23)
    }

    /// Asks the controller to reset the port.
    fn request_reset(&mut self) {
        self.set_raw(self.raw() | Self::mask(4, 4) | Self::mask(17, 17));
    }

    /// Clears every change bit that is set.
    fn acknowledge_changes(&mut self) {
        self.set_raw(self.raw() | Self::changes());
    }
}

/// The speed a port reports, as the slot context wants it (protocol speed ID).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    Full,
    Low,
    High,
    Super,
}

impl Speed {
    pub fn from_portsc(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Full),
            2 => Some(Self::Low),
            3 => Some(Self::High),
            4 => Some(Self::Super),
            _ => None,
        }
    }

    /// The value the slot context carries.
    pub fn psi(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Low => 2,
            Self::High => 3,
            Self::Super => 4,
        }
    }

    /// The largest control packet at this speed.
    pub fn max_packet_size(self) -> u16 {
        match self {
            Self::Full | Self::Low => 8,
            Self::High => 64,
            Self::Super => 512,
        }
    }
}

/// The code a completion event carries (xHCI specification, table 6-96).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompletionCode {
    Success,
    TrbError,
    SlotNotEnabled,
    EndpointNotEnabled,
    ParameterError,
    ContextStateError,
    EventRingFull,
    CommandRingStopped,
    CommandAborted,
    Other(u32),
}

impl CompletionCode {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::Success,
            5 => Self::TrbError,
            11 => Self::SlotNotEnabled,
            12 => Self::EndpointNotEnabled,
            17 => Self::ParameterError,
            19 => Self::ContextStateError,
            21 => Self::EventRingFull,
            24 => Self::CommandRingStopped,
            25 => Self::CommandAborted,
            other => Self::Other(other),
        }
    }
}

/// `Enable Slot` (TRB type 9): the controller picks a slot for a device.
const TRB_ENABLE_SLOT: u32 = 9;
/// `Address Device` (TRB type 11): the controller gives the device an address
/// and allocates the endpoints its input context describes.
const TRB_ADDRESS_DEVICE: u32 = 11;

/// 32-byte contexts: one slot context, one endpoint context per endpoint.
const CONTEXT_SIZE: usize = 0x20;
/// Words in a page of contexts.
const CONTEXT_WORDS: usize = 1024;
/// `HCCPARAMS1`: context size, 0 for 32 bytes and 1 for 64.
const HCC_CONTEXT_SIZE: u32 = 1 << 2;

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
    parameter: u64,
    status: u32,
    control: u32,
}

// SAFETY: a TRB is four integers, so every bit pattern is valid.
unsafe impl Zeroable for Trb {}

/// `No Op Command` (TRB type 23; type 8 is the transfer-ring `No Op`): the
/// controller completes it without doing anything, which makes it a check that
/// the rings work.
const TRB_NO_OP_COMMAND: u32 = 23;
/// `Link` (TRB type 6): jumps back to the start of the ring.
const TRB_LINK: u32 = 6;
/// `Command Completion Event` (TRB type 33): the controller's answer to a command.
const TRB_COMMAND_COMPLETION: u32 = 33;
/// `Port Status Change Event` (TRB type 34): a port changed on its own.
const TRB_PORT_STATUS_CHANGE: u32 = 34;

fn trb_type(trb: &Trb) -> u32 {
    (trb.control >> 10) & 0x3f
}

fn completion_code(trb: &Trb) -> CompletionCode {
    CompletionCode::from_raw((trb.status >> 24) & 0xff)
}

/// The slot a completion event was about.
fn slot_id(trb: &Trb) -> u32 {
    trb.control >> 24
}

impl Trb {
    fn no_op_command(cycle: bool) -> Self {
        Self {
            control: (TRB_NO_OP_COMMAND << 10) | u32::from(cycle),
            ..Self::default()
        }
    }

    /// `Address Device`: the input context at `input` describes the device.
    fn address_device(input: u64, slot: u32, cycle: bool) -> Self {
        Self {
            parameter: input,
            status: 0,
            control: (TRB_ADDRESS_DEVICE << 10) | (slot << 24) | u32::from(cycle),
        }
    }

    /// `Enable Slot`: the controller answers with the slot it picked.
    fn enable_slot(cycle: bool, slot_type: u32) -> Self {
        Self {
            control: (TRB_ENABLE_SLOT << 10) | (slot_type << 16) | u32::from(cycle),
            ..Self::default()
        }
    }

    /// Points the ring's last slot back at `ring` and flips the cycle state, so
    /// the controller can tell a fresh lap from the previous one.
    fn link(ring: u64, cycle: bool) -> Self {
        Self {
            parameter: ring,
            status: 0,
            control: (TRB_LINK << 10) | (1 << 1) | u32::from(cycle),
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
unsafe impl Zeroable for ErstEntry {}

/// The four register groups of a host controller's window.
///
/// The capability registers start the window, the operational registers sit at
/// `CAPLENGTH`, the runtime registers at `RTSOFF`, and the doorbell array at
/// `DBOFF`; the controller reports all three offsets itself.
#[derive(Clone, Copy)]
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

    /// The status register of port `port` (numbered from 1).
    fn port_status(&self, port: u32) -> Mmio<u32> {
        let offset = PORT_REGISTERS + (port as usize - 1) * PORT_SIZE;
        self.operational(offset)
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
    fn enqueue(&mut self, trb: Trb) -> Result<()> {
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
    fn dequeue(&mut self) -> Option<Trb> {
        let entry = self.entries.read_slot(self.index)?;
        if (entry.control & 1 != 0) != self.cycle {
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
type Scratchpad = (IoBox<[u64; MAX_SCRATCHPAD]>, Vec<IoBox<[u8; 4096]>>);

/// The root hub's ports and what they report.
struct Ports {
    registers: Registers,
    count: u32,
}

impl Ports {
    fn count(&self) -> u32 {
        self.count
    }

    fn status(&self, port: u32) -> Portsc {
        Portsc::new(self.registers.port_status(port).read())
    }

    /// Acknowledges the change bits a port has set.
    fn acknowledge(&self, port: u32, mut status: Portsc) {
        status.acknowledge_changes();
        self.registers.port_status(port).write(status.raw());
    }

    /// Resets `port` and waits until the device is enabled, so it can be
    /// addressed.
    fn reset(&self, port: u32) -> Result<()> {
        let status = self.registers.port_status(port);
        let mut current = Portsc::new(status.read());
        if current.connected() == 0 {
            return Err("no device on the port");
        }
        current.acknowledge_changes();
        status.write(current.raw());
        current.request_reset();
        status.write(current.raw());

        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            let mut current = Portsc::new(status.read());
            if current.reset_changed() != 0 {
                current.acknowledge_changes();
                status.write(current.raw());
                if Portsc::new(status.read()).enabled() == 0 {
                    return Err("the port did not become enabled");
                }
                return Ok(());
            }
            if timer::nanoseconds() >= deadline {
                return Err("the port reset did not finish");
            }
            core::hint::spin_loop();
        }
    }

    /// The speed the port reports.
    fn speed(&self, port: u32) -> Result<Speed> {
        Speed::from_portsc(self.status(port).speed())
            .ok_or("the port reports an unknown speed")
    }
}

/// A device the controller has given an address to.
struct Device {
    slot: u32,
    port: u32,
    /// The endpoint 0 ring: control transfers go through it.
    ep0: Ring<RING_TRBS>,
    /// Where the controller keeps the device's contexts.
    _context: IoBox<[u32; CONTEXT_WORDS]>,
    /// The input context the device was addressed with.
    _input: IoBox<[u32; CONTEXT_WORDS]>,
}

/// The command ring and the doorbell that says it moved.
struct CommandChannel {
    ring: Ring<RING_TRBS>,
}

impl CommandChannel {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            ring: Ring::new(tables),
        }
    }

    fn address(&self) -> u64 {
        self.ring.address()
    }

    /// The cycle bit the next command must carry.
    fn cycle(&self) -> bool {
        self.ring.cycle
    }

    fn link_to_start(&mut self) {
        self.ring.link_to_start();
    }

    fn send(&mut self, registers: &Registers, trb: Trb) -> Result<()> {
        self.ring.enqueue(trb)?;
        registers.ring_doorbell(0, 0);
        Ok(())
    }
}

/// The event ring, the table that describes it, and the read side of the
/// controller's answers.
struct EventChannel {
    ring: Ring<RING_TRBS>,
    erst: IoBox<[ErstEntry; 1]>,
}

impl EventChannel {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            ring: Ring::new(tables),
            erst: IoBox::new(tables),
        }
    }

    /// Points the controller at the segment table and the ring.
    fn program(&mut self, registers: &Registers) -> Result<()> {
        self.erst
            .write_slot(
                0,
                ErstEntry {
                    base: self.ring.address(),
                    size: RING_TRBS as u32,
                    _reserved: 0,
                },
            )
            .ok_or("the segment table has no room")?;
        registers.write_rt(ERSTSZ, 1);
        registers.write_rt64(ERSTBA, self.erst.address());
        registers.write_rt64(ERDP, self.ring.address());
        Ok(())
    }

    /// The next event the controller writes, if one is waiting.
    fn next(&mut self, registers: &Registers) -> Result<Trb> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            if let Some(event) = self.ring.dequeue() {
                let dequeue = self.ring.address() + self.ring.offset();
                registers.write_rt64(ERDP, dequeue);
                return Ok(event);
            }
            if timer::nanoseconds() >= deadline {
                return Err("the xHCI controller sent no event");
            }
            core::hint::spin_loop();
        }
    }

    /// Waits for a command's completion, letting events that belong to other
    /// things pass: ports report changes on their own schedule.
    fn wait_for_completion(&mut self, registers: &Registers) -> Result<Trb> {
        loop {
            let event = self.next(registers)?;
            match trb_type(&event) {
                TRB_COMMAND_COMPLETION => return Ok(event),
                TRB_PORT_STATUS_CHANGE => continue,
                _ => continue,
            }
        }
    }
}

/// The contexts an `Address Device` command hands to the controller.
struct InputContext {
    memory: IoBox<[u32; CONTEXT_WORDS]>,
}

impl InputContext {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            memory: IoBox::new(tables),
        }
    }

    fn address(&self) -> u64 {
        self.memory.address()
    }

    /// Adds the slot context (A0) and endpoint 0 (A1) to this command.
    fn add_slot_and_endpoint0(&mut self) -> Result<()> {
        self.memory
            .write_slot(1, 0b11)
            .ok_or("the input context is too small")?;
        Ok(())
    }

    /// Speed, one context entry, and the root hub port the device sits behind.
    fn set_slot(&mut self, speed: Speed, port: u32) -> Result<()> {
        let slot = CONTEXT_SIZE / 4;
        self.memory
            .write_slot(slot, (speed.psi() << 20) | (1 << 27))
            .ok_or("the input context is too small")?;
        self.memory
            .write_slot(slot + 1, port << 16)
            .ok_or("the input context is too small")?;
        Ok(())
    }

    /// Endpoint 0: control, three retries, its packet size and its ring.
    fn set_endpoint0(&mut self, max_packet: u16, ring: u64) -> Result<()> {
        let ep0 = 2 * (CONTEXT_SIZE / 4);
        self.memory
            .write_slot(ep0 + 1, (u32::from(max_packet) << 16) | (4 << 3) | (3 << 1))
            .ok_or("the input context is too small")?;
        // The dequeue pointer keeps its low four bits for the cycle state.
        self.memory
            .write_slot(ep0 + 2, (ring as u32 & !0xf) | 1)
            .ok_or("the input context is too small")?;
        self.memory
            .write_slot(ep0 + 3, (ring >> 32) as u32)
            .ok_or("the input context is too small")?;
        // Control endpoints use an average TRB length of 8.
        self.memory
            .write_slot(ep0 + 4, 8)
            .ok_or("the input context is too small")?;
        Ok(())
    }
}

/// An xHCI host controller that has been reset, given rings, and started.
pub struct HostController {
    registers: Registers,
    commands: CommandChannel,
    events: EventChannel,
    ports: Ports,
    dcbaa: IoBox<[u64; MAX_SLOTS]>,
    _scratchpad: Option<Scratchpad>,
    devices: Vec<Device>,
}

impl HostController {
    /// Takes the controller over: halt it, reset it, give it its rings and start
    /// it, then prove the whole chain with a no-op command.
    pub fn init(registers: Registers, tables: &mut PageTables) -> Result<Self> {
        let hcsparams1 = registers.read_cap(HCSPARAMS1);
        let max_slots = hcsparams1 & 0xff;
        let max_interrupters = (hcsparams1 >> 8) & 0x7ff;
        let max_ports = (hcsparams1 >> 24) & 0xff;
        let scratchpad = scratchpad_count(registers.read_cap(HCSPARAMS2));
        if registers.read_cap(HCCPARAMS1) & HCC_CONTEXT_SIZE != 0 {
            return Err("this driver only supports 32-byte contexts");
        }
        info!(
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
        registers.write_op(PAGESIZE, 1);

        let mut controller = Self {
            registers,
            commands: CommandChannel::new(tables),
            events: EventChannel::new(tables),
            ports: Ports {
                registers,
                count: max_ports,
            },
            dcbaa: IoBox::new(tables),
            _scratchpad: None,
            devices: Vec::new(),
        };
        controller.program(max_slots, scratchpad, tables)?;
        controller.start()?;
        controller.check_with_no_op()?;
        Ok(controller)
    }

    /// Gives the controller its device context array and both rings.
    fn program(
        &mut self,
        max_slots: u32,
        scratchpad: u32,
        tables: &mut PageTables,
    ) -> Result<()> {
        if max_slots as usize >= MAX_SLOTS {
            return Err("the controller has more slots than this driver supports");
        }
        if scratchpad as usize > MAX_SCRATCHPAD {
            return Err("the controller wants more scratchpad buffers than supported");
        }
        if scratchpad > 0 {
            // Entry 0 of the device context array is where the controller looks
            // for the scratchpad buffers it needs for its own work.
            let mut buffers = IoBox::<[u64; MAX_SCRATCHPAD]>::new(tables);
            let mut pages = Vec::new();
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
        self.registers.write_op64(DCBAAP, self.dcbaa.address());
        self.registers.write_op(CONFIG, max_slots & 0xff);
        self.events.program(&self.registers)?;
        self.commands.link_to_start();
        self.registers.write_op64(CRCR, self.commands.address() | 1);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        self.registers
            .write_op(USBCMD, self.registers.read_op(USBCMD) | USBCMD_RUN);
        self.registers.wait_status_clear(USBSTS_HALTED)
    }

    /// The version the controller reports, in BCD.
    pub fn version(&self) -> u16 {
        self.registers.version()
    }

    /// Port numbers the controller reports, from 1.
    pub fn ports(&self) -> u32 {
        self.ports.count()
    }

    /// Sends a no-op command and waits for its completion event: if that arrives,
    /// the doorbell, the command ring, the event ring and the completion path all
    /// work.
    fn check_with_no_op(&mut self) -> Result<()> {
        let trb = Trb::no_op_command(self.commands.cycle());
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers)?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the no-op command did not complete successfully");
        }
        info!("xhci: started, and the no-op command completed");
        Ok(())
    }

    /// Finds connected devices, resets their ports, and lets the controller pick
    /// a slot for each.
    pub fn enumerate_ports(&mut self, tables: &mut PageTables) -> Result<()> {
        for port in 1..=self.ports.count() {
            let status = self.ports.status(port);
            if status.connected() == 0 {
                continue;
            }
            self.ports.acknowledge(port, status);
            info!(
                "xhci: port {port} has a device (portsc {:#010x})",
                status.raw()
            );
            self.ports.reset(port)?;
            let after = self.ports.status(port);
            // Slot type 0 is a USB2 device; a SuperSpeed port asks for type 1.
            let slot = self.enable_slot(0)?;
            info!(
                "xhci: port {port} reset (portsc {:#010x}), slot {slot} assigned",
                after.raw()
            );
            self.address_device(tables, slot, port)?;
        }
        for device in &self.devices {
            info!(
                "xhci: slot {} on port {} addressed, endpoint 0 ring at {:#018x}",
                device.slot,
                device.port,
                device.ep0.address()
            );
        }
        Ok(())
    }

    /// Asks the controller for a slot, which is where a device's contexts live.
    fn enable_slot(&mut self, slot_type: u32) -> Result<u32> {
        let trb = Trb::enable_slot(self.commands.cycle(), slot_type);
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers)?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the enable slot command failed");
        }
        Ok(slot_id(&event))
    }

    /// Gives the device on `port` an address: the input context describes the
    /// slot and endpoint 0, and the controller fills in the device context.
    fn address_device(
        &mut self,
        tables: &mut PageTables,
        slot: u32,
        port: u32,
    ) -> Result<()> {
        let speed = self.ports.speed(port)?;
        let max_packet = speed.max_packet_size();
        let ep0 = Ring::<RING_TRBS>::new(tables);
        let mut input = InputContext::new(tables);
        let context = IoBox::<[u32; CONTEXT_WORDS]>::new(tables);

        input.add_slot_and_endpoint0()?;
        input.set_slot(speed, port)?;
        input.set_endpoint0(max_packet, ep0.address())?;
        self.dcbaa
            .write_slot(slot as usize, context.address())
            .ok_or("the device context array is full")?;

        let trb = Trb::address_device(input.address(), slot, self.commands.cycle());
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers)?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the address device command failed");
        }
        // The controller writes the device context, and the slot context's last
        // word carries the address it assigned.
        let slot_status = context.read_slot(11).unwrap_or(0);
        info!(
            "xhci: slot {slot} has device address {}",
            slot_status & 0xff
        );
        self.devices.push(Device {
            slot,
            port,
            ep0,
            _context: context,
            _input: input.memory,
        });
        Ok(())
    }
}

fn scratchpad_count(hcsparams2: u32) -> u32 {
    let low = (hcsparams2 >> 21) & 0x1f;
    let high = (hcsparams2 >> 27) & 0x1f;
    (high << 5) | low
}

use crate::bitfield;
use crate::error;
use crate::executor;
use crate::info;
use crate::iobox::IoBox;
use crate::iobox::Zeroable;
use crate::mmio::Mmio;
use crate::mmio::Window;
use crate::paging::PageTables;
use crate::result::Result;
use crate::timer;
use crate::usb::control;
use crate::usb::descriptor;
use crate::usb::descriptor::ConfigurationDescriptor;
use crate::usb::descriptor::DeviceDescriptor;
use crate::usb::descriptor::EndpointDescriptor;
use crate::usb::descriptor::InterfaceDescriptor;
use crate::usb::descriptor::descriptors;
use crate::usb::hid;
use alloc::vec::Vec;
use core::mem::offset_of;
use core::mem::size_of;

/// The capability registers, starting at the window's first byte.
#[repr(C)]
struct Capability {
    caplength_and_version: u32,
    hcsparams1: u32,
    hcsparams2: u32,
    hcsparams3: u32,
    hccparams1: u32,
    dboff: u32,
    rtsoff: u32,
    hccparams2: u32,
}

/// The operational registers, starting at `CAPLENGTH`.
#[repr(C)]
struct Operational {
    usbcmd: u32,
    usbsts: u32,
    pagesize: u32,
    _reserved0c: u32,
    dnctrl: u32,
    _reserved10: u32,
    crcr: u64,
    _reserved20: [u32; 4],
    dcbaap: u64,
    config: u32,
    _reserved3c: u32,
}
const _: () = assert!(offset_of!(Capability, hcsparams1) == 0x04);
const _: () = assert!(offset_of!(Capability, hccparams1) == 0x10);
const _: () = assert!(offset_of!(Capability, dboff) == 0x14);
const _: () = assert!(offset_of!(Capability, rtsoff) == 0x18);
const _: () = assert!(size_of::<Operational>() == 0x40);
const _: () = assert!(offset_of!(Operational, crcr) == 0x18);
const _: () = assert!(offset_of!(Operational, dcbaap) == 0x30);
const _: () = assert!(offset_of!(Operational, config) == 0x38);

/// One interrupter's runtime registers.
#[repr(C)]
struct Interrupter {
    iman: u32,
    imod: u32,
    erstsz: u32,
    _reserved0c: u32,
    erstba: u64,
    erdp: u64,
}

/// The runtime registers: a microframe counter, then the interrupters.
#[repr(C)]
struct Runtime {
    microframe_index: u32,
    _reserved: [u32; 7],
    interrupter0: Interrupter,
}
const _: () = assert!(offset_of!(Runtime, interrupter0) == 0x20);
const _: () = assert!(offset_of!(Interrupter, erstsz) == 0x08);
const _: () = assert!(offset_of!(Interrupter, erstba) == 0x10);
const _: () = assert!(offset_of!(Interrupter, erdp) == 0x18);

/// Where the port register set starts, relative to `CAPLENGTH`.
const PORT_REGISTERS: usize = 0x400;
/// One port's register set.
#[repr(C)]
struct Port {
    portsc: u32,
    portpmsc: u32,
    portli: u32,
    porthlpmc: u32,
}
const _: () = assert!(size_of::<Port>() == 0x10);

bitfield! {
    /// `USBCMD`: whether the controller runs, and whether it is reset.
    UsbCmd: u32 {
        run: 0, 0 -> set_run,
        reset: 1, 1 -> set_reset,
    }
}

bitfield! {
    /// `USBSTS`: what the controller is doing.
    UsbSts: u32 {
        halted: 0, 0,
        not_ready: 11, 11,
    }
}

bitfield! {
    /// `HCCPARAMS1`: what the controller supports.
    HccParams1: u32 {
        context_size: 2, 2,
    }
}

bitfield! {
    /// `PORTSC`: a port's status and control word.
    Portsc: u32 {
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

/// A setup packet is eight bytes.
const SETUP_LENGTH: usize = 8;

/// 32-byte contexts: one slot context, one endpoint context per endpoint.
const CONTEXT_SIZE: usize = 0x20;
/// Words in a page of contexts.
const CONTEXT_WORDS: usize = crate::paging::PAGE_SIZE as usize / 4;
/// How long to wait for the controller to reach a state, in milliseconds.
const TIMEOUT_MS: u64 = 100;
/// TRBs in the command ring and in the event ring segment.
const RING_TRBS: usize = 256;
/// Most device slots the controller may report (`MaxSlots` is one byte).
const MAX_SLOTS: usize = 256;
/// Most scratchpad buffers the controller may report (the count is ten bits).
const MAX_SCRATCHPAD: usize = 1023;

/// A transfer request block (`TRB`): 16 bytes the controller and the driver hand
/// to each other through a ring.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Trb {
    parameter: u64,
    status: u32,
    control: u32,
}

// SAFETY: a TRB is four integers, so every bit pattern is valid.
unsafe impl Zeroable for Trb {}

/// What a TRB asks for or reports (xHCI specification, table 6-91).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrbType {
    Normal,
    SetupStage,
    DataStage,
    StatusStage,
    Link,
    EnableSlot,
    DisableSlot,
    AddressDevice,
    ConfigureEndpoint,
    NoOpCommand,
    TransferEvent,
    CommandCompletion,
    PortStatusChange,
    Other(u32),
}

impl TrbType {
    fn raw(self) -> u32 {
        match self {
            Self::Normal => 1,
            Self::SetupStage => 2,
            Self::DataStage => 3,
            Self::StatusStage => 4,
            Self::Link => 6,
            Self::EnableSlot => 9,
            Self::DisableSlot => 10,
            Self::AddressDevice => 11,
            Self::ConfigureEndpoint => 12,
            Self::NoOpCommand => 23,
            Self::TransferEvent => 32,
            Self::CommandCompletion => 33,
            Self::PortStatusChange => 34,
            Self::Other(raw) => raw,
        }
    }

    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::Normal,
            2 => Self::SetupStage,
            3 => Self::DataStage,
            4 => Self::StatusStage,
            6 => Self::Link,
            9 => Self::EnableSlot,
            10 => Self::DisableSlot,
            11 => Self::AddressDevice,
            12 => Self::ConfigureEndpoint,
            23 => Self::NoOpCommand,
            32 => Self::TransferEvent,
            33 => Self::CommandCompletion,
            34 => Self::PortStatusChange,
            other => Self::Other(other),
        }
    }
}

bitfield! {
    /// The control dword of a TRB: what kind it is and which lap it belongs to.
    TrbControl: u32 {
        cycle: 0, 0,
        link_toggle: 1, 1,
        trb_type: 15, 10,
        endpoint: 20, 16,
        slot: 31, 24,
    }
}

bitfield! {
    /// The status dword of a transfer event: how much was left and how it went.
    TransferEventStatus: u32 {
        residual_length: 23, 0,
        completion_code: 31, 24,
    }
}

fn trb_type(trb: &Trb) -> TrbType {
    TrbType::from_raw(TrbControl::new(trb.control).trb_type())
}

fn completion_code(trb: &Trb) -> CompletionCode {
    CompletionCode::from_raw((trb.status >> 24) & 0xff)
}

/// The slot a completion event was about.
fn slot_id(trb: &Trb) -> u32 {
    TrbControl::new(trb.control).slot()
}

impl Trb {
    fn no_op_command() -> Self {
        Self {
            control: TrbType::NoOpCommand.raw() << 10,
            ..Self::default()
        }
    }

    /// `Address Device`: the input context at `input` describes the device.
    fn address_device(input_context: u64, slot: u32) -> Self {
        Self {
            parameter: input_context,
            status: 0,
            control: (TrbType::AddressDevice.raw() << 10) | (slot << 24),
        }
    }

    /// `Disable Slot`: frees a slot the device was using.
    fn disable_slot(slot: u32) -> Self {
        Self {
            control: (TrbType::DisableSlot.raw() << 10) | (slot << 24),
            ..Self::default()
        }
    }

    /// `Enable Slot`: the controller answers with the slot it picked.
    fn enable_slot(slot_type: u32) -> Self {
        Self {
            control: (TrbType::EnableSlot.raw() << 10) | (slot_type << 16),
            ..Self::default()
        }
    }

    /// `Normal` (TRB type 1): moves one buffer's worth of data on an endpoint.
    fn normal(buffer: u64, length: u32) -> Self {
        Self {
            parameter: buffer,
            status: length,
            control: (TrbType::Normal.raw() << 10) | (1 << 5),
        }
    }

    /// `Configure Endpoint` (TRB type 12): adds the endpoints an input context
    /// describes to a device that already has an address.
    fn configure_endpoint(input_context: u64, slot: u32) -> Self {
        Self {
            parameter: input_context,
            status: 0,
            control: (TrbType::ConfigureEndpoint.raw() << 10) | (slot << 24),
        }
    }

    /// `Setup Stage` (TRB type 2): the eight request bytes travel in the TRB
    /// itself, which is what the IDT bit says.
    fn setup_stage(request: u64) -> Self {
        Self {
            parameter: request,
            status: SETUP_LENGTH as u32,
            control: (TrbType::SetupStage.raw() << 10) | (1 << 6),
        }
    }

    /// `Data Stage` (TRB type 3): moves `length` bytes to or from `buffer`.
    fn data_stage(buffer: u64, length: u32, direction_in: bool) -> Self {
        Self {
            parameter: buffer,
            status: length,
            control: (TrbType::DataStage.raw() << 10) | (u32::from(direction_in) << 16),
        }
    }

    /// `Status Stage` (TRB type 4): the empty last stage of a control transfer.
    ///
    /// It carries Interrupt On Completion, so the controller reports the whole
    /// transfer once this stage finishes.
    fn status_stage(direction_in: bool) -> Self {
        Self {
            parameter: 0,
            status: 0,
            control: (TrbType::StatusStage.raw() << 10)
                | (u32::from(direction_in) << 16)
                | (1 << 5),
        }
    }

    /// Points the ring's last slot back at `ring` and flips the cycle state, so
    /// the controller can tell a fresh lap from the previous one.
    fn link(ring: u64) -> Self {
        Self {
            parameter: ring,
            status: 0,
            control: (TrbType::Link.raw() << 10) | (1 << 1),
        }
    }
}

/// One entry of the event ring segment table (`ERST`): where a segment is and
/// how many TRBs it holds.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ErstEntry {
    ring_segment_base_address: u64,
    ring_segment_size: u32,
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
        let caplength = u64::from(
            window
                .at::<u8>(offset_of!(Capability, caplength_and_version))
                .read(),
        );
        if caplength == 0 {
            return Err("the xHCI controller reports a zero CAPLENGTH");
        }
        let dboff = u64::from(window.at::<u32>(offset_of!(Capability, dboff)).read());
        let rtsoff = u64::from(window.at::<u32>(offset_of!(Capability, rtsoff)).read());
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
        self.window
            .at(self.rtsoff as usize + offset_of!(Runtime, interrupter0) + offset)
    }

    /// One entry of the doorbell array, relative to `DBOFF`.
    fn doorbell(&self, slot: u32) -> Mmio<u32> {
        self.window.at(self.dboff as usize + slot as usize * 4)
    }

    /// The status register of port `port` (numbered from 1).
    fn port_status(&self, port: u32) -> Mmio<u32> {
        let offset = PORT_REGISTERS + (port as usize - 1) * size_of::<Port>();
        self.operational(offset)
    }

    /// The version the controller reports, in BCD (`0x0100` is 1.0).
    pub fn version(&self) -> u16 {
        // The capability dword holds CAPLENGTH, a reserved byte, then the
        // version, so read the whole dword and shift the version out.
        (self.read_cap(offset_of!(Capability, caplength_and_version)) >> 16) as u16
    }

    fn capability_value(&self) -> Capability {
        let _ = offset_of!(Capability, caplength_and_version);
        Capability {
            caplength_and_version: self
                .read_cap(offset_of!(Capability, caplength_and_version)),
            hcsparams1: self.read_cap(offset_of!(Capability, hcsparams1)),
            hcsparams2: self.read_cap(offset_of!(Capability, hcsparams2)),
            hcsparams3: self.read_cap(offset_of!(Capability, hcsparams3)),
            hccparams1: self.read_cap(offset_of!(Capability, hccparams1)),
            dboff: self.read_cap(offset_of!(Capability, dboff)),
            rtsoff: self.read_cap(offset_of!(Capability, rtsoff)),
            hccparams2: self.read_cap(offset_of!(Capability, hccparams2)),
        }
    }

    fn usbcmd(&self) -> UsbCmd {
        UsbCmd::new(self.read_op(offset_of!(Operational, usbcmd)))
    }

    fn set_usbcmd(&self, command: UsbCmd) {
        self.write_op(offset_of!(Operational, usbcmd), command.raw());
    }

    fn usbsts(&self) -> UsbSts {
        UsbSts::new(self.read_op(offset_of!(Operational, usbsts)))
    }

    /// Waits until `done` holds, or fails with `what` after the timeout.
    fn wait_until(
        &self,
        what: &'static str,
        mut done: impl FnMut() -> bool,
    ) -> Result<()> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        while !done() {
            if timer::nanoseconds() >= deadline {
                return Err(what);
            }
            core::hint::spin_loop();
        }
        Ok(())
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

    fn ring_command_doorbell(&self) {
        self.doorbell(0).write(0);
    }

    /// Tells the controller that endpoint `endpoint` of `slot` has work.
    fn ring_endpoint_doorbell(&self, slot: u32, endpoint: u32) {
        self.doorbell(slot).write(endpoint);
    }
}

/// Brings the controller up inside a task: it proves the command path, addresses
/// the devices already on the ports, and then watches for changes.
pub fn start(registers: Registers, mut tables: PageTables) {
    executor::spawn(async move {
        if let Err(message) = run(registers, &mut tables).await {
            error!("xhci: {message}");
        }
    });
}

async fn run(registers: Registers, tables: &mut PageTables) -> Result<()> {
    let mut host = HostController::init(registers, tables)?;
    info!("xhci: version {:#06x}", host.version());
    host.check_with_no_op().await?;
    host.enumerate_ports(tables).await?;
    host.watch(tables).await
}

/// A ring of TRBs that the controller walks at its own pace.
///
/// The driver writes entries at `index`; the cycle bit on each entry marks the
/// lap it belongs to, so both sides can tell a fresh entry from one left over
/// from an earlier lap.
struct Ring<const N: usize> {
    entries: IoBox<[Trb; N]>,
    next_index: usize,
    cycle_state: bool,
}

impl<const N: usize> Ring<N> {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            entries: IoBox::new(tables),
            next_index: 0,
            cycle_state: true,
        }
    }

    fn address(&self) -> u64 {
        self.entries.address()
    }

    /// Where the next entry sits, in bytes from the ring's address.
    fn offset(&self) -> u64 {
        (self.next_index * core::mem::size_of::<Trb>()) as u64
    }

    /// Hands over one entry, stamped with the cycle bit of the current lap.
    ///
    /// Reaching the link closes the lap: the link is rewritten for the new cycle
    /// and the ring starts over at its first entry.
    fn enqueue(&mut self, trb: Trb) -> Result<()> {
        if self.next_index + 1 == N {
            self.cycle_state = !self.cycle_state;
            self.link_to_start();
            self.next_index = 0;
        }
        let mut trb = trb;
        trb.control = (trb.control & !1) | u32::from(self.cycle_state);
        self.entries
            .write_slot(self.next_index, trb)
            .ok_or("the ring has no room")?;
        self.next_index += 1;
        Ok(())
    }

    /// The next entry whose cycle bit says it belongs to this lap.
    fn dequeue(&mut self) -> Option<Trb> {
        let entry = self.entries.read_slot(self.next_index)?;
        if (entry.control & 1 != 0) != self.cycle_state {
            return None;
        }
        self.next_index += 1;
        if self.next_index == N {
            self.next_index = 0;
            self.cycle_state = !self.cycle_state;
        }
        Some(entry)
    }

    /// Points the last slot back at the first, closing the ring.
    fn link_to_start(&mut self) {
        let mut link = Trb::link(self.entries.address());
        link.control |= u32::from(self.cycle_state);
        self.entries.write_slot(N - 1, link);
    }
}

/// The scratchpad buffers: the array of their addresses, and the pages it names.
type Scratchpad = (IoBox<[u64; MAX_SCRATCHPAD]>, Vec<IoBox<[u8; 4096]>>);

/// The root hub's ports and what they report.
struct Ports {
    registers: Registers,
    number_of_ports: u32,
}

impl Ports {
    fn number_of_ports(&self) -> u32 {
        self.number_of_ports
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
    endpoint0_ring: Ring<RING_TRBS>,
    /// Where the controller keeps the device's contexts.
    _device_context: IoBox<[u32; CONTEXT_WORDS]>,
    /// The input context the device was addressed with.
    _input_context: IoBox<[u32; CONTEXT_WORDS]>,
    /// The interrupt endpoint, once the configuration is set.
    interrupt: Option<InterruptEndpoint>,
}

/// An interrupt IN endpoint: the ring the reports arrive on, and the buffer one
/// report is written into.
struct InterruptEndpoint {
    endpoint_id: u32,
    ring: Ring<RING_TRBS>,
    buffer: IoBox<[u8; 8]>,
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

    fn link_to_start(&mut self) {
        self.ring.link_to_start();
    }

    fn send(&mut self, registers: &Registers, trb: Trb) -> Result<()> {
        self.ring.enqueue(trb)?;
        registers.ring_command_doorbell();
        Ok(())
    }
}

/// The event ring, the table that describes it, and the read side of the
/// controller's answers.
struct EventChannel {
    ring: Ring<RING_TRBS>,
    event_ring_segment_table: IoBox<[ErstEntry; 1]>,
    /// Ports the controller reported a change on, waiting to be handled.
    port_changes: Vec<u32>,
}

impl EventChannel {
    fn new(tables: &mut PageTables) -> Self {
        Self {
            ring: Ring::new(tables),
            event_ring_segment_table: IoBox::new(tables),
            port_changes: Vec::new(),
        }
    }

    /// Points the controller at the segment table and the ring.
    fn program(&mut self, registers: &Registers) -> Result<()> {
        self.event_ring_segment_table
            .write_slot(
                0,
                ErstEntry {
                    ring_segment_base_address: self.ring.address(),
                    ring_segment_size: RING_TRBS as u32,
                    _reserved: 0,
                },
            )
            .ok_or("the segment table has no room")?;
        registers.write_rt(offset_of!(Interrupter, erstsz), 1);
        registers.write_rt64(
            offset_of!(Interrupter, erstba),
            self.event_ring_segment_table.address(),
        );
        registers.write_rt64(offset_of!(Interrupter, erdp), self.ring.address());
        Ok(())
    }

    /// Takes one event if the controller has written one.
    fn take(&mut self, registers: &Registers) -> Option<Trb> {
        let event = self.ring.dequeue()?;
        let dequeue = self.ring.address() + self.ring.offset();
        registers.write_rt64(offset_of!(Interrupter, erdp), dequeue);
        Some(event)
    }

    /// Waits for a command's completion.
    ///
    /// Port changes are recorded rather than returned: they are the host's
    /// business, not the command's. Waiting yields to the other tasks instead of
    /// spinning.
    async fn wait_for_completion(&mut self, registers: &Registers) -> Result<Trb> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            while let Some(event) = self.take(registers) {
                match trb_type(&event) {
                    TrbType::CommandCompletion => return Ok(event),
                    TrbType::PortStatusChange => {
                        self.port_changes.push(event.parameter as u32 >> 24);
                    }
                    _ => continue,
                }
            }
            if timer::nanoseconds() >= deadline {
                return Err("the xHCI controller sent no event");
            }
            timer::sleep(core::time::Duration::from_millis(1)).await;
        }
    }
}

bitfield! {
    /// Slot context word 0: the device's speed and how many contexts follow.
    SlotContext0: u32 {
        route_string: 19, 0,
        speed: 23, 20 -> set_speed,
        context_entries: 31, 27 -> set_context_entries,
    }
}

bitfield! {
    /// Slot context word 1: where the device sits.
    SlotContext1: u32 {
        max_exit_latency: 15, 0,
        root_hub_port: 23, 16 -> set_root_hub_port,
    }
}

bitfield! {
    /// Slot context word 3: what the controller reports about the device.
    SlotContext3: u32 {
        device_address: 7, 0,
        slot_state: 31, 27,
    }
}

bitfield! {
    /// Endpoint context word 0: how often an interrupt endpoint is polled.
    EndpointContext0: u32 {
        interval: 23, 16 -> set_interval,
    }
}

bitfield! {
    /// Endpoint context word 1: what the endpoint is and how it behaves.
    EndpointContext1: u32 {
        cerr: 2, 1 -> set_cerr,
        ep_type: 5, 3 -> set_ep_type,
        max_packet: 31, 16 -> set_max_packet,
    }
}

bitfield! {
    /// Endpoint context word 4: the average TRB length and the periodic payload.
    EndpointContext4: u32 {
        average_trb_length: 15, 0 -> set_average_trb_length,
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

    /// The word where endpoint `dci`'s context starts.
    fn endpoint_context_words(dci: usize) -> usize {
        // One context for the input control and one for the slot come first.
        8 + 8 * dci
    }

    /// Adds `dci` (and the slot context) to this command.
    fn add_context(&mut self, dci: usize) -> Result<()> {
        let flags = 0b1 | (1 << dci);
        self.memory
            .write_slot(1, flags)
            .ok_or("the input context is too small")?;
        Ok(())
    }

    /// Speed, one context entry, and the root hub port the device sits behind.
    fn set_slot(&mut self, speed: Speed, port: u32) -> Result<()> {
        let slot = CONTEXT_SIZE / 4;
        let mut word0 = SlotContext0::new(
            self.memory
                .read_slot(slot)
                .ok_or("the input context is too small")?,
        );
        word0.set_speed(speed.psi());
        word0.set_context_entries(1);
        self.memory
            .write_slot(slot, word0.raw())
            .ok_or("the input context is too small")?;

        let mut word1 = SlotContext1::new(
            self.memory
                .read_slot(slot + 1)
                .ok_or("the input context is too small")?,
        );
        word1.set_root_hub_port(port);
        self.memory
            .write_slot(slot + 1, word1.raw())
            .ok_or("the input context is too small")?;
        Ok(())
    }

    /// Endpoint 0: control, three retries, its packet size and its ring.
    fn set_endpoint0(&mut self, max_packet: u16, ring: u64) -> Result<()> {
        self.set_endpoint(1, 4, max_packet, 0, ring)
    }

    /// The index of the last valid endpoint context, which sizes the slot.
    fn set_context_entries(&mut self, dci: usize) -> Result<()> {
        let slot = CONTEXT_SIZE / 4;
        let mut word0 = SlotContext0::new(
            self.memory
                .read_slot(slot)
                .ok_or("the input context is too small")?,
        );
        word0.set_context_entries(dci as u32);
        self.memory
            .write_slot(slot, word0.raw())
            .ok_or("the input context is too small")?;
        Ok(())
    }

    /// An endpoint with `endpoint_type`, its packet size, its polling interval
    /// and the ring it reads.
    fn set_endpoint(
        &mut self,
        dci: usize,
        endpoint_type: u32,
        max_packet: u16,
        interval: u8,
        ring: u64,
    ) -> Result<()> {
        let ep0 = Self::endpoint_context_words(dci);
        let mut word0 = EndpointContext0::new(
            self.memory
                .read_slot(ep0)
                .ok_or("the input context is too small")?,
        );
        word0.set_interval(u32::from(interval));
        self.memory
            .write_slot(ep0, word0.raw())
            .ok_or("the input context is too small")?;
        let mut word1 = EndpointContext1::new(
            self.memory
                .read_slot(ep0 + 1)
                .ok_or("the input context is too small")?,
        );
        word1.set_max_packet(u32::from(max_packet));
        word1.set_ep_type(endpoint_type);
        word1.set_cerr(3);
        self.memory
            .write_slot(ep0 + 1, word1.raw())
            .ok_or("the input context is too small")?;

        // The dequeue pointer keeps its low four bits for the cycle state.
        self.memory
            .write_slot(ep0 + 2, (ring as u32 & !0xf) | 1)
            .ok_or("the input context is too small")?;
        self.memory
            .write_slot(ep0 + 3, (ring >> 32) as u32)
            .ok_or("the input context is too small")?;

        let mut word4 = EndpointContext4::new(
            self.memory
                .read_slot(ep0 + 4)
                .ok_or("the input context is too small")?,
        );
        // Control endpoints use an average TRB length of 8.
        word4.set_average_trb_length(8);
        self.memory
            .write_slot(ep0 + 4, word4.raw())
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
    /// Device context base address array (`DCBAA`): where each slot's context
    /// lives, and entry 0 the scratchpad buffers.
    dcbaa: IoBox<[u64; MAX_SLOTS]>,
    _scratchpad_buffers: Option<Scratchpad>,
    devices: Vec<Device>,
}

impl HostController {
    /// Takes the controller over: halt it, reset it, give it its rings and start
    /// it, then prove the whole chain with a no-op command.
    pub fn init(registers: Registers, tables: &mut PageTables) -> Result<Self> {
        let capability = registers.capability_value();
        let max_slots = capability.hcsparams1 & 0xff;
        let max_interrupters = (capability.hcsparams1 >> 8) & 0x7ff;
        let max_ports = (capability.hcsparams1 >> 24) & 0xff;
        let scratchpad = scratchpad_count(capability.hcsparams2);
        if HccParams1::new(capability.hccparams1).context_size() != 0 {
            return Err("this driver only supports 32-byte contexts");
        }
        info!(
            "xhci: {max_slots} slots, {max_interrupters} interrupters, {max_ports} ports, \
             {scratchpad} scratchpad buffers"
        );

        // The firmware may have left the controller running with its own rings;
        // stop it before taking them away.
        let mut command = registers.usbcmd();
        command.set_run(0);
        registers.set_usbcmd(command);
        registers.wait_until("the controller did not stop", || {
            registers.usbsts().halted() == 1
        })?;

        command = registers.usbcmd();
        command.set_reset(1);
        registers.set_usbcmd(command);
        registers.wait_until("the controller did not finish resetting", || {
            registers.usbcmd().reset() == 0
        })?;
        registers.wait_until("the controller is not ready", || {
            registers.usbsts().not_ready() == 0
        })?;
        registers.write_op(offset_of!(Operational, pagesize), 1);

        let mut controller = Self {
            registers,
            commands: CommandChannel::new(tables),
            events: EventChannel::new(tables),
            ports: Ports {
                registers,
                number_of_ports: max_ports,
            },
            dcbaa: IoBox::new(tables),
            _scratchpad_buffers: None,
            devices: Vec::new(),
        };
        controller.program(max_slots, scratchpad, tables)?;
        controller.start()?;
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
            let mut scratchpad_buffers = Vec::new();
            for index in 0..scratchpad as usize {
                let buffer = IoBox::<[u8; 4096]>::new(tables);
                buffers
                    .write_slot(index, buffer.address())
                    .ok_or("the scratchpad array is full")?;
                scratchpad_buffers.push(buffer);
            }
            self.dcbaa
                .write_slot(0, buffers.address())
                .ok_or("the device context array is full")?;
            self._scratchpad_buffers = Some((buffers, scratchpad_buffers));
        }
        self.registers
            .write_op64(offset_of!(Operational, dcbaap), self.dcbaa.address());
        self.registers
            .write_op(offset_of!(Operational, config), max_slots & 0xff);
        self.events.program(&self.registers)?;
        self.commands.link_to_start();
        self.registers
            .write_op64(offset_of!(Operational, crcr), self.commands.address() | 1);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let mut command = self.registers.usbcmd();
        command.set_run(1);
        self.registers.set_usbcmd(command);
        self.registers
            .wait_until("the controller did not start", || {
                self.registers.usbsts().halted() == 0
            })
    }

    /// The version the controller reports, in BCD.
    pub fn version(&self) -> u16 {
        self.registers.version()
    }

    /// Port numbers the controller reports, from 1.
    pub fn number_of_ports(&self) -> u32 {
        self.ports.number_of_ports()
    }

    /// Sends a no-op command and waits for its completion event: if that arrives,
    /// the doorbell, the command ring, the event ring and the completion path all
    /// work.
    async fn check_with_no_op(&mut self) -> Result<()> {
        let trb = Trb::no_op_command();
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers).await?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the no-op command did not complete successfully");
        }
        info!("xhci: started, and the no-op command completed");
        Ok(())
    }

    /// Finds the devices already on the ports and addresses them.
    pub async fn enumerate_ports(&mut self, tables: &mut PageTables) -> Result<()> {
        for port in 1..=self.ports.number_of_ports() {
            let status = self.ports.status(port);
            if status.connected() == 0 {
                continue;
            }
            self.attach_device(tables, port).await?;
        }
        Ok(())
    }

    /// Resets the port, gives the device a slot and an address.
    async fn attach_device(&mut self, tables: &mut PageTables, port: u32) -> Result<()> {
        let status = self.ports.status(port);
        self.ports.acknowledge(port, status);
        info!(
            "xhci: port {port} has a device (portsc {:#010x})",
            status.raw()
        );
        self.ports.reset(port)?;
        let after = self.ports.status(port);
        // Slot type 0 is a USB2 device; a SuperSpeed port asks for type 1.
        let slot = self.enable_slot(0).await?;
        info!(
            "xhci: port {port} reset (portsc {:#010x}), slot {slot} assigned",
            after.raw()
        );
        self.address_device(tables, slot, port).await?;
        let descriptor = self.read_device_descriptor(tables, slot).await?;
        info!(
            "xhci: slot {slot} is device {:#06x}:{:#06x}, class {:#04x}, {} configuration(s)",
            descriptor.vendor_id,
            descriptor.product_id,
            descriptor.class,
            descriptor.number_of_configurations
        );
        let (configuration_value, interrupt_endpoint) =
            self.read_configuration(tables, slot).await?;
        self.set_configuration(slot, configuration_value).await?;
        if let Some(endpoint) = interrupt_endpoint {
            self.configure_endpoint(tables, slot, endpoint).await?;
        }
        Ok(())
    }

    /// Keeps reading events and handles any port that changed while the OS runs.
    async fn watch(&mut self, tables: &mut PageTables) -> Result<()> {
        loop {
            // The controller writes its events without telling the CPU; a short
            // sleep keeps the polling from starving the other tasks.
            timer::sleep(core::time::Duration::from_millis(1)).await;
            while let Some(event) = self.events.take(&self.registers) {
                match trb_type(&event) {
                    TrbType::PortStatusChange => {
                        self.handle_port_change(tables, event.parameter as u32 >> 24)
                            .await?;
                    }
                    TrbType::TransferEvent => {
                        let control = TrbControl::new(event.control);
                        if control.endpoint() != 1 {
                            self.handle_report(control.slot(), control.endpoint())?;
                        }
                    }
                    other => info!("xhci: event {other:?} with nothing waiting for it"),
                }
            }
            while let Some(port) = self.events.port_changes.pop() {
                self.handle_port_change(tables, port).await?;
            }
        }
    }

    async fn handle_port_change(
        &mut self,
        tables: &mut PageTables,
        port: u32,
    ) -> Result<()> {
        let status = self.ports.status(port);
        self.ports.acknowledge(port, status);
        if status.connected() == 0 {
            if let Some(position) =
                self.devices.iter().position(|device| device.port == port)
            {
                let device = self.devices.remove(position);
                self.disable_slot(device.slot).await?;
                info!(
                    "xhci: port {port} lost its device, slot {} freed",
                    device.slot
                );
            }
            return Ok(());
        }
        if self.devices.iter().any(|device| device.port == port) {
            return Ok(());
        }
        self.attach_device(tables, port).await
    }

    /// The bytes the controller wrote into a descriptor buffer.
    ///
    /// # Safety
    ///
    /// The buffer must be a page-aligned `IoBox` of at least `length` bytes that
    /// the controller has finished writing.
    unsafe fn descriptor_bytes(address: u64, length: usize) -> &'static [u8] {
        // SAFETY: the caller guarantees the lifetime and the length.
        unsafe { core::slice::from_raw_parts(address as *const u8, length) }
    }

    /// Reads the device descriptor over endpoint 0.
    async fn read_device_descriptor(
        &mut self,
        tables: &mut PageTables,
        slot: u32,
    ) -> Result<DeviceDescriptor> {
        let buffer = IoBox::<[u8; 64]>::new(tables);
        let length = DeviceDescriptor::LENGTH;
        let request = control::get_descriptor(descriptor::DEVICE, 0, length as u16);
        self.control_in(slot, request, buffer.address(), length as u32)
            .await?;
        // SAFETY: the transfer wrote `length` bytes into the buffer.
        let bytes = unsafe { Self::descriptor_bytes(buffer.address(), length) };
        DeviceDescriptor::from_bytes(bytes).ok_or("the device descriptor was too short")
    }

    /// Reads the configuration and logs the interfaces and endpoints it holds.
    async fn read_configuration(
        &mut self,
        tables: &mut PageTables,
        slot: u32,
    ) -> Result<(u8, Option<EndpointDescriptor>)> {
        let buffer = IoBox::<[u8; 512]>::new(tables);
        // The header says how long the whole configuration is.
        let header_length = ConfigurationDescriptor::LENGTH;
        let request =
            control::get_descriptor(descriptor::CONFIGURATION, 0, header_length as u16);
        self.control_in(slot, request, buffer.address(), header_length as u32)
            .await?;
        // SAFETY: the transfer wrote the header into the buffer.
        let header = unsafe { Self::descriptor_bytes(buffer.address(), header_length) };
        let configuration = ConfigurationDescriptor::from_bytes(header)
            .ok_or("the configuration descriptor was too short")?;
        let total_length = usize::from(configuration.total_length);
        if total_length > 512 {
            return Err("the configuration descriptor is larger than the buffer");
        }

        let request =
            control::get_descriptor(descriptor::CONFIGURATION, 0, total_length as u16);
        self.control_in(slot, request, buffer.address(), total_length as u32)
            .await?;
        // SAFETY: the transfer wrote the whole configuration into the buffer.
        let bytes = unsafe { Self::descriptor_bytes(buffer.address(), total_length) };
        info!(
            "xhci: slot {slot} configuration {}: {} interface(s), {} bytes",
            configuration.configuration_value,
            configuration.number_of_interfaces,
            configuration.total_length
        );

        let mut interrupt_endpoint = None;
        for raw in descriptors(bytes) {
            match raw.kind {
                descriptor::INTERFACE => {
                    if let Some(interface) = InterfaceDescriptor::from_bytes(raw.bytes) {
                        info!(
                            "xhci: slot {slot} interface {}: class {}.{}.{}, {} endpoint(s)",
                            interface.interface_number,
                            interface.class,
                            interface.subclass,
                            interface.protocol,
                            interface.number_of_endpoints
                        );
                    }
                }
                descriptor::ENDPOINT => {
                    if let Some(endpoint) = EndpointDescriptor::from_bytes(raw.bytes) {
                        info!(
                            "xhci: slot {slot} endpoint {:#04x}: {}, {} bytes every {}",
                            endpoint.address,
                            transfer_type_name(
                                endpoint.transfer_type(),
                                endpoint.direction_in()
                            ),
                            endpoint.max_packet_size,
                            endpoint.interval
                        );
                        if endpoint.transfer_type() == 3 && endpoint.direction_in() {
                            interrupt_endpoint = Some(endpoint);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok((configuration.configuration_value, interrupt_endpoint))
    }

    fn device_position(&self, slot: u32) -> Result<usize> {
        self.devices
            .iter()
            .position(|device| device.slot == slot)
            .ok_or("no device has that slot")
    }

    /// Sends a control-IN transfer on endpoint 0 and waits for it to finish.
    async fn control_in(
        &mut self,
        slot: u32,
        request: u64,
        buffer: u64,
        length: u32,
    ) -> Result<()> {
        let position = self.device_position(slot)?;
        {
            let ring = &mut self.devices[position].endpoint0_ring;
            ring.enqueue(Trb::setup_stage(request))?;
            ring.enqueue(Trb::data_stage(buffer, length, true))?;
            ring.enqueue(Trb::status_stage(false))?;
        }
        self.registers.ring_endpoint_doorbell(slot, 1);
        self.wait_transfer(slot, 1).await
    }

    /// Waits until the controller reports a transfer on `endpoint` of `slot`.
    async fn wait_transfer(&mut self, slot: u32, endpoint: u32) -> Result<()> {
        let deadline = timer::nanoseconds() + TIMEOUT_MS * 1_000_000;
        loop {
            while let Some(event) = self.events.take(&self.registers) {
                match trb_type(&event) {
                    TrbType::TransferEvent => {
                        let control = TrbControl::new(event.control);
                        if control.slot() == slot && control.endpoint() == endpoint {
                            let status = TransferEventStatus::new(event.status);
                            let code = CompletionCode::from_raw(status.completion_code());
                            if code != CompletionCode::Success {
                                error!("xhci: transfer failed with {code:?}");
                                return Err("the transfer failed");
                            }
                            return Ok(());
                        }
                    }
                    TrbType::PortStatusChange => {
                        self.events.port_changes.push(event.parameter as u32 >> 24);
                    }
                    other => info!("xhci: event {other:?} with nothing waiting for it"),
                }
            }
            if timer::nanoseconds() >= deadline {
                return Err("the transfer timed out");
            }
            timer::sleep(core::time::Duration::from_millis(1)).await;
        }
    }

    /// Selects a configuration (USB 2.0, 9.4.7): the request carries no data.
    async fn set_configuration(&mut self, slot: u32, value: u8) -> Result<()> {
        let request = 0x09u64 << 8 | u64::from(value) << 16;
        let position = self.device_position(slot)?;
        {
            let ring = &mut self.devices[position].endpoint0_ring;
            ring.enqueue(Trb::setup_stage(request))?;
            ring.enqueue(Trb::status_stage(true))?;
        }
        self.registers.ring_endpoint_doorbell(slot, 1);
        self.wait_transfer(slot, 1).await
    }

    /// Adds the interrupt endpoint of the selected configuration.
    async fn configure_endpoint(
        &mut self,
        tables: &mut PageTables,
        slot: u32,
        endpoint: EndpointDescriptor,
    ) -> Result<()> {
        let position = self.device_position(slot)?;
        let port = self.devices[position].port;
        let port_speed = self.ports.speed(port)?;
        // Endpoint `n` IN is DCI 2n+1.
        let endpoint_id =
            u32::from(endpoint.number()) * 2 + u32::from(endpoint.direction_in());
        let ring = Ring::<RING_TRBS>::new(tables);
        let buffer = IoBox::<[u8; 8]>::new(tables);

        let mut input_context = InputContext::new(tables);
        input_context.add_context(endpoint_id as usize)?;
        input_context.set_slot(port_speed, port)?;
        input_context.set_context_entries(endpoint_id as usize)?;
        // Type 7 is an interrupt IN endpoint (xHCI 6.2.3).
        input_context.set_endpoint(
            endpoint_id as usize,
            7,
            endpoint.max_packet_size,
            endpoint.interval,
            ring.address(),
        )?;

        let trb = Trb::configure_endpoint(input_context.address(), slot);
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers).await?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the configure endpoint command failed");
        }
        self.devices[position].interrupt = Some(InterruptEndpoint {
            endpoint_id,
            ring,
            buffer,
        });
        self.arm_interrupt(position);
        info!(
            "xhci: slot {slot} endpoint {:#04x} configured, {} byte reports",
            endpoint.address, endpoint.max_packet_size
        );
        Ok(())
    }

    /// Queues one report buffer on the interrupt endpoint.
    fn arm_interrupt(&mut self, position: usize) {
        let device = &mut self.devices[position];
        let Some(interrupt) = device.interrupt.as_mut() else {
            return;
        };
        if interrupt
            .ring
            .enqueue(Trb::normal(interrupt.buffer.address(), 8))
            .is_err()
        {
            return;
        }
        let slot = device.slot;
        let endpoint = interrupt.endpoint_id;
        self.registers.ring_endpoint_doorbell(slot, endpoint);
    }

    /// Reads a report the interrupt endpoint delivered and queues the next one.
    fn handle_report(&mut self, slot: u32, endpoint_id: u32) -> Result<()> {
        let Ok(position) = self.device_position(slot) else {
            return Ok(());
        };
        let keys = {
            let device = &self.devices[position];
            let Some(interrupt) = device.interrupt.as_ref() else {
                return Ok(());
            };
            if interrupt.endpoint_id != endpoint_id {
                return Ok(());
            }
            // SAFETY: the buffer is page aligned and the controller wrote the
            // eight bytes of a boot-protocol report into it.
            let report = unsafe { Self::descriptor_bytes(interrupt.buffer.address(), 8) };
            hid::keys(report)
        };
        for key in keys.iter().flatten() {
            info!("xhci: slot {slot} key {key:?}");
        }
        self.arm_interrupt(position);
        Ok(())
    }

    /// Frees the slot a device was using.
    async fn disable_slot(&mut self, slot: u32) -> Result<()> {
        let trb = Trb::disable_slot(slot);
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers).await?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the disable slot command failed");
        }
        Ok(())
    }

    /// Asks the controller for a slot, which is where a device's contexts live.
    async fn enable_slot(&mut self, slot_type: u32) -> Result<u32> {
        let trb = Trb::enable_slot(slot_type);
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers).await?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the enable slot command failed");
        }
        Ok(slot_id(&event))
    }

    /// Gives the device on `port` an address: the input context describes the
    /// slot and endpoint 0, and the controller fills in the device context.
    async fn address_device(
        &mut self,
        tables: &mut PageTables,
        slot: u32,
        port: u32,
    ) -> Result<()> {
        let speed = self.ports.speed(port)?;
        let max_packet = speed.max_packet_size();
        let endpoint0 = Ring::<RING_TRBS>::new(tables);
        let mut input_context = InputContext::new(tables);
        let device_context = IoBox::<[u32; CONTEXT_WORDS]>::new(tables);

        input_context.add_context(1)?;
        input_context.set_slot(speed, port)?;
        input_context.set_endpoint0(max_packet, endpoint0.address())?;
        self.dcbaa
            .write_slot(slot as usize, device_context.address())
            .ok_or("the device context array is full")?;

        let trb = Trb::address_device(input_context.address(), slot);
        self.commands.send(&self.registers, trb)?;
        let event = self.events.wait_for_completion(&self.registers).await?;
        if completion_code(&event) != CompletionCode::Success {
            return Err("the address device command failed");
        }
        // The controller writes the device context, and the slot context's last
        // word carries the address it assigned.
        let slot_context = SlotContext3::new(device_context.read_slot(11).unwrap_or(0));
        info!(
            "xhci: slot {slot} has device address {}",
            slot_context.device_address()
        );
        self.devices.push(Device {
            slot,
            port,
            endpoint0_ring: endpoint0,
            _device_context: device_context,
            _input_context: input_context.memory,
            interrupt: None,
        });
        Ok(())
    }
}

/// What an endpoint's transfer type means (USB 2.0, 9.6.6, table 9-13).
fn transfer_type_name(transfer_type: u8, direction_in: bool) -> &'static str {
    match (transfer_type, direction_in) {
        (0, _) => "control",
        (1, true) => "isochronous IN",
        (1, false) => "isochronous OUT",
        (2, true) => "bulk IN",
        (2, false) => "bulk OUT",
        (3, true) => "interrupt IN",
        _ => "interrupt OUT",
    }
}

fn scratchpad_count(hcsparams2: u32) -> u32 {
    let low = (hcsparams2 >> 21) & 0x1f;
    let high = (hcsparams2 >> 27) & 0x1f;
    (high << 5) | low
}

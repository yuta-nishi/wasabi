//! PCI configuration space, reached through ECAM.
//!
//! Every function owns 4 KiB of memory-mapped configuration space, and the
//! `MCFG` ACPI table says where that window starts and which buses it covers.
//! Reading a register is therefore an ordinary memory read at
//! `base + (bus << 20 | device << 15 | function << 12) + offset`.

use crate::acpi::Mcfg;
use crate::acpi::Rsdp;
use crate::acpi::TableHeader;
use crate::info;
use crate::result::Result;
use core::fmt;
use core::ptr::read_volatile;
use core::ptr::write_volatile;

/// Offset of the command register in the standard header.
const COMMAND: u16 = 0x04;
/// Command bit 1: the function decodes the memory window its BARs describe.
const COMMAND_MEMORY: u16 = 1 << 1;
/// Command bit 2: the function may read and write memory on its own (DMA).
const COMMAND_BUS_MASTER: u16 = 1 << 2;
/// Offset of the first base address register; the six follow each other.
const BAR0: u16 = 0x10;

/// One of a function's six base address registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    /// A window in memory space that the CPU reaches with ordinary loads and
    /// stores; `size` is the window's length and always a power of two.
    Memory {
        address: u64,
        size: u64,
        prefetchable: bool,
    },
    /// A window in I/O space, reached with `in`/`out` instead of loads.
    Io { address: u32, size: u32 },
}

/// The bus, device and function numbers that address one PCI function.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Address {
    bus: u8,
    device: u8,
    function: u8,
}

impl Address {
    /// Every function on the buses `start_bus..=end_bus`.
    pub fn iter(start_bus: u8, end_bus: u8) -> impl Iterator<Item = Address> {
        (u16::from(start_bus)..=u16::from(end_bus)).flat_map(|bus| {
            (0..32u8).flat_map(move |device| {
                (0..8u8).map(move |function| Address {
                    bus: bus as u8,
                    device,
                    function,
                })
            })
        })
    }

    /// Where this function's 4 KiB window starts, relative to the ECAM base.
    const fn offset(self) -> usize {
        ((self.bus as usize) << 20)
            | ((self.device as usize) << 15)
            | ((self.function as usize) << 12)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// One ECAM window: the base address and the buses it covers.
pub struct ConfigSpace {
    base: usize,
    start_bus: u8,
    end_bus: u8,
}

impl ConfigSpace {
    /// Takes the ECAM window from `MCFG`. Only the first window is used; the
    /// table may describe more than one segment.
    pub fn from_acpi(rsdp: &Rsdp) -> Result<Self> {
        let header = rsdp
            .xsdt()
            .find(b"MCFG")
            .ok_or("no MCFG table in the XSDT")?;
        // SAFETY: `find` only returns tables whose signature is `MCFG`.
        let mcfg = unsafe { &*(header as *const TableHeader as *const Mcfg) };
        let entry = mcfg
            .entries()
            .next()
            .ok_or("MCFG describes no ECAM window")?;
        Ok(Self {
            base: entry.base_address() as usize,
            start_bus: entry.start_bus(),
            end_bus: entry.end_bus(),
        })
    }

    /// The addresses this window can reach.
    pub fn addresses(&self) -> impl Iterator<Item = Address> {
        Address::iter(self.start_bus, self.end_bus)
    }

    fn register(&self, address: Address, offset: u16) -> usize {
        self.base + address.offset() + usize::from(offset)
    }

    /// A 16-bit configuration register. Reads are volatile: the value comes from
    /// the device, not from memory the CPU may cache.
    pub fn read_u16(&self, address: Address, offset: u16) -> u16 {
        // SAFETY: ECAM entries are 4 KiB and offsets are even, so the address is
        // aligned and inside the window the firmware described.
        unsafe { read_volatile(self.register(address, offset) as *const u16) }
    }

    /// A 32-bit configuration register.
    pub fn read_u32(&self, address: Address, offset: u16) -> u32 {
        // SAFETY: as in `read_u16`; offsets are four-byte aligned.
        unsafe { read_volatile(self.register(address, offset) as *const u32) }
    }

    /// Writes a 16-bit configuration register.
    pub fn write_u16(&self, address: Address, offset: u16, value: u16) {
        // SAFETY: as in `read_u16`.
        unsafe { write_volatile(self.register(address, offset) as *mut u16, value) }
    }

    /// Writes a 32-bit configuration register.
    pub fn write_u32(&self, address: Address, offset: u16, value: u32) {
        // SAFETY: as in `read_u16`.
        unsafe { write_volatile(self.register(address, offset) as *mut u32, value) }
    }

    /// The command register (`0x04`): which spaces this function may decode and
    /// whether it may drive the bus by itself.
    pub fn command(&self, address: Address) -> u16 {
        self.read_u16(address, COMMAND)
    }

    /// Lets the function answer on its BAR window and start its own bus
    /// transactions. A device is deaf until both bits are set.
    pub fn enable(&self, address: Address) {
        self.write_u16(
            address,
            COMMAND,
            self.command(address) | COMMAND_MEMORY | COMMAND_BUS_MASTER,
        );
    }

    /// Vendor and device ID, or `None` when no function answers at `address`.
    pub fn vendor_and_device(&self, address: Address) -> Option<(u16, u16)> {
        let vendor = self.read_u16(address, 0x00);
        if vendor == 0xffff {
            return None;
        }
        Some((vendor, self.read_u16(address, 0x02)))
    }

    /// The programming interface byte (`0x09`): for a USB controller, `0x30`
    /// means xHCI, `0x20` EHCI, `0x10` OHCI and `0x00` UHCI.
    pub fn programming_interface(&self, address: Address) -> u8 {
        ((self.read_u32(address, 0x08) >> 8) & 0xff) as u8
    }

    /// The class code and its subclass (header offset `0x0b` and `0x0a`).
    pub fn class(&self, address: Address) -> (u8, u8) {
        let register = self.read_u32(address, 0x08);
        (
            ((register >> 24) & 0xff) as u8,
            ((register >> 16) & 0xff) as u8,
        )
    }

    /// Reads base address register `index` (0..6).
    ///
    /// A BAR only holds the window's start address; the length lives nowhere, so
    /// it is measured by writing all ones and reading back which bits the
    /// function keeps, then restoring what was there.
    pub fn bar(&self, address: Address, index: u8) -> Option<Bar> {
        let offset = BAR0 + u16::from(index) * 4;
        let low = self.read_u32(address, offset);
        if low & 0b1 == 0b1 {
            // I/O space: bits 2..1 are reserved zero, so they come back as 0.
            let original = low;
            self.write_u32(address, offset, !0);
            let mask = self.read_u32(address, offset) & !0b11;
            self.write_u32(address, offset, original);
            let size = (!mask).wrapping_add(1);
            return Some(Bar::Io {
                address: low & !0b11,
                size,
            });
        }
        if low == 0 {
            return None;
        }
        // Memory space: bits 2..1 say whether the window is 64-bit, and bit 3
        // whether reads have no side effects.
        let is_64bit = (low >> 1) & 0b11 == 0b10;
        let prefetchable = low & 0b1000 != 0;
        let high_offset = offset + 4;
        let high = if is_64bit {
            self.read_u32(address, high_offset)
        } else {
            0
        };
        self.write_u32(address, offset, !0);
        if is_64bit {
            self.write_u32(address, high_offset, !0);
        }
        let mask_low = self.read_u32(address, offset);
        let mask_high = if is_64bit {
            self.read_u32(address, high_offset)
        } else {
            0
        };
        self.write_u32(address, offset, low);
        if is_64bit {
            self.write_u32(address, high_offset, high);
        }
        let mask = ((u64::from(mask_high)) << 32) | u64::from(mask_low & !0b1111);
        Some(Bar::Memory {
            address: ((u64::from(high)) << 32) | u64::from(low & !0b1111),
            size: (!mask).wrapping_add(1),
            prefetchable,
        })
    }

    /// Logs every function that answers in this window, and returns the first
    /// xHCI controller on the way: class `0c.03` (USB) with programming
    /// interface `0x30`, the byte that separates xHCI from the older EHCI
    /// (`0x20`), OHCI (`0x10`) and UHCI (`0x00`) controllers.
    pub fn probe(&self) -> Option<Address> {
        info!(
            "pci: ECAM at {:#018x} covering buses {:#04x}..={:#04x}",
            self.base, self.start_bus, self.end_bus
        );
        let mut found = 0;
        let mut xhci = None;
        for address in self.addresses() {
            let Some((vendor, device)) = self.vendor_and_device(address) else {
                continue;
            };
            let (class, subclass) = self.class(address);
            info!(
                "pci: {address} {vendor:04x}:{device:04x} class {class:02x}.{subclass:02x}"
            );
            found += 1;
            if xhci.is_none()
                && (class, subclass) == (0x0c, 0x03)
                && self.programming_interface(address) == 0x30
            {
                xhci = Some(address);
            }
        }
        info!("pci: {found} functions found");
        xhci
    }
}

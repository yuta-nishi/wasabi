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

    /// Vendor and device ID, or `None` when no function answers at `address`.
    pub fn vendor_and_device(&self, address: Address) -> Option<(u16, u16)> {
        let vendor = self.read_u16(address, 0x00);
        if vendor == 0xffff {
            return None;
        }
        Some((vendor, self.read_u16(address, 0x02)))
    }

    /// The class code and its subclass (header offset `0x0b` and `0x0a`).
    pub fn class(&self, address: Address) -> (u8, u8) {
        let register = self.read_u32(address, 0x08);
        (
            ((register >> 24) & 0xff) as u8,
            ((register >> 16) & 0xff) as u8,
        )
    }

    /// Logs every function that answers in this window.
    pub fn probe(&self) {
        info!(
            "pci: ECAM at {:#018x} covering buses {:#04x}..={:#04x}",
            self.base, self.start_bus, self.end_bus
        );
        let mut found = 0;
        for address in self.addresses() {
            let Some((vendor, device)) = self.vendor_and_device(address) else {
                continue;
            };
            let (class, subclass) = self.class(address);
            info!(
                "pci: {address} {vendor:04x}:{device:04x} class {class:02x}.{subclass:02x}"
            );
            found += 1;
        }
        info!("pci: {found} functions found");
    }
}

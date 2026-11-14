//! ACPI: the tables the firmware describes through the RSDP.
//!
//! The EFI system table publishes the RSDP, the RSDP points at the XSDT, and the
//! XSDT lists every other table by physical address. Only the HPET description
//! is needed so far.

use crate::result::Result;
use core::mem::size_of;

/// The RSDP signature, `RSD PTR `.
const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |sum, byte| sum.wrapping_add(*byte))
}

/// `RSDP` (ACPI 6.5, 5.2.5.3): the root pointer the firmware publishes.
#[repr(C, packed)]
pub struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    _oem_id: [u8; 6],
    _revision: u8,
    _rsdt_address: u32,
    _length: u32,
    xsdt_address: u64,
    _extended_checksum: u8,
    _reserved: [u8; 3],
}
const _: () = assert!(size_of::<Rsdp>() == 36);

impl Rsdp {
    /// The first `length` bytes of this structure.
    ///
    /// # Safety
    ///
    /// `length` must not exceed the bytes the firmware actually published.
    unsafe fn bytes(&self, length: usize) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const Self as *const u8, length) }
    }

    /// Whether this is an ACPI 2.0-or-later RSDP: the signature plus the
    /// extended checksum that covers all 36 bytes. ACPI 1.0 has no XSDT and is
    /// not supported.
    pub fn is_valid(&self) -> bool {
        // SAFETY: ACPI 2.0 and later always publish the full 36 bytes.
        self.signature == *RSDP_SIGNATURE && checksum(unsafe { self.bytes(36) }) == 0
    }

    /// The XSDT the RSDP points at. The checksum already covered the address,
    /// so it is trusted here.
    pub fn xsdt(&self) -> &'static Xsdt {
        // SAFETY: an RSDP that passed `is_valid` describes this XSDT.
        unsafe { &*(self.xsdt_address as *const Xsdt) }
    }
}

/// The header every ACPI table starts with (ACPI 6.5, 5.2.6).
#[repr(C, packed)]
pub struct TableHeader {
    pub signature: [u8; 4],
    length: u32,
    _revision: u8,
    _checksum: u8,
    _oem_id: [u8; 6],
    _oem_table_id: [u8; 8],
    _oem_revision: u32,
    _creator_id: u32,
    _creator_revision: u32,
}
const _: () = assert!(size_of::<TableHeader>() == 36);

/// The extended system description table: a header and 64-bit addresses.
#[repr(C, packed)]
pub struct Xsdt {
    header: TableHeader,
}
const _: () = assert!(size_of::<Xsdt>() == 36);

impl Xsdt {
    fn length(&self) -> usize {
        self.header.length as usize
    }

    fn entry_count(&self) -> usize {
        (self.length() - size_of::<Xsdt>()) / size_of::<u64>()
    }

    fn entry(&self, index: usize) -> &'static TableHeader {
        // SAFETY: the entries follow the header and are as many as `length`
        // says; the caller keeps `index` below `entry_count`.
        unsafe {
            let base = self as *const Self as *const u8;
            let entries = base.add(size_of::<Xsdt>()) as *const u64;
            let address = entries.add(index).read_unaligned();
            &*(address as *const TableHeader)
        }
    }

    /// Looks a table up by its 4-byte signature.
    pub fn find(&self, signature: &[u8; 4]) -> Option<&'static TableHeader> {
        (0..self.entry_count())
            .map(|index| self.entry(index))
            .find(|header| &header.signature == signature)
    }
}

/// A memory address as ACPI describes it (ACPI 6.5, 5.2.3.2).
#[repr(C, packed)]
struct GenericAddress {
    address_space_id: u8,
    _bit_width: u8,
    _bit_offset: u8,
    _access_size: u8,
    address: u64,
}
const _: () = assert!(size_of::<GenericAddress>() == 12);

/// `HPET` (ACPI 6.5, 5.2.25.15): where the timer registers live.
#[repr(C, packed)]
struct HpetDescription {
    header: TableHeader,
    _event_timer_block_id: u32,
    base_address: GenericAddress,
    _hpet_number: u8,
    _minimum_tick: u16,
    _page_protection: u8,
}
const _: () = assert!(size_of::<HpetDescription>() == 56);

/// The memory-mapped base address of the HPET registers.
pub fn hpet_base(rsdp: &Rsdp) -> Result<u64> {
    let header = rsdp
        .xsdt()
        .find(b"HPET")
        .ok_or("no HPET table in the XSDT")?;
    // SAFETY: `find` only returns tables whose signature is `HPET`.
    let table = unsafe { &*(header as *const TableHeader as *const HpetDescription) };
    let address = table.base_address.address_space_id;
    if address != 0 {
        return Err("the HPET registers are not in memory space");
    }
    Ok(table.base_address.address)
}

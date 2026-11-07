//! Reading the page tables the firmware left behind.
//!
//! x86_64 walks four levels of 512-entry tables — PML4, PDPT, PD and PT — and
//! each entry points at the next level until the last one names a 4 KiB frame.
//! A PDPT entry (1 GiB) or a PD entry (2 MiB) can also end the walk early as a
//! huge page.

use crate::info;
use crate::x86::read_cr3;
use core::fmt;

const ENTRIES: usize = 512;
/// Bits 12..51 of an entry hold the physical address of the next level.
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[derive(Clone, Copy)]
#[repr(transparent)]
struct Entry(u64);

impl Entry {
    fn present(self) -> bool {
        self.0 & 1 != 0
    }
    fn huge(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    fn address(self) -> u64 {
        self.0 & ADDRESS_MASK
    }
}

/// Prints the flag bits of an entry in words, e.g. `present, writable`.
struct Flags(u64);

impl fmt::Display for Flags {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let names = [
            (1, "present"),
            (2, "writable"),
            (4, "user"),
            (1 << 7, "huge"),
            (1 << 63, "no-execute"),
        ];
        let mut first = true;
        for (bit, name) in names {
            if self.0 & bit != 0 {
                if !first {
                    f.write_str(", ")?;
                }
                first = false;
                f.write_str(name)?;
            }
        }
        Ok(())
    }
}

/// The virtual range one entry covers: `index` selects a slot at `shift`.
fn range(index: usize, shift: u32) -> (u64, u64) {
    let start = (index as u64) << shift;
    (start, start + (1 << shift))
}

/// The same range for one entry of the walk through `address`: the entry covers
/// 2^`shift` bytes, so the start is `address` with its low bits cleared.
fn range_of(address: u64, shift: u32) -> (u64, u64) {
    let start = address & !((1 << shift) - 1);
    (start, start + (1 << shift))
}

fn root() -> &'static [Entry] {
    let address = (read_cr3() & ADDRESS_MASK) as *const Entry;
    // SAFETY: CR3 points at the PML4 the CPU is currently using.
    unsafe { core::slice::from_raw_parts(address, ENTRIES) }
}

fn next(entry: Entry) -> &'static [Entry] {
    let address = entry.address() as *const Entry;
    // SAFETY: a present entry without PS points at the next level.
    unsafe { core::slice::from_raw_parts(address, ENTRIES) }
}

/// Prints the present entries of the root (PML4) table.
pub fn dump_root() {
    info!(
        "paging: cr3 = {:#018x} (the PML4 the firmware set up)",
        read_cr3() & ADDRESS_MASK
    );
    for (index, entry) in root().iter().enumerate() {
        if entry.present() {
            let (start, end) = range(index, 39);
            info!(
                "  pml4[{index:03}] {start:#018x}..{end:#018x} -> next table {:#x}, flags={}",
                entry.address(),
                Flags(entry.0)
            );
        }
    }
}

/// Prints the chain of entries that maps `address`, one line per level, and
/// the physical address it ends up at.
pub fn dump_translation(address: u64) {
    info!("paging: {address:#018x}");
    let mut table = root();
    let mut shift = 39;
    for name in ["pml4", "pdpt", "pd", "pt"] {
        let index = ((address >> shift) & 0x1ff) as usize;
        let entry = table[index];
        let (start, end) = range_of(address, shift);
        if !entry.present() {
            info!("  {name}[{index:03}] {start:#018x}..{end:#018x} -> not present");
            return;
        }
        if entry.huge() {
            let offset = address & ((1 << shift) - 1);
            info!(
                "  {name}[{index:03}] {start:#018x}..{end:#018x} -> huge page, flags={}",
                Flags(entry.0)
            );
            info!("  -> physical {:#018x}", entry.address() + offset);
            return;
        }
        if name == "pt" {
            let offset = address & 0xfff;
            info!(
                "  {name}[{index:03}] {start:#018x}..{end:#018x} -> 4 KiB page, flags={}",
                Flags(entry.0)
            );
            info!("  -> physical {:#018x}", entry.address() + offset);
            return;
        }
        info!(
            "  {name}[{index:03}] {start:#018x}..{end:#018x} -> next table {:#x}, flags={}",
            entry.address(),
            Flags(entry.0)
        );
        table = next(entry);
        shift -= 9;
    }
}

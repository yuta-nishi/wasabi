//! Walking the page tables.
//!
//! x86_64 uses four levels of 512-entry tables — PML4, PDPT, PD and PT — and
//! each entry points at the next level until the last one names a 4 KiB frame.
//! A PDPT entry (1 GiB) or a PD entry (2 MiB) can also end the walk early as a
//! huge page.
//!
//! `translate_in` only reads memory and reports what it found, so the walk can
//! be tested with hand-made tables; `dump_root` and `dump_translation` print the
//! results for the tables `CR3` currently points at.

use crate::info;
use crate::x86::read_cr3;
use alloc::vec::Vec;
use core::fmt;

/// Bits 12..51 of an entry hold the physical address of the next level.
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const ENTRIES: usize = 512;

/// One entry of any level, as it appears in memory.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Entry(u64);

impl Entry {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
    pub const fn raw(self) -> u64 {
        self.0
    }
    pub const fn present(self) -> bool {
        self.0 & 1 != 0
    }
    pub const fn huge(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    /// The physical address of the next table, or of the frame this entry maps.
    pub const fn address(self) -> u64 {
        self.0 & ADDRESS_MASK
    }
}

/// The four levels of the tree, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Pml4,
    Pdpt,
    Pd,
    Pt,
}

impl Level {
    const ALL: [Level; 4] = [Level::Pml4, Level::Pdpt, Level::Pd, Level::Pt];

    pub const fn name(self) -> &'static str {
        match self {
            Level::Pml4 => "pml4",
            Level::Pdpt => "pdpt",
            Level::Pd => "pd",
            Level::Pt => "pt",
        }
    }

    /// How many low bits of the address this level's index selects (12..39).
    pub const fn shift(self) -> u32 {
        match self {
            Level::Pml4 => 39,
            Level::Pdpt => 30,
            Level::Pd => 21,
            Level::Pt => 12,
        }
    }
}

/// One entry of a walk: which slot was read and what it covers.
#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub level: Level,
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub entry: Entry,
}

/// Where an address ends up.
#[derive(Debug)]
pub enum Translation {
    /// An entry on the way was not present: the address is not mapped.
    NotMapped(Vec<Step>),
    /// The walk hit a huge page (1 GiB at the PDPT, 2 MiB at the PD).
    Huge(Vec<Step>, u64),
    /// The walk ended at a 4 KiB frame.
    Page(Vec<Step>, u64),
}

/// Walks `address` through `root` (the PML4) and reports what it found.
pub fn translate_in(root: &[Entry], address: u64) -> Translation {
    let mut steps = Vec::new();
    let mut table: &[Entry] = root;
    for level in Level::ALL {
        let shift = level.shift();
        let index = ((address >> shift) & 0x1ff) as usize;
        let entry = table[index];
        let start = address & !((1 << shift) - 1);
        steps.push(Step {
            level,
            index,
            start,
            end: start + (1 << shift),
            entry,
        });
        if !entry.present() {
            return Translation::NotMapped(steps);
        }
        if entry.huge() {
            let physical = entry.address() + (address & ((1 << shift) - 1));
            return Translation::Huge(steps, physical);
        }
        if level == Level::Pt {
            let physical = entry.address() + (address & 0xfff);
            return Translation::Page(steps, physical);
        }
        table = next(entry);
    }
    unreachable!("the walk always ends at the PT level");
}

/// Walks `address` through the tables `CR3` currently points at.
pub fn translate(address: u64) -> Translation {
    translate_in(root(), address)
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

/// The flag bits of an entry in words, e.g. `present, writable`.
struct Flags(Entry);

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
            if self.0.raw() & bit != 0 {
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

fn print_step(step: &Step) {
    let (level, index, start, end) = (step.level.name(), step.index, step.start, step.end);
    if !step.entry.present() {
        info!("  {level}[{index:03}] {start:#018x}..{end:#018x} -> not present");
    } else if step.entry.huge() {
        info!(
            "  {level}[{index:03}] {start:#018x}..{end:#018x} -> huge page ({})",
            Flags(step.entry)
        );
    } else {
        info!(
            "  {level}[{index:03}] {start:#018x}..{end:#018x} -> next table {:#x} ({})",
            step.entry.address(),
            Flags(step.entry)
        );
    }
}

/// Prints the present entries of the root (PML4) table.
pub fn dump_root() {
    info!(
        "paging: cr3 = {:#018x} (the PML4 the firmware set up)",
        read_cr3() & ADDRESS_MASK
    );
    let shift = Level::Pml4.shift();
    for (index, entry) in root().iter().enumerate() {
        if entry.present() {
            let start = (index as u64) << shift;
            print_step(&Step {
                level: Level::Pml4,
                index,
                start,
                end: start + (1 << shift),
                entry: *entry,
            });
        }
    }
}

/// Prints the walk for `address` and where it ends up.
pub fn dump_translation(address: u64) {
    info!("paging: {address:#018x}");
    let (steps, physical) = match translate(address) {
        Translation::NotMapped(steps) => {
            for step in &steps {
                print_step(step);
            }
            info!("  -> not mapped");
            return;
        }
        Translation::Huge(steps, physical) => (steps, physical),
        Translation::Page(steps, physical) => (steps, physical),
    };
    for step in &steps {
        print_step(step);
    }
    info!("  -> physical {physical:#018x}");
}

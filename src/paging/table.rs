use crate::bitfield;
use crate::x86::read_cr3;
use crate::x86::write_cr3;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::asm;
use core::pin::Pin;

bitfield! {
    /// A page table entry (Intel SDM 4.5).
    EntryFlags: u64 {
        present: 0, 0 -> set_present,
        writable: 1, 1 -> set_writable,
        no_cache: 4, 4 -> set_no_cache,
        huge: 7, 7 -> set_huge,
        address: 51, 12 -> set_address,
    }
}

impl EntryFlags {
    /// A present, writable mapping of `physical`.
    fn mapping(physical: u64) -> Self {
        let mut flags = Self::new(0);
        flags.set_present(1);
        flags.set_writable(1);
        flags.set_address(physical >> 12);
        flags
    }

    /// The same mapping, with reads and writes going to the device.
    fn uncached(physical: u64) -> Self {
        let mut flags = Self::mapping(physical);
        flags.set_no_cache(1);
        flags
    }

    /// The physical address this entry maps.
    pub(super) const fn physical(self) -> u64 {
        self.address() << 12
    }
}

pub(super) const ENTRIES: usize = 512;

/// 4 KiB (`1 << 12`): the unit a page table maps, and therefore the size of one
/// table.
pub const PAGE_SIZE: u64 = 0x1000;
/// The address bits below a page boundary.
const PAGE_OFFSET_MASK: u64 = PAGE_SIZE - 1;
/// A 2 MiB page: one PD entry that maps a whole frame instead of pointing at a
/// table of 4 KiB pages.
pub const HUGE_PAGE_SIZE: u64 = 0x20_0000;

/// One page of entries. It must be page aligned: its address is stored in the
/// high bits of the entry that points at it, so a misaligned table would put
/// stray bits into the flag fields.
#[repr(align(4096))]
pub struct Table([Entry; ENTRIES]);

impl Table {
    /// A table with every entry not present.
    pub fn zeroed() -> Box<Table> {
        Box::new(Table([Entry::from_raw(0); ENTRIES]))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.0
    }

    pub fn entries_mut(&mut self) -> &mut [Entry] {
        &mut self.0
    }
}

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
        EntryFlags::new(self.0).present() != 0
    }
    pub const fn huge(self) -> bool {
        EntryFlags::new(self.0).huge() != 0
    }
    /// The physical address of the next table, or of the frame this entry maps.
    pub const fn address(self) -> u64 {
        EntryFlags::new(self.0).physical()
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
        // Bit 7 is PS for a PDPT or PD entry, but PAT for a PT entry.
        if entry.huge() && level != Level::Pt {
            let physical = entry.address() + (address & ((1 << shift) - 1));
            return Translation::Huge(steps, physical);
        }
        if level == Level::Pt {
            let physical = entry.address() + (address & PAGE_OFFSET_MASK);
            return Translation::Page(steps, physical);
        }
        table = next_table(entry);
    }
    unreachable!("the walk always ends at the PT level");
}

/// Drops any cached translation for `virtual_address`.
///
/// Needed when an entry that was already present changes: the old translation,
/// with its old attributes, would otherwise keep being used.
pub fn invalidate_page(virtual_address: u64) {
    // SAFETY: `invlpg` affects only the translation of that one address.
    unsafe {
        asm!("invlpg [{}]", in(reg) virtual_address, options(nostack, preserves_flags))
    }
}

/// Walks `address` through the tables `CR3` currently points at.
pub fn translate(address: u64) -> Translation {
    translate_in(root(), address)
}

/// A page table tree that this program built.
///
/// The lower tables are leaked on purpose: once the CPU can reach a table it
/// must stay where it is, and the tree is never torn down. The PML4 is pinned
/// for the same reason, because its address ends up in `CR3`.
pub struct PageTables {
    pml4: Pin<Box<Table>>,
}

impl Default for PageTables {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTables {
    /// Creates an empty tree with every entry not present.
    pub fn new() -> Self {
        Self {
            pml4: Box::pin(*Table::zeroed()),
        }
    }

    /// Maps `0..end` at the same physical addresses, using 2 MiB pages for the
    /// aligned bulk and 4 KiB pages for the tail.
    ///
    /// One 4 KiB page per entry would need about a million entries and 8 MiB of
    /// page tables for 4 GiB; 2 MiB pages need 512 entries per GiB and three
    /// tables.
    pub fn identity_map(end: u64) -> Self {
        let mut tables = Self::new();
        let mut address = 0;
        while address + HUGE_PAGE_SIZE <= end {
            tables.map_huge_page(address, address);
            address += HUGE_PAGE_SIZE;
        }
        while address < end {
            tables.map_page(address, address);
            address += PAGE_SIZE;
        }
        tables
    }

    /// Maps 2 MiB at `virtual_address`, which must be aligned to 2 MiB.
    pub fn map_huge_page(&mut self, virtual_address: u64, physical_address: u64) {
        assert_eq!(
            virtual_address % HUGE_PAGE_SIZE,
            0,
            "virtual address alignment"
        );
        assert_eq!(
            physical_address % HUGE_PAGE_SIZE,
            0,
            "physical address alignment"
        );
        let (table, index) = self
            .page_directory_and_index(virtual_address, true)
            .expect("PD allocated");
        // SAFETY: `table` points at the PD `pd_and_index` just returned.
        unsafe {
            (*table).entries_mut()[index] = Entry::from_raw({
                let mut flags = EntryFlags::mapping(physical_address);
                flags.set_huge(1);
                flags.raw()
            })
        };
    }

    /// Maps one page, allocating the tables on the way down.
    pub fn map_page(&mut self, virtual_address: u64, physical_address: u64) {
        self.map_page_with(virtual_address, EntryFlags::mapping(physical_address));
    }

    /// Maps `length` bytes from `virtual_address` uncached, whether or not they
    /// were mapped before, and drops the translations that changed.
    pub fn map_range_uncached(&mut self, virtual_address: u64, length: u64) {
        // SAFETY: the pages are mapped below, before anything reads them.
        for page in 0..length.div_ceil(PAGE_SIZE) {
            let address = virtual_address + page * PAGE_SIZE;
            self.map_page_uncached(address, address);
            invalidate_page(address);
        }
    }

    /// Maps one page of device memory.
    ///
    /// The cache-disable bit matters here: a cached read of a device register
    /// returns the old value instead of asking the device again.
    pub fn map_page_uncached(&mut self, virtual_address: u64, physical_address: u64) {
        self.map_page_with(virtual_address, EntryFlags::uncached(physical_address));
    }

    /// # Invalidation
    ///
    /// Replacing an entry that was already present does not invalidate anything:
    /// the CPU may keep translating through the old attributes until the entry
    /// is flushed. Newly mapped pages need no flush, because a translation the
    /// CPU never made is not cached.
    fn map_page_with(&mut self, virtual_address: u64, flags: EntryFlags) {
        assert_eq!(virtual_address % PAGE_SIZE, 0, "virtual address alignment");

        let (table, index) = self
            .page_table_and_index(virtual_address, true)
            .expect("walking with allocation always reaches a PT");
        // SAFETY: `table` points at the PT `pt_and_index` just returned.
        unsafe { (*table).entries_mut()[index] = Entry::from_raw(flags.raw()) };
    }

    /// Clears the mapping for one page, so touching it faults.
    pub fn unmap_page(&mut self, virtual_address: u64) {
        assert_eq!(virtual_address % PAGE_SIZE, 0, "virtual address alignment");

        let Some((table, index)) = self.page_table_and_index(virtual_address, false)
        else {
            return;
        };
        // SAFETY: `table` points at the PT that holds this page.
        unsafe { (*table).entries_mut()[index] = Entry::from_raw(0) };
    }

    /// Walks to the PD that holds `address`, for a 2 MiB page.
    fn page_directory_and_index(
        &mut self,
        address: u64,
        allocate: bool,
    ) -> Option<(*mut Table, usize)> {
        let mut table: *mut Table = &mut *self.pml4;
        for level in [Level::Pml4, Level::Pdpt] {
            let index = ((address >> level.shift()) & 0x1ff) as usize;
            // SAFETY: `table` points at a table owned by this address space.
            let entry = unsafe { &mut (*table).entries_mut()[index] };
            if !entry.present() {
                if !allocate {
                    return None;
                }
                let next = Box::into_raw(Table::zeroed());
                *entry = Entry::from_raw(EntryFlags::mapping(next as u64).raw());
            }
            table = entry.address() as *mut Table;
        }
        let index = ((address >> Level::Pd.shift()) & 0x1ff) as usize;
        Some((table, index))
    }

    /// Replaces a 2 MiB page with a table of 4 KiB pages covering the same
    /// range, so one page inside it can be changed.
    ///
    /// Every translation in the 2 MiB range changes, so all of them are dropped.
    fn split_huge_page(&mut self, address: u64) -> bool {
        let Some((table, index)) = self.page_directory_and_index(address, false) else {
            return false;
        };
        // SAFETY: `table` points at the PD that holds this entry.
        let entry = unsafe { &mut (*table).entries_mut()[index] };
        if !entry.present() || !entry.huge() {
            return false;
        }
        // Attributes stay; the PS bit moves from the PD entry to a table pointer,
        // and in a 4 KiB entry bit 7 means PAT, so it must not survive there.
        let original = EntryFlags::new(entry.raw());
        let base = entry.address();
        let mut pages = Table::zeroed();
        for (index, page) in pages.entries_mut().iter_mut().enumerate() {
            let mut flags = original;
            flags.set_huge(0);
            flags.set_address((base + (index as u64) * PAGE_SIZE) >> 12);
            *page = Entry::from_raw(flags.raw());
        }
        let pages = Box::into_raw(pages);
        let mut table_flags = original;
        table_flags.set_huge(0);
        table_flags.set_address(pages as u64 >> 12);
        *entry = Entry::from_raw(table_flags.raw());
        let start = address & !(HUGE_PAGE_SIZE - 1);
        for page in 0..HUGE_PAGE_SIZE / PAGE_SIZE {
            invalidate_page(start + page * PAGE_SIZE);
        }
        true
    }

    /// Walks down to the PT that holds `address`, allocating missing levels
    /// when `allocate` is set, and returns that table with the entry index.
    fn page_table_and_index(
        &mut self,
        address: u64,
        allocate: bool,
    ) -> Option<(*mut Table, usize)> {
        let mut table: *mut Table = &mut *self.pml4;
        for level in [Level::Pml4, Level::Pdpt, Level::Pd] {
            let index = ((address >> level.shift()) & 0x1ff) as usize;
            // SAFETY: `table` points at a table owned by this address space.
            let entry = unsafe { &mut (*table).entries_mut()[index] };
            if !entry.present() {
                if !allocate {
                    return None;
                }
                let next = Box::into_raw(Table::zeroed());
                *entry = Entry::from_raw(EntryFlags::mapping(next as u64).raw());
            } else if level == Level::Pd && entry.huge() {
                self.split_huge_page(address);
            }
            table = entry.address() as *mut Table;
        }
        let index = ((address >> Level::Pt.shift()) & 0x1ff) as usize;
        Some((table, index))
    }

    /// The root of the tree, for `translate_in`.
    pub fn root(&self) -> &[Entry] {
        self.pml4.entries()
    }

    /// Makes this tree the active one by loading its PML4 into `CR3`.
    pub fn switch(&self) {
        // The tree lives for the rest of the run and is never moved.
        write_cr3(self.root().as_ptr() as u64);
    }
}

pub(super) fn root() -> &'static [Entry] {
    let address = EntryFlags::new(read_cr3()).physical() as *const Entry;
    // SAFETY: CR3 points at the PML4 the CPU is currently using.
    unsafe { core::slice::from_raw_parts(address, ENTRIES) }
}

fn next_table(entry: Entry) -> &'static [Entry] {
    let address = entry.address() as *const Entry;
    // SAFETY: a present entry without PS points at the next level.
    unsafe { core::slice::from_raw_parts(address, ENTRIES) }
}

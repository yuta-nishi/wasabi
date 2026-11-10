extern crate alloc;

use wasabi::paging::Entry;
use wasabi::paging::PageTables;
use wasabi::paging::Table;
use wasabi::paging::Translation;
use wasabi::paging::translate_in;

/// The entry another table would use to point at this one.
fn pointer(table: &Table) -> Entry {
    Entry::from_raw(table.entries().as_ptr() as u64 | 1 | 2)
}

pub fn four_level_walk_finds_a_4k_frame() {
    let mut pt = Table::zeroed();
    pt.entries_mut()[0] = Entry::from_raw(0x5000 | 1 | 2 | 4);
    let mut pd = Table::zeroed();
    pd.entries_mut()[0] = pointer(&pt);
    let mut pdpt = Table::zeroed();
    pdpt.entries_mut()[0] = pointer(&pd);
    let mut pml4 = Table::zeroed();
    pml4.entries_mut()[0] = pointer(&pdpt);

    match translate_in(pml4.entries(), 0x234) {
        Translation::Page(steps, physical) => {
            assert_eq!(steps.len(), 4);
            assert_eq!(physical, 0x5234);
        }
        other => panic!("expected a 4 KiB page, got {other:?}"),
    }
}

pub fn huge_page_ends_the_walk_early() {
    let mut pd = Table::zeroed();
    pd.entries_mut()[0] = Entry::from_raw(0x20_0000 | 1 | 2 | (1 << 7));
    let mut pdpt = Table::zeroed();
    pdpt.entries_mut()[0] = pointer(&pd);
    let mut pml4 = Table::zeroed();
    pml4.entries_mut()[0] = pointer(&pdpt);

    match translate_in(pml4.entries(), 0x1234) {
        Translation::Huge(steps, physical) => {
            assert_eq!(steps.len(), 3);
            assert_eq!(physical, 0x20_1234);
        }
        other => panic!("expected a huge page, got {other:?}"),
    }
}

pub fn missing_entry_reports_not_mapped() {
    let pml4 = Table::zeroed();

    match translate_in(pml4.entries(), 0x1234) {
        Translation::NotMapped(steps) => assert_eq!(steps.len(), 1),
        other => panic!("expected an unmapped address, got {other:?}"),
    }
}

pub fn new_tables_map_requested_pages() {
    let mut tables = PageTables::new();
    tables.map_page(0, 0);
    tables.map_page(0x3000, 0x9000);

    match translate_in(tables.root(), 0) {
        Translation::Page(_, physical) => assert_eq!(physical, 0),
        other => panic!("page 0: {other:?}"),
    }
    match translate_in(tables.root(), 0x3000) {
        Translation::Page(_, physical) => assert_eq!(physical, 0x9000),
        other => panic!("page 0x3000: {other:?}"),
    }
}

pub fn unmapping_page_zero_leaves_it_not_mapped() {
    let mut tables = PageTables::new();
    tables.map_page(0, 0);
    tables.map_page(0x1000, 0x1000);
    tables.unmap_page(0);

    match translate_in(tables.root(), 0) {
        Translation::NotMapped(_) => {}
        other => panic!("page 0: {other:?}"),
    }
    match translate_in(tables.root(), 0x1000) {
        Translation::Page(_, physical) => assert_eq!(physical, 0x1000),
        other => panic!("page 0x1000: {other:?}"),
    }
}

extern crate alloc;

use alloc::boxed::Box;
use wasabi::paging::Entry;
use wasabi::paging::Translation;
use wasabi::paging::translate_in;

/// A page table has to be page aligned, because its address is stored in the
/// high bits of the entry that points at it.
#[repr(align(4096))]
struct Table([Entry; 512]);

impl Table {
    fn empty() -> Box<Table> {
        Box::new(Table([Entry::from_raw(0); 512]))
    }

    /// The entry another table would use to point at this one.
    fn pointer(&self) -> Entry {
        Entry::from_raw(self.0.as_ptr() as u64 | 1 | 2)
    }
}

pub fn four_level_walk_finds_a_4k_frame() {
    let mut pt = Table::empty();
    pt.0[0] = Entry::from_raw(0x5000 | 1 | 2 | 4);
    let mut pd = Table::empty();
    pd.0[0] = pt.pointer();
    let mut pdpt = Table::empty();
    pdpt.0[0] = pd.pointer();
    let mut pml4 = Table::empty();
    pml4.0[0] = pdpt.pointer();

    match translate_in(&pml4.0, 0x234) {
        Translation::Page(steps, physical) => {
            assert_eq!(steps.len(), 4);
            assert_eq!(physical, 0x5234);
        }
        other => panic!("expected a 4 KiB page, got {other:?}"),
    }
}

pub fn huge_page_ends_the_walk_early() {
    let mut pd = Table::empty();
    pd.0[0] = Entry::from_raw(0x20_0000 | 1 | 2 | (1 << 7));
    let mut pdpt = Table::empty();
    pdpt.0[0] = pd.pointer();
    let mut pml4 = Table::empty();
    pml4.0[0] = pdpt.pointer();

    match translate_in(&pml4.0, 0x1234) {
        Translation::Huge(steps, physical) => {
            assert_eq!(steps.len(), 3);
            assert_eq!(physical, 0x20_1234);
        }
        other => panic!("expected a huge page, got {other:?}"),
    }
}

pub fn missing_entry_reports_not_mapped() {
    let pml4 = Table::empty();

    match translate_in(&pml4.0, 0x1234) {
        Translation::NotMapped(steps) => assert_eq!(steps.len(), 1),
        other => panic!("expected an unmapped address, got {other:?}"),
    }
}

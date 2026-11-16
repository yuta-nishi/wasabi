//! Page tables: walking the ones `CR3` points at, and building our own.
//!
//! x86_64 uses four levels of 512-entry tables — PML4, PDPT, PD and PT — and
//! each entry points at the next level until the last one names a 4 KiB frame.
//! A PDPT entry (1 GiB) or a PD entry (2 MiB) can also end the walk early as a
//! huge page.

mod dump;
mod table;

pub use dump::dump_root;
pub use dump::dump_translation;
pub use table::Entry;
pub use table::Level;
pub use table::PAGE_SIZE;
pub use table::PageTables;
pub use table::Step;
pub use table::Table;
pub use table::Translation;
pub use table::translate;
pub use table::translate_in;

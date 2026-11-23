use super::table::Entry;
use super::table::EntryFlags;
use super::table::Level;
use super::table::Step;
use super::table::Translation;
use super::table::root;
use super::table::translate;
use crate::info;
use crate::x86::read_cr3;
use core::fmt;

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
    let (level, index, start, end) =
        (step.level.name(), step.index, step.start, step.end);
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
        "paging: cr3 = {:#018x} (the active PML4)",
        EntryFlags::new(read_cr3()).physical()
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

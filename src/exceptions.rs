//! Installs the exception machinery: GDT/TSS first, then the IDT.

use crate::gdt::Gdt;
use crate::gdt::KERNEL_CODE_SELECTOR;
use crate::idt::Idt;
use crate::info;

/// Owns the tables the CPU points at. Dropping it would leave the CPU with
/// dangling descriptors, so callers keep it alive for the rest of the run.
pub struct Exceptions {
    _gdt: Gdt,
    _idt: Idt,
}

pub fn init() -> Exceptions {
    let gdt = Gdt::new();
    gdt.load();
    let idt = Idt::new(KERNEL_CODE_SELECTOR);
    idt.load();
    info!("exceptions installed");
    Exceptions { _gdt: gdt, _idt: idt }
}

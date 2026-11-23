use crate::gdt::Gdt;
use crate::gdt::KERNEL_CODE_SELECTOR;
use crate::idt::Idt;
use crate::info;
use alloc::boxed::Box;

/// Installs the tables the CPU points at.
///
/// They are leaked: the CPU keeps their addresses for the rest of the run, so
/// dropping them would leave it with dangling descriptors.
pub fn init() {
    let gdt = Box::leak(Box::new(Gdt::new()));
    gdt.load();
    let idt = Box::leak(Box::new(Idt::new(KERNEL_CODE_SELECTOR)));
    idt.load();
    info!("exceptions installed");
}

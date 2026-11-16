use core::arch::asm;

pub fn hlt() {
    unsafe { asm!("hlt") }
}

/// Lets the CPU take interrupts again. Until this runs, `hlt` never returns.
pub fn enable_interrupts() {
    unsafe { asm!("sti") }
}

pub fn write_io_port_u8(port: u16, data: u8) {
    unsafe {
        asm!("out dx, al",
            in("al") data,
            in("dx") port)
    }
}

pub fn read_io_port_u8(port: u16) -> u8 {
    let data: u8;
    unsafe {
        asm!("in al, dx",
            out("al") data,
            in("dx") port)
    }
    data
}

pub fn read_cr2() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr2", out(reg) value) }
    value
}

pub fn write_cr3(value: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) value) }
}

pub fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value) }
    value
}

/// Operand of `lgdt` / `lidt`: a 16-bit limit and a 64-bit base address.
#[derive(Clone, Copy)]
#[repr(C, packed)]
pub struct DescriptorTablePointer {
    pub limit: u16,
    pub base: u64,
}
const _: () = assert!(core::mem::size_of::<DescriptorTablePointer>() == 10);

/// Disables interrupts and returns the previous flag word, for
/// [`restore_interrupts`].
pub fn disable_interrupts() -> u64 {
    let flags: u64;
    unsafe { asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem)) }
    flags
}

/// Restores the flag word saved by [`disable_interrupts`].
///
/// # Safety
///
/// `flags` must come from [`disable_interrupts`] on the same stack.
pub unsafe fn restore_interrupts(flags: u64) {
    unsafe { asm!("push {}", "popfq", in(reg) flags, options(nomem)) }
}

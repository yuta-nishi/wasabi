use core::arch::asm;

pub fn hlt() {
    unsafe { asm!("hlt") }
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

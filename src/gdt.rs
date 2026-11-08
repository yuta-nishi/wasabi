//! Global Descriptor Table and Task State Segment.
//!
//! Long mode barely uses segmentation, but the CPU still needs a GDT to hold the
//! TSS descriptor and to give the kernel a valid code/data segment. Everything
//! here is referenced by address, so the owners are returned instead of being
//! dropped.

use alloc::boxed::Box;
use crate::x86::DescriptorTablePointer;
use core::arch::asm;
use core::mem::size_of;
use core::pin::Pin;

/// Selector of the kernel code segment (GDT entry 1).
pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
/// Selector of the kernel data segment (GDT entry 2).
const KERNEL_DATA_SELECTOR: u16 = 0x10;
/// Selector of the TSS (GDT entries 3 and 4: a 16-byte system descriptor).
const TSS_SELECTOR: u16 = 0x18;

/// Present, DPL 0, 64-bit code segment, 4 GiB limit.
const KERNEL_CODE: u64 = 0x00af_9a00_0000_ffff;
/// Present, DPL 0, data segment.
const KERNEL_DATA: u64 = 0x00cf_9200_0000_ffff;

/// Stack the CPU switches to for exceptions (and later for interrupts).
const INTERRUPT_STACK_SIZE: usize = 16 * 1024;

/// The 104-byte x86_64 TSS; only `rsp0` and IST1 are used so far.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    _reserved0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}
const _: () = assert!(size_of::<Tss>() == 104);

/// The GDT as it is laid out in memory.
#[repr(C)]
struct GdtBacking {
    null: u64,
    kernel_code: u64,
    kernel_data: u64,
    tss_low: u64,
    tss_high: u64,
}
const _: () = assert!(size_of::<GdtBacking>() == 5 * 8);

/// Low half of a 64-bit TSS descriptor: present, type 0x9 (available TSS).
const fn tss_low(base: u64, limit: u32) -> u64 {
    let limit = limit as u64;
    (limit & 0xffff)
        | ((base & 0x00ff_ffff) << 16)
        | (0b1000_1001 << 40)
        | (((limit >> 16) & 0xf) << 48)
        | (((base >> 24) & 0xff) << 56)
}

/// Owns the GDT, the TSS and the interrupt stack they point at.
pub struct Gdt {
    entries: Pin<Box<GdtBacking>>,
    _tss: Pin<Box<Tss>>,
    _interrupt_stack: Pin<Box<[u8; INTERRUPT_STACK_SIZE]>>,
}

impl Default for Gdt {
    fn default() -> Self {
        Self::new()
    }
}

impl Gdt {
    pub fn new() -> Self {
        let interrupt_stack = Box::pin([0u8; INTERRUPT_STACK_SIZE]);
        let rsp0 = interrupt_stack[..].as_ptr() as u64 + INTERRUPT_STACK_SIZE as u64;

        let tss = Box::pin(Tss {
            _reserved0: 0,
            rsp0,
            rsp1: 0,
            rsp2: 0,
            _reserved1: 0,
            ist: [rsp0, 0, 0, 0, 0, 0, 0],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: size_of::<Tss>() as u16,
        });
        let tss_base = &*tss as *const Tss as u64;

        let entries = Box::pin(GdtBacking {
            null: 0,
            kernel_code: KERNEL_CODE,
            kernel_data: KERNEL_DATA,
            tss_low: tss_low(tss_base, size_of::<Tss>() as u32 - 1),
            tss_high: tss_base >> 32,
        });

        Self {
            entries,
            _tss: tss,
            _interrupt_stack: interrupt_stack,
        }
    }

    /// Loads the table, reloads the segment registers and installs the TSS.
    pub fn load(&self) {
        let pointer = DescriptorTablePointer {
            limit: size_of::<GdtBacking>() as u16 - 1,
            base: &*self.entries as *const GdtBacking as u64,
        };

        // SAFETY: `pointer` describes the table owned by `self`, which outlives
        // every use of the loaded descriptors, and the selectors below name
        // entries that exist in it.
        unsafe {
            asm!("lgdt [{}]", in(reg) &pointer, options(nostack, preserves_flags));
            // A far return is the only way to reload CS outside of an interrupt.
            asm!(
                "push {code}",
                "lea {target}, [rip + 2f]",
                "push {target}",
                "retfq",
                "2:",
                code = in(reg) KERNEL_CODE_SELECTOR as u64,
                target = out(reg) _,
                options(preserves_flags),
            );
            asm!(
                "mov ds, {data:x}",
                "mov es, {data:x}",
                "mov ss, {data:x}",
                "mov fs, {data:x}",
                "mov gs, {data:x}",
                data = in(reg) KERNEL_DATA_SELECTOR,
                options(preserves_flags),
            );
            asm!(
                "ltr {tss:x}",
                tss = in(reg) TSS_SELECTOR,
                options(nostack, preserves_flags),
            );
        }
    }
}

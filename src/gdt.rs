use crate::bitfield;
use crate::x86::DescriptorTablePointer;
use alloc::boxed::Box;
use core::arch::asm;
use core::mem::size_of;
use core::pin::Pin;

/// Selector of the kernel code segment (GDT entry 1).
pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
/// Selector of the kernel data segment (GDT entry 2).
const KERNEL_DATA_SELECTOR: u16 = 0x10;
/// Selector of the TSS (GDT entries 3 and 4: a 16-byte system descriptor).
const TSS_SELECTOR: u16 = 0x18;

bitfield! {
    /// A code, data or system segment descriptor (Intel SDM 3.4.5).
    SegmentDescriptor: u64 {
        limit_low: 15, 0 -> set_limit_low,
        base_low: 31, 16 -> set_base_low,
        base_middle: 39, 32 -> set_base_middle,
        kind: 43, 40 -> set_kind,
        system: 44, 44 -> set_system,
        dpl: 46, 45 -> set_dpl,
        present: 47, 47 -> set_present,
        limit_high: 51, 48 -> set_limit_high,
        available: 52, 52 -> set_available,
        long: 53, 53 -> set_long,
        wide: 54, 54 -> set_wide,
        granularity: 55, 55 -> set_granularity,
        base_high: 63, 56 -> set_base_high,
    }
}

/// A segment covering all of memory, in 4 KiB units.
const LIMIT_4_GIB: u32 = 0x000f_ffff;

/// A 64-bit code segment: readable, present, ring 0.
fn kernel_code() -> SegmentDescriptor {
    let mut descriptor = SegmentDescriptor::new(0);
    descriptor.set_limit_low(LIMIT_4_GIB as u64 & 0xffff);
    descriptor.set_limit_high(u64::from(LIMIT_4_GIB >> 16) & 0xf);
    descriptor.set_kind(0b1010);
    descriptor.set_system(1);
    descriptor.set_present(1);
    descriptor.set_long(1);
    descriptor.set_granularity(1);
    descriptor
}

/// A data segment: writable, present, ring 0.
fn kernel_data() -> SegmentDescriptor {
    let mut descriptor = SegmentDescriptor::new(0);
    descriptor.set_limit_low(LIMIT_4_GIB as u64 & 0xffff);
    descriptor.set_limit_high(u64::from(LIMIT_4_GIB >> 16) & 0xf);
    descriptor.set_kind(0b0010);
    descriptor.set_system(1);
    descriptor.set_present(1);
    descriptor.set_wide(1);
    descriptor.set_granularity(1);
    descriptor
}

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

#[repr(C)]
struct GdtBacking {
    null_descriptor: u64,
    kernel_code_descriptor: u64,
    kernel_data_descriptor: u64,
    tss_descriptor_low: u64,
    tss_descriptor_high: u64,
}
const _: () = assert!(size_of::<GdtBacking>() == 5 * 8);

/// Low half of a 64-bit TSS descriptor: present, type 0x9 (available TSS).
fn tss_descriptor_low(base: u64, limit: u32) -> u64 {
    let mut descriptor = SegmentDescriptor::new(0);
    descriptor.set_limit_low(u64::from(limit) & 0xffff);
    descriptor.set_limit_high(u64::from(limit >> 16) & 0xf);
    descriptor.set_base_low(base & 0xffff);
    descriptor.set_base_middle((base >> 16) & 0xff);
    descriptor.set_kind(0b1001);
    descriptor.set_present(1);
    descriptor.set_base_high((base >> 24) & 0xff);
    descriptor.raw()
}

/// Owns the GDT, the TSS and the interrupt stack they point at.
///
/// Long mode barely uses segmentation, but the CPU still needs a GDT to hold the
/// TSS descriptor and to give the kernel a valid code segment.
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
            null_descriptor: 0,
            kernel_code_descriptor: kernel_code().raw(),
            kernel_data_descriptor: kernel_data().raw(),
            tss_descriptor_low: tss_descriptor_low(tss_base, size_of::<Tss>() as u32 - 1),
            tss_descriptor_high: tss_base >> 32,
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

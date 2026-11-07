//! Interrupt Descriptor Table and the exception entry stubs.
//!
//! Every vector gets a small assembly stub. The breakpoint exception counts
//! itself and returns to the interrupted code; anything else reports and stops
//! the run.

use crate::qemu::QemuExitCode;
use crate::qemu::exit_qemu;
use crate::serial::print_str;
use alloc::boxed::Box;
use core::arch::asm;
use core::arch::global_asm;
use core::mem::size_of;
use core::pin::Pin;
use core::sync::atomic::AtomicU64;

/// How many breakpoint exceptions have been handled so far. It only grows, so a
/// test can prove that the handler ran and that execution resumed.
pub static BREAKPOINTS_HANDLED: AtomicU64 = AtomicU64::new(0);

/// Present, DPL 0, 64-bit interrupt gate.
const INTERRUPT_GATE: u8 = 0x8e;
/// IST index (1-based) of the stack the CPU switches to for every exception.
const INTERRUPT_STACK_INDEX: u8 = 1;

const EXCEPTION_COUNT: usize = 32;

#[derive(Clone, Copy)]
#[repr(C)]
struct IdtDescriptor {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}
const _: () = assert!(size_of::<IdtDescriptor>() == 16);

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}
const _: () = assert!(size_of::<DescriptorTablePointer>() == 10);

global_asm!(
    r#"
    // Every exception except the breakpoint ends the run.
    .macro isr_fatal n
    .globl isr_\n
    isr_\n:
        push 0
        push \n
        jmp isr_fatal_common
    .endm

    isr_fatal 0
    isr_fatal 1
    isr_fatal 2
    isr_fatal 4
    isr_fatal 5
    isr_fatal 6
    isr_fatal 7
    isr_fatal 8
    isr_fatal 9
    isr_fatal 10
    isr_fatal 11
    isr_fatal 12
    isr_fatal 13
    isr_fatal 14
    isr_fatal 15
    isr_fatal 16
    isr_fatal 17
    isr_fatal 18
    isr_fatal 19
    isr_fatal 20
    isr_fatal 21
    isr_fatal 22
    isr_fatal 23
    isr_fatal 24
    isr_fatal 25
    isr_fatal 26
    isr_fatal 27
    isr_fatal 28
    isr_fatal 29
    isr_fatal 30
    isr_fatal 31

    // The breakpoint exception only counts itself and resumes. The CPU pushed
    // the return frame, so a bare `iretq` is enough. The counter is addressed
    // relative to RIP: an absolute symbol address does not fit in 32 bits.
    .globl isr_3
    isr_3:
        lock inc qword ptr [rip + {breakpoints}]
        iretq

    .globl isr_fatal_common
    isr_fatal_common:
        and rsp, -16
        call {fatal}
    "#,
    breakpoints = sym BREAKPOINTS_HANDLED,
    fatal = sym fatal_exception,
);

unsafe extern "C" {
    fn isr_0();
    fn isr_1();
    fn isr_2();
    fn isr_3();
    fn isr_4();
    fn isr_5();
    fn isr_6();
    fn isr_7();
    fn isr_8();
    fn isr_9();
    fn isr_10();
    fn isr_11();
    fn isr_12();
    fn isr_13();
    fn isr_14();
    fn isr_15();
    fn isr_16();
    fn isr_17();
    fn isr_18();
    fn isr_19();
    fn isr_20();
    fn isr_21();
    fn isr_22();
    fn isr_23();
    fn isr_24();
    fn isr_25();
    fn isr_26();
    fn isr_27();
    fn isr_28();
    fn isr_29();
    fn isr_30();
    fn isr_31();
}

impl IdtDescriptor {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            attributes: 0,
            offset_mid: 0,
            offset_high: 0,
            _reserved: 0,
        }
    }

    fn interrupt(stub: unsafe extern "C" fn(), selector: u16) -> Self {
        let address = stub as usize as u64;
        Self {
            offset_low: address as u16,
            selector,
            ist: INTERRUPT_STACK_INDEX,
            attributes: INTERRUPT_GATE,
            offset_mid: (address >> 16) as u16,
            offset_high: (address >> 32) as u32,
            _reserved: 0,
        }
    }
}

/// Owns the table the IDTR points at.
pub struct Idt {
    entries: Pin<Box<[IdtDescriptor; 0x100]>>,
}

impl Idt {
    pub fn new(selector: u16) -> Self {
        const STUBS: [unsafe extern "C" fn(); EXCEPTION_COUNT] = [
            isr_0, isr_1, isr_2, isr_3, isr_4, isr_5, isr_6, isr_7, isr_8, isr_9, isr_10, isr_11,
            isr_12, isr_13, isr_14, isr_15, isr_16, isr_17, isr_18, isr_19, isr_20, isr_21, isr_22,
            isr_23, isr_24, isr_25, isr_26, isr_27, isr_28, isr_29, isr_30, isr_31,
        ];

        let mut entries = [IdtDescriptor::missing(); 0x100];
        for (vector, stub) in STUBS.iter().enumerate() {
            entries[vector] = IdtDescriptor::interrupt(*stub, selector);
        }
        Self {
            entries: Box::pin(entries),
        }
    }

    pub fn load(&self) {
        let pointer = DescriptorTablePointer {
            limit: size_of::<[IdtDescriptor; 0x100]>() as u16 - 1,
            base: self.entries[..].as_ptr() as u64,
        };
        // SAFETY: `pointer` describes the table owned by `self`.
        unsafe {
            asm!("lidt [{}]", in(reg) &pointer, options(nostack, preserves_flags));
        }
    }
}

/// Reports an unexpected exception and stops the run. It is called from
/// assembly, so it takes no arguments and never returns.
extern "C" fn fatal_exception() {
    print_str("unexpected exception; stopping\n");
    exit_qemu(QemuExitCode::Fail);
}

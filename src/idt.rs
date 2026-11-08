//! Interrupt Descriptor Table and the exception entry stubs.
//!
//! Every vector gets a small assembly stub. Each stub records what happened in
//! the statics below before it hands over: the CPU pushed the interrupted RIP
//! (and an error code for some vectors) when it entered the stub, so the values
//! are read from the hardware frame before anything else is pushed.
//!
//! The breakpoint exception counts itself and returns to the interrupted code;
//! anything else reports and stops the run.

use crate::error;
use crate::qemu::QemuExitCode;
use crate::qemu::exit_qemu;
use crate::x86::DescriptorTablePointer;
use alloc::boxed::Box;
use core::arch::asm;
use core::arch::global_asm;
use core::mem::size_of;
use core::pin::Pin;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

/// How many breakpoint exceptions have been handled so far. It only grows, so a
/// test can prove that the handler ran and that execution resumed.
pub static BREAKPOINTS_HANDLED: AtomicU64 = AtomicU64::new(0);
/// Vector of the exception the stubs last handed over.
pub static LAST_VECTOR: AtomicU64 = AtomicU64::new(0);
/// Error code the CPU pushed for that exception (`0` when it pushes none).
pub static LAST_ERROR: AtomicU64 = AtomicU64::new(0);
/// Address of the instruction that raised it.
pub static LAST_RIP: AtomicU64 = AtomicU64::new(0);

/// Present, DPL 0, 64-bit interrupt gate.
const INTERRUPT_GATE: u8 = 0x8e;
/// IST index (1-based) of the stack the CPU switches to for every exception.
const INTERRUPT_STACK_INDEX: u8 = 1;

const EXCEPTION_COUNT: usize = 32;

const EXCEPTION_NAMES: [&str; EXCEPTION_COUNT] = [
    "divide error",
    "debug",
    "non-maskable interrupt",
    "breakpoint",
    "overflow",
    "bound range exceeded",
    "invalid opcode",
    "device not available",
    "double fault",
    "coprocessor segment overrun",
    "invalid TSS",
    "segment not present",
    "stack-segment fault",
    "general protection fault",
    "page fault",
    "reserved",
    "x87 floating-point exception",
    "alignment check",
    "machine check",
    "SIMD floating-point exception",
    "virtualization exception",
    "control protection exception",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "hypervisor injection exception",
    "VMM communication exception",
    "security exception",
    "reserved",
];

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

global_asm!(
    r#"
    // Vectors that push no error code: the hardware frame starts with RIP.
    .macro isr n
    .globl wasabi_isr_\n
    wasabi_isr_\n:
        push rax
        mov eax, \n
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {rip}], rax
        mov qword ptr [rip + {error}], 0
        pop rax
        jmp wasabi_isr_common
    .endm

    // Vectors that push an error code: it comes first on the stack.
    .macro isr_err n
    .globl wasabi_isr_\n
    wasabi_isr_\n:
        push rax
        mov eax, \n
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {error}], rax
        mov rax, qword ptr [rsp + 16]
        mov qword ptr [rip + {rip}], rax
        pop rax
        jmp wasabi_isr_common
    .endm

    isr     0
    isr     1
    isr     2
    isr     4
    isr     5
    isr     6
    isr     7
    isr_err 8
    isr     9
    isr_err 10
    isr_err 11
    isr_err 12
    isr_err 13
    isr_err 14
    isr     15
    isr     16
    isr_err 17
    isr     18
    isr     19
    isr     20
    isr_err 21
    isr     22
    isr     23
    isr     24
    isr     25
    isr     26
    isr     27
    isr     28
    isr_err 29
    isr_err 30
    isr     31

    // The breakpoint exception only counts itself and resumes.
    .globl wasabi_isr_3
    wasabi_isr_3:
        push rax
        mov eax, 3
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {rip}], rax
        mov qword ptr [rip + {error}], 0
        pop rax
        lock inc qword ptr [rip + {breakpoints}]
        iretq

    .globl wasabi_isr_common
    wasabi_isr_common:
        and rsp, -16
        call {fatal}
        ud2

    // The IDT entries are filled from this table, so the stubs are listed once.
    .section .rodata
    .globl wasabi_isr_table
    wasabi_isr_table:
        .quad wasabi_isr_0
        .quad wasabi_isr_1
        .quad wasabi_isr_2
        .quad wasabi_isr_3
        .quad wasabi_isr_4
        .quad wasabi_isr_5
        .quad wasabi_isr_6
        .quad wasabi_isr_7
        .quad wasabi_isr_8
        .quad wasabi_isr_9
        .quad wasabi_isr_10
        .quad wasabi_isr_11
        .quad wasabi_isr_12
        .quad wasabi_isr_13
        .quad wasabi_isr_14
        .quad wasabi_isr_15
        .quad wasabi_isr_16
        .quad wasabi_isr_17
        .quad wasabi_isr_18
        .quad wasabi_isr_19
        .quad wasabi_isr_20
        .quad wasabi_isr_21
        .quad wasabi_isr_22
        .quad wasabi_isr_23
        .quad wasabi_isr_24
        .quad wasabi_isr_25
        .quad wasabi_isr_26
        .quad wasabi_isr_27
        .quad wasabi_isr_28
        .quad wasabi_isr_29
        .quad wasabi_isr_30
        .quad wasabi_isr_31
    "#,
    vector = sym LAST_VECTOR,
    error = sym LAST_ERROR,
    rip = sym LAST_RIP,
    breakpoints = sym BREAKPOINTS_HANDLED,
    fatal = sym fatal_exception,
);

unsafe extern "C" {
    #[link_name = "wasabi_isr_table"]
    static ISR_TABLE: [unsafe extern "C" fn(); EXCEPTION_COUNT];
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
        let mut entries = [IdtDescriptor::missing(); 0x100];
        // SAFETY: the table is defined next to the stubs and counts them.
        let stubs = unsafe { ISR_TABLE };
        for (vector, stub) in stubs.iter().enumerate() {
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
    let vector = LAST_VECTOR.load(Ordering::Relaxed);
    let error_code = LAST_ERROR.load(Ordering::Relaxed);
    let rip = LAST_RIP.load(Ordering::Relaxed);
    let name = EXCEPTION_NAMES
        .get(vector as usize)
        .unwrap_or(&"unknown exception");
    error!("exception #{vector:02} ({name}) at {rip:#018x}, error code {error_code:#x}");
    exit_qemu(QemuExitCode::Fail);
}

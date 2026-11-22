use crate::error;
use crate::interrupts;
use crate::qemu::QemuExitCode;
use crate::qemu::exit_qemu;
use crate::x86::DescriptorTablePointer;
use crate::x86::read_cr2;
use alloc::boxed::Box;
use core::arch::asm;
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

/// CPU exceptions (0..31) plus the timer interrupt (32).
const VECTOR_COUNT: usize = 33;
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

unsafe extern "C" {
    #[link_name = "wasabi_isr_table"]
    static ISR_TABLE: [unsafe extern "C" fn(); VECTOR_COUNT];
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

/// Owns the table the IDTR points at; every vector is an entry from the stub
/// table, which records what happened before handing over.
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

/// Counts a timer interrupt and acknowledges it.
pub(crate) extern "C" fn timer_tick() {
    interrupts::TICKS.fetch_add(1, Ordering::Relaxed);
    interrupts::send_eoi();
}

/// Reports an unexpected exception and stops the run. It is called from
/// assembly, so it takes no arguments and never returns.
pub(crate) extern "C" fn fatal_exception() {
    let vector = LAST_VECTOR.load(Ordering::Relaxed);
    let error_code = LAST_ERROR.load(Ordering::Relaxed);
    let rip = LAST_RIP.load(Ordering::Relaxed);
    let name = EXCEPTION_NAMES
        .get(vector as usize)
        .unwrap_or(&"unknown exception");
    error!(
        "exception #{vector:02} ({name}) at {rip:#018x}, error code {error_code:#x}, cr2={:#018x}",
        read_cr2()
    );
    exit_qemu(QemuExitCode::Fail);
}

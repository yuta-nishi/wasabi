use core::sync::atomic::Ordering;
use wasabi::idt::BREAKPOINTS_HANDLED;
use wasabi::idt::LAST_RIP;
use wasabi::idt::LAST_VECTOR;

fn trigger_breakpoint() {
    // SAFETY: the test binary installs the IDT before running any test.
    unsafe { core::arch::asm!("int3") }
}

pub fn breakpoint_resumes_execution() {
    let before = BREAKPOINTS_HANDLED.load(Ordering::Relaxed);
    trigger_breakpoint();
    let after = BREAKPOINTS_HANDLED.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);
    assert_eq!(LAST_VECTOR.load(Ordering::Relaxed), 3);

    // A breakpoint is a trap: the CPU reports the address *after* `int3`, so the
    // byte just before the saved RIP must be the 0xcc opcode itself.
    let rip = LAST_RIP.load(Ordering::Relaxed);
    let just_executed = unsafe { *(rip as *const u8).sub(1) };
    assert_eq!(just_executed, 0xcc);
}

use core::sync::atomic::Ordering;
use wasabi::idt::BREAKPOINTS_HANDLED;

fn trigger_breakpoint() {
    // SAFETY: the test binary installs the IDT before running any test.
    unsafe { core::arch::asm!("int3") }
}

pub fn breakpoint_resumes_execution() {
    let before = BREAKPOINTS_HANDLED.load(Ordering::Relaxed);
    trigger_breakpoint();
    let after = BREAKPOINTS_HANDLED.load(Ordering::Relaxed);
    assert_eq!(after, before + 1);
}

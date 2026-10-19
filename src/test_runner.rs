use crate::print;
use crate::println;
use crate::qemu::QemuExitCode;
use crate::qemu::exit_qemu;

/// A `no_std` stand-in for the built-in test harness.
///
/// The book drives unit tests with `#![feature(custom_test_frameworks)]` and
/// `#[test_case]`, which are nightly-only. On stable the tests are listed here and
/// driven from our own UEFI entry point (`tests/runner.rs`) instead: each test is
/// called in turn, and a failing assertion ends the run from the panic handler
/// with `QemuExitCode::Fail`. Progress goes to the serial port.
pub fn run_tests(tests: &[(&str, fn())]) -> ! {
    println!("running {} tests", tests.len());
    for (name, test) in tests {
        print!("test {name} ... ");
        test();
        println!("ok");
    }
    println!("{} tests passed", tests.len());
    exit_qemu(QemuExitCode::Success)
}

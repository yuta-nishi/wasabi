use crate::print;
use crate::println;
use crate::qemu::QemuExitCode;
use crate::qemu::exit_qemu;

/// A `no_std` stand-in for the built-in test harness.
///
/// The built-in harness is nightly-only, so the caller lists the tests and this
/// runs them one by one from the UEFI entry point: a failing assertion ends the
/// run from the panic handler with `QemuExitCode::Fail`, and progress goes to the
/// serial port.
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

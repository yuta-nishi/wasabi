#![no_std]
#![no_main]

use core::panic::PanicInfo;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::test_runner::run_tests;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    _image_handle: EfiHandle,
    _efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    run_tests(&[])
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    exit_qemu(QemuExitCode::Fail);
}

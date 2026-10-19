#![no_std]
#![no_main]

// Lives in a subdirectory so cargo does not pick it up as a test target of its
// own: it is a helper module of this one.
#[path = "runner/allocator.rs"]
mod allocator;
#[path = "runner/debug.rs"]
mod debug;

use core::panic::PanicInfo;
use wasabi::allocator::ALLOCATOR;
use wasabi::println;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::test_runner::run_tests;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;
use wasabi::uefi::MemoryMapHolder;

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    _image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    // The tests allocate, so give them the same heap the OS builds: the
    // conventional-memory regions of the firmware's map.
    let mut memory_map = MemoryMapHolder::new();
    let status = efi_system_table
        .boot_services()
        .get_memory_map(&mut memory_map);
    assert_eq!(status, EfiStatus::Success, "get_memory_map failed");
    ALLOCATOR.init_with_mmap(&memory_map);

    run_tests(&[
        (
            "round_up_to_nearest_pow2_cases",
            allocator::round_up_to_nearest_pow2_cases,
        ),
        ("vec_grow_and_drop", allocator::vec_grow_and_drop),
        (
            "allocations_are_aligned",
            allocator::allocations_are_aligned,
        ),
        (
            "allocated_objects_have_no_overlap",
            allocator::allocated_objects_have_no_overlap,
        ),
        (
            "hexdump_prints_16_bytes_per_line",
            debug::hexdump_prints_16_bytes_per_line,
        ),
        (
            "hexdump_pads_the_last_line",
            debug::hexdump_pads_the_last_line,
        ),
    ])
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{info}");
    exit_qemu(QemuExitCode::Fail);
}

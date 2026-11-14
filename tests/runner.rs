#![no_std]
#![no_main]

// Lives in a subdirectory so cargo does not pick it up as a test target of its
// own: it is a helper module of this one.
#[path = "runner/allocator.rs"]
mod allocator;
#[path = "runner/debug.rs"]
mod debug;
#[path = "runner/executor.rs"]
mod executor;
#[path = "runner/interrupts.rs"]
mod interrupts;
#[path = "runner/paging.rs"]
mod paging;
#[path = "runner/timer.rs"]
mod timer;

use core::panic::PanicInfo;
use wasabi::error;
use wasabi::init::init_basic_runtime;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::test_runner::run_tests;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    // Give the tests the same runtime the OS builds: the heap comes from the
    // frozen memory map, and the firmware is gone before we install our own
    // GDT/IDT (its interrupt handlers would not survive the switch).
    let _memory_map = init_basic_runtime(image_handle, efi_system_table);
    let _exceptions = wasabi::exceptions::init();
    let rsdp = efi_system_table
        .acpi_rsdp()
        .expect("the firmware has no ACPI table");
    let hpet_base = wasabi::acpi::hpet_base(rsdp).expect("no HPET in ACPI");
    // SAFETY: the ACPI table points at the HPET's memory-mapped registers.
    unsafe { wasabi::timer::init(hpet_base) }.expect("HPET init failed");

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
        (
            "breakpoint_resumes_execution",
            interrupts::breakpoint_resumes_execution,
        ),
        (
            "four_level_walk_finds_a_4k_frame",
            paging::four_level_walk_finds_a_4k_frame,
        ),
        (
            "huge_page_ends_the_walk_early",
            paging::huge_page_ends_the_walk_early,
        ),
        (
            "missing_entry_reports_not_mapped",
            paging::missing_entry_reports_not_mapped,
        ),
        (
            "new_tables_map_requested_pages",
            paging::new_tables_map_requested_pages,
        ),
        (
            "unmapping_page_zero_leaves_it_not_mapped",
            paging::unmapping_page_zero_leaves_it_not_mapped,
        ),
        (
            "executor_runs_every_task_to_completion",
            executor::executor_runs_every_task_to_completion,
        ),
        (
            "yielding_lets_another_task_run",
            executor::yielding_lets_another_task_run,
        ),
        (
            "sleep_waits_for_its_deadline",
            timer::sleep_waits_for_its_deadline,
        ),
    ])
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("{info}");
    exit_qemu(QemuExitCode::Fail);
}

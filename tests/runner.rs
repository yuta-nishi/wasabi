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
#[path = "runner/input.rs"]
mod input;
#[path = "runner/interrupts.rs"]
mod interrupts;
#[path = "runner/mutex.rs"]
mod mutex;
#[path = "runner/paging.rs"]
mod paging;
#[path = "runner/timer.rs"]
mod timer;
#[path = "runner/xhci.rs"]
mod xhci;

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
    wasabi::init::init_timer(efi_system_table).expect("initializing the timer failed");

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
            "hexdump_prints_16_bytes_per_line_and_pads_the_last_one",
            debug::hexdump_prints_16_bytes_per_line_and_pads_the_last_one,
        ),
        (
            "breakpoint_resumes_execution",
            interrupts::breakpoint_resumes_execution,
        ),
        (
            "a_line_is_handed_over_when_enter_arrives",
            input::a_line_is_handed_over_when_enter_arrives,
        ),
        (
            "backspace_takes_one_byte_off_the_end",
            input::backspace_takes_one_byte_off_the_end,
        ),
        (
            "backspace_on_an_empty_line_is_harmless",
            input::backspace_on_an_empty_line_is_harmless,
        ),
        (
            "a_line_stops_growing_at_the_capacity",
            input::a_line_stops_growing_at_the_capacity,
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
            "pat_bit_in_a_leaf_entry_is_not_a_huge_page",
            paging::pat_bit_in_a_leaf_entry_is_not_a_huge_page,
        ),
        (
            "identity_map_uses_2m_pages",
            paging::identity_map_uses_2m_pages,
        ),
        (
            "unmapping_a_page_splits_the_huge_page_around_it",
            paging::unmapping_a_page_splits_the_huge_page_around_it,
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
        (
            "mutex_guards_can_be_taken_again_after_dropping",
            mutex::mutex_guards_can_be_taken_again_after_dropping,
        ),
        (
            "portsc_fields_come_out_of_the_word",
            xhci::portsc_fields_come_out_of_the_word,
        ),
        (
            "a_reset_port_reads_as_enabled",
            xhci::a_reset_port_reads_as_enabled,
        ),
        (
            "speed_maps_to_the_slot_context_value",
            xhci::speed_maps_to_the_slot_context_value,
        ),
        (
            "completion_codes_carry_names",
            xhci::completion_codes_carry_names,
        ),
        ("trb_types_have_names", xhci::trb_types_have_names),
        (
            "context_words_use_the_specification_positions",
            xhci::context_words_use_the_specification_positions,
        ),
        (
            "pci_command_fields_are_named",
            xhci::pci_command_fields_are_named,
        ),
    ])
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("{info}");
    exit_qemu(QemuExitCode::Fail);
}

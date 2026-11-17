//! Startup steps that turn a UEFI application into a standalone OS.

use crate::allocator::ALLOCATOR;
use crate::graphics::Bitmap;
use crate::graphics::draw_test_pattern;
use crate::graphics::fill_rect;
use crate::info;
use crate::paging::PAGE_SIZE;
use crate::paging::PageTables;
use crate::result::Result;
use crate::uefi::EfiHandle;
use crate::uefi::EfiMemoryType;
use crate::uefi::EfiSystemTable;
use crate::uefi::MemoryMapHolder;
use crate::uefi::exit_from_efi_boot_services;
use crate::uefi::init_vram;
use core::sync::atomic::Ordering;

/// Leaves the firmware behind: fetches the memory map, exits boot services and
/// hands the now-frozen conventional-memory regions to the heap allocator.
///
/// The map has to be fetched before `ExitBootServices` (only the firmware can
/// produce it) and used after it (until then the firmware may still allocate
/// from conventional memory, which would invalidate the map).
pub fn init_basic_runtime(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> MemoryMapHolder {
    let mut memory_map = MemoryMapHolder::new();
    exit_from_efi_boot_services(image_handle, efi_system_table, &mut memory_map);
    ALLOCATOR.init_with_mmap(&memory_map);
    let regions = memory_map
        .iter()
        .filter(|e| e.memory_type() == EfiMemoryType::CONVENTIONAL_MEMORY)
        .count();
    info!("the heap is built from {regions} conventional-memory regions");
    memory_map
}

/// Builds page tables that identity-map the memory this OS uses and everything
/// below 4 GiB (which also covers the firmware's MMIO).
pub fn init_paging(memory_map: &MemoryMapHolder) -> PageTables {
    let mut end = 0x1_0000_0000u64;
    for e in memory_map.iter() {
        if e.memory_type() == EfiMemoryType::CONVENTIONAL_MEMORY {
            end = end.max(e.physical_start() + e.number_of_pages() * PAGE_SIZE);
        }
    }
    let mut tables = PageTables::identity_map(end);
    // Leaving page 0 unmapped turns a null pointer dereference into a page
    // fault instead of a silent read of address zero.
    tables.unmap_page(0);
    info!("paging: identity-mapped {PAGE_SIZE:#x}..{end:#x}, page 0 is unmapped");
    tables
}

/// Takes over the firmware's framebuffer, clears it, paints the test pattern and
/// makes it the screen `print!` writes to.
pub fn init_screen(efi_system_table: &EfiSystemTable) -> Result<()> {
    let mut vram = init_vram(efi_system_table)?;
    let (width, height) = (vram.width(), vram.height());
    fill_rect(&mut vram, 0x000000, 0, 0, width, height)?;
    draw_test_pattern(&mut vram);
    crate::console::install_screen(vram);
    Ok(())
}

/// Starts the timer interrupt: remaps the PIC, drives the PIT at `hz`, enables
/// interrupts, and waits for the first tick.
///
/// Both halves matter. Without the PIT nothing ever wakes the executor, and
/// without `sti` the CPU never even takes the interrupt; either way the tasks
/// would stop after their first poll and the executor would spin in `hlt`.
pub fn init_interrupts(hz: u32) -> Result<()> {
    crate::interrupts::remap_pic();
    crate::interrupts::start_pit(hz);
    // SAFETY: `exceptions::init` has loaded an IDT, and the timer stub sends
    // EOI, so the interrupt is handled and execution resumes.
    crate::x86::enable_interrupts();

    // Prove that a tick arrives, and fail here rather than hanging in `hlt` if
    // it never does. The HPET is a separate clock from the PIT, so it can bound
    // the wait without trusting the interrupt being waited for.
    let start = crate::interrupts::TICKS.load(Ordering::Relaxed);
    let deadline = crate::timer::nanoseconds() + 10 * u64::from(1_000_000_000u32 / hz);
    while crate::interrupts::TICKS.load(Ordering::Relaxed) == start {
        if crate::timer::nanoseconds() >= deadline {
            return Err("the timer interrupt never fired");
        }
        core::hint::spin_loop();
    }
    info!("interrupts: PIC remapped, PIT at {hz} Hz, interrupts enabled");
    Ok(())
}

/// Finds the HPET through ACPI and starts it, so `timer::nanoseconds()` works.
pub fn init_timer(efi_system_table: &EfiSystemTable) -> Result<()> {
    let rsdp = efi_system_table
        .acpi_rsdp()
        .ok_or("the firmware has no ACPI table")?;
    let base = crate::timer::base_from_acpi(rsdp)?;
    info!("timer: HPET registers at {base:#018x}");
    // SAFETY: the ACPI table points at the HPET's memory-mapped registers.
    unsafe { crate::timer::init(base) }
}

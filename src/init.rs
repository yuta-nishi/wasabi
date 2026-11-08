//! Startup steps that turn a UEFI application into a standalone OS.

use crate::allocator::ALLOCATOR;
use crate::info;
use crate::paging::PAGE_SIZE;
use crate::paging::PageTables;
use crate::uefi::EfiHandle;
use crate::uefi::EfiMemoryType;
use crate::uefi::EfiSystemTable;
use crate::uefi::MemoryMapHolder;
use crate::uefi::exit_from_efi_boot_services;

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
    let tables = PageTables::identity_map(end);
    info!("paging: identity-mapped 0..{end:#x} with our own tables");
    tables
}

//! Startup steps that turn a UEFI application into a standalone OS.

use crate::allocator::ALLOCATOR;
use crate::uefi::EfiHandle;
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
    memory_map
}

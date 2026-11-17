//! The boot services table, the system table, and the protocols we borrow.

use super::ffi::EfiGuid;
use super::ffi::EfiHandle;
use super::ffi::EfiStatus;
use super::ffi::Out;
use super::memory_map::MemoryMapHolder;
use crate::acpi::Rsdp;
use crate::result::Result;
use core::ffi::c_void;
use core::mem::offset_of;
use core::ptr::NonNull;
use core::ptr::null_mut;

const EFI_ACPI_TABLE_GUID: EfiGuid = EfiGuid {
    data0: 0x8868_e871,
    data1: 0xe4f1,
    data2: 0x11d3,
    data3: [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
};

const EFI_LOADED_IMAGE_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data0: 0x5b1b31a1,
    data1: 0x9562,
    data2: 0x11d2,
    data3: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// `EFI_BOOT_SERVICES.GetMemoryMap` (UEFI spec, Memory Allocation Services).
///
/// `memory_map_size` is in/out: it holds the buffer size on entry, and on
/// `BufferTooSmall` the firmware writes the required size into it.
type GetMemoryMapFn = extern "efiapi" fn(
    memory_map_size: *mut usize,
    memory_map: *mut u8,
    map_key: *mut usize,
    descriptor_size: *mut usize,
    descriptor_version: *mut u32,
) -> EfiStatus;

/// `EFI_BOOT_SERVICES.ExitBootServices` (UEFI spec 7.4.6).
///
/// `map_key` must come from the most recent `GetMemoryMap`; the call fails if the
/// map changed in between, in which case the map has to be fetched again.
type ExitBootServicesFn =
    extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus;

/// `EFI_BOOT_SERVICES.HandleProtocol` (UEFI spec 7.3.12).
type HandleProtocolFn = extern "efiapi" fn(
    handle: EfiHandle,
    protocol: *const EfiGuid,
    out_interface: Out<*mut c_void>,
) -> EfiStatus;

/// `EFI_BOOT_SERVICES.LocateProtocol` (UEFI spec 7.3.16).
///
/// `registration` is OPTIONAL: pass null to locate a protocol without
/// subscribing to protocol-notify events.
type LocateProtocolFn = extern "efiapi" fn(
    protocol: *const EfiGuid,
    registration: *mut c_void,
    out_interface: Out<*mut c_void>,
) -> EfiStatus;

#[repr(C)]
pub struct EfiBootServicesTable {
    _reserved0: [u64; 7],
    get_memory_map_fn: GetMemoryMapFn,
    _reserved1: [u64; 11],
    handle_protocol_fn: HandleProtocolFn,
    _reserved2: [u64; 9],
    exit_boot_services_fn: ExitBootServicesFn,
    _reserved3: [u64; 10],
    locate_protocol_fn: LocateProtocolFn,
}
const _: () = assert!(offset_of!(EfiBootServicesTable, get_memory_map_fn) == 56);
const _: () = assert!(offset_of!(EfiBootServicesTable, handle_protocol_fn) == 152);
const _: () = assert!(offset_of!(EfiBootServicesTable, exit_boot_services_fn) == 232);
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol_fn) == 320);

impl EfiBootServicesTable {
    /// Fills `map` with the firmware's current memory map.
    pub fn get_memory_map(&self, map: &mut MemoryMapHolder) -> EfiStatus {
        let get_memory_map_fn = self.get_memory_map_fn;
        get_memory_map_fn(
            &mut map.memory_map_size,
            map.memory_map_buffer.as_mut_ptr(),
            &mut map.map_key,
            &mut map.descriptor_size,
            &mut map.descriptor_version,
        )
    }

    /// Borrows the `protocol` interface attached to `handle`.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::locate_protocol`].
    pub(super) unsafe fn handle_protocol<'a, T>(
        &self,
        handle: EfiHandle,
        protocol: &EfiGuid,
    ) -> Option<&'a T> {
        let mut out: *mut T = null_mut();
        let handle_protocol_fn = self.handle_protocol_fn;
        let status =
            handle_protocol_fn(handle, protocol, &raw mut out as Out<*mut c_void>);
        if status != EfiStatus::Success {
            return None;
        }
        // SAFETY: written by the firmware on success; non-null is checked below.
        let out = NonNull::new(out)?;
        Some(unsafe { out.as_ref() })
    }

    /// Locates the protocol identified by `protocol` and borrows its interface.
    ///
    /// # Safety
    ///
    /// `'a` is chosen by the caller: they must ensure the located interface stays
    /// valid and is not mutated for `'a`.
    pub(super) unsafe fn locate_protocol<'a, T>(
        &self,
        protocol: &EfiGuid,
    ) -> Option<&'a T> {
        let mut out: *mut T = null_mut();
        let locate_protocol_fn = self.locate_protocol_fn;
        let status =
            locate_protocol_fn(protocol, null_mut(), &raw mut out as Out<*mut c_void>);
        if status != EfiStatus::Success {
            return None;
        }
        // SAFETY: written by the firmware on success; non-null is checked below.
        let out = NonNull::new(out)?;
        Some(unsafe { out.as_ref() })
    }
}

/// One entry of the system table's configuration table (UEFI spec 4.6).
#[repr(C)]
struct EfiConfigurationTable {
    vendor_guid: EfiGuid,
    vendor_table: *const u8,
}

#[repr(C)]
pub struct EfiSystemTable {
    _reserved0: [u64; 12],
    pub(super) boot_services: &'static EfiBootServicesTable,
    number_of_table_entries: usize,
    configuration_table: *const EfiConfigurationTable,
}
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);
const _: () = assert!(offset_of!(EfiSystemTable, number_of_table_entries) == 104);
const _: () = assert!(offset_of!(EfiSystemTable, configuration_table) == 112);

impl EfiSystemTable {
    pub fn boot_services(&self) -> &EfiBootServicesTable {
        self.boot_services
    }

    /// The ACPI root pointer the firmware publishes in its configuration table.
    pub fn acpi_rsdp(&self) -> Option<&'static Rsdp> {
        // SAFETY: the firmware provides `number_of_table_entries` entries, and
        // they live as long as its own tables do.
        let entries = unsafe {
            core::slice::from_raw_parts(
                self.configuration_table,
                self.number_of_table_entries,
            )
        };
        let entry = entries
            .iter()
            .find(|entry| entry.vendor_guid == EFI_ACPI_TABLE_GUID)?;
        // SAFETY: the ACPI GUID means the vendor table is the RSDP.
        let rsdp = unsafe { &*(entry.vendor_table as *const Rsdp) };
        rsdp.is_valid().then_some(rsdp)
    }
}

/// `EFI_LOADED_IMAGE_PROTOCOL` (UEFI spec 9.2).
#[repr(C)]
pub struct EfiLoadedImageProtocol {
    revision: u32,
    parent_handle: EfiHandle,
    system_table: *mut c_void,
    device_handle: EfiHandle,
    file_path: *mut c_void,
    _reserved: *mut c_void,
    load_options_size: u32,
    load_options: *mut c_void,
    /// Where the firmware loaded this image, and how big it is.
    pub image_base: *mut c_void,
    pub image_size: u64,
    image_code_type: u32,
    image_data_type: u32,
    unload: *mut c_void,
}

/// The `LoadedImage` protocol of the running image: its base address and size.
pub fn locate_loaded_image_protocol<'a>(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> Result<&'a EfiLoadedImageProtocol> {
    // SAFETY: the protocol is owned by the firmware and outlives this program.
    unsafe {
        efi_system_table
            .boot_services
            .handle_protocol(image_handle, &EFI_LOADED_IMAGE_PROTOCOL_GUID)
    }
    .ok_or("Failed to locate the loaded image protocol")
}

/// Hands control over from the firmware to us: after this returns, boot services
/// (including `GetMemoryMap`) must not be called any more.
///
/// The memory map can change between the two calls, so re-fetch it until
/// `ExitBootServices` succeeds.
pub fn exit_from_efi_boot_services(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
    memory_map: &mut MemoryMapHolder,
) {
    loop {
        let status = efi_system_table.boot_services.get_memory_map(memory_map);
        assert_eq!(status, EfiStatus::Success);
        let exit_boot_services_fn = efi_system_table.boot_services.exit_boot_services_fn;
        let status = exit_boot_services_fn(image_handle, memory_map.map_key);
        if status == EfiStatus::Success {
            break;
        }
    }
}

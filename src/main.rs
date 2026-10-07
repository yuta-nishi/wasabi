#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::mem::offset_of;
use core::mem::size_of;
use core::panic::PanicInfo;
use core::ptr::NonNull;
use core::ptr::null_mut;
use core::slice;

type EfiHandle = u64;

/// A C output parameter: a pointer through which the callee writes a `T`.
type Out<T> = *mut T;

type Result<T, E = &'static str> = core::result::Result<T, E>;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EfiGuid {
    pub data0: u32,
    pub data1: u16,
    pub data2: u16,
    pub data3: [u8; 8],
}

const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data0: 0x9042a9de,
    data1: 0x23dc,
    data2: 0x4a38,
    data3: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[must_use]
#[repr(u64)]
enum EfiStatus {
    Success = 0,
}

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
struct EfiBootServicesTable {
    _reserved0: [u64; 40],
    locate_protocol_fn: LocateProtocolFn,
}
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol_fn) == 320);

impl EfiBootServicesTable {
    /// Locates the protocol identified by `protocol` and borrows its interface.
    ///
    /// # Safety
    ///
    /// `'a` is chosen by the caller: they must ensure the located interface stays
    /// valid and is not mutated for `'a`.
    unsafe fn locate_protocol<'a, T>(&self, protocol: &EfiGuid) -> Option<&'a T> {
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

#[repr(C)]
struct EfiSystemTable {
    _reserved0: [u64; 12],
    pub boot_services: &'static EfiBootServicesTable,
}
const _: () = assert!(offset_of!(EfiSystemTable, boot_services) == 96);

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolPixelInfo {
    version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    _padding0: [u32; 5],
    pub pixels_per_scan_line: u32,
}
const _: () = assert!(size_of::<EfiGraphicsOutputProtocolPixelInfo>() == 36);

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocolMode<'a> {
    pub max_mode: u32,
    pub mode: u32,
    pub info: &'a EfiGraphicsOutputProtocolPixelInfo,
    pub size_of_info: u64,
    pub frame_buffer_base: usize,
    pub frame_buffer_size: usize,
}

#[repr(C)]
#[derive(Debug)]
struct EfiGraphicsOutputProtocol<'a> {
    reserved: [u64; 3],
    pub mode: &'a EfiGraphicsOutputProtocolMode<'a>,
}

fn locate_graphic_protocol<'a>(
    efi_system_table: &EfiSystemTable,
) -> Result<&'a EfiGraphicsOutputProtocol<'a>> {
    // SAFETY: the GOP interface is owned by the firmware and lives for the whole
    // lifetime of this program, so borrowing it for `'a` is sound.
    unsafe {
        efi_system_table
            .boot_services
            .locate_protocol(&EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID)
    }
    .ok_or("Failed to locate graphics output protocol")
}

pub fn hlt() {
    unsafe { asm!("hlt") }
}

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    _image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    let efi_graphics_output_protocol = locate_graphic_protocol(efi_system_table).unwrap();
    let vram_addr = efi_graphics_output_protocol.mode.frame_buffer_base;
    let vram_byte_size = efi_graphics_output_protocol.mode.frame_buffer_size;
    // SAFETY: `frame_buffer_base`/`frame_buffer_size` describe the linear
    // framebuffer the firmware handed us; it is valid for this many bytes.
    let vram = unsafe {
        slice::from_raw_parts_mut(
            vram_addr as *mut u32,
            vram_byte_size / size_of::<u32>(),
        )
    };
    for e in vram {
        *e = 0xffffff;
    }
    loop {
        hlt()
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()
    }
}

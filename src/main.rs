#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::fmt;
use core::fmt::Write;
use core::mem::offset_of;
use core::mem::size_of;
use core::panic::PanicInfo;
use core::ptr::NonNull;
use core::ptr::null_mut;
use core::writeln;
use wasabi::graphics::Bitmap;
use wasabi::graphics::draw_font_fg;
use wasabi::graphics::draw_test_pattern;
use wasabi::graphics::fill_rect;
use wasabi::result::Result;

type EfiHandle = u64;

/// A C output parameter: a pointer through which the callee writes a `T`.
type Out<T> = *mut T;

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

/// The `EFI_MEMORY_TYPE` values that describe what a memory region is for.
#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum EfiMemoryType {
    RESERVED = 0,
    LOADER_CODE,
    LOADER_DATA,
    BOOT_SERVICES_CODE,
    BOOT_SERVICES_DATA,
    RUNTIME_SERVICES_CODE,
    RUNTIME_SERVICES_DATA,
    CONVENTIONAL_MEMORY,
    UNUSABLE_MEMORY,
    ACPI_RECLAIM_MEMORY,
    ACPI_MEMORY_NVS,
    MEMORY_MAPPED_IO,
    MEMORY_MAPPED_IO_PORT_SPACE,
    PAL_CODE,
    PERSISTENT_MEMORY,
}

/// One entry of the memory map that `GetMemoryMap` returns.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EfiMemoryDescriptor {
    memory_type: EfiMemoryType,
    physical_start: u64,
    virtual_start: u64,
    number_of_pages: u64,
    attribute: u64,
}

const MEMORY_MAP_BUFFER_SIZE: usize = 0x8000;

/// The caller-provided buffer plus the metadata `GetMemoryMap` fills in.
struct MemoryMapHolder {
    memory_map_buffer: [u8; MEMORY_MAP_BUFFER_SIZE],
    memory_map_size: usize,
    map_key: usize,
    descriptor_size: usize,
    descriptor_version: u32,
}

struct MemoryMapIterator<'a> {
    map: &'a MemoryMapHolder,
    ofs: usize,
}

impl<'a> Iterator for MemoryMapIterator<'a> {
    type Item = &'a EfiMemoryDescriptor;
    fn next(&mut self) -> Option<&'a EfiMemoryDescriptor> {
        if self.ofs >= self.map.memory_map_size {
            None
        } else {
            // SAFETY: `ofs` stays inside the buffer because it is advanced by
            // `descriptor_size` until it reaches `memory_map_size`.
            let e: &EfiMemoryDescriptor = unsafe {
                &*(self.map.memory_map_buffer.as_ptr().add(self.ofs)
                    as *const EfiMemoryDescriptor)
            };
            self.ofs += self.map.descriptor_size;
            Some(e)
        }
    }
}

impl MemoryMapHolder {
    pub const fn new() -> MemoryMapHolder {
        MemoryMapHolder {
            memory_map_buffer: [0; MEMORY_MAP_BUFFER_SIZE],
            memory_map_size: MEMORY_MAP_BUFFER_SIZE,
            map_key: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }
    pub fn iter(&self) -> MemoryMapIterator<'_> {
        MemoryMapIterator { map: self, ofs: 0 }
    }
}

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
    _reserved0: [u64; 7],
    get_memory_map_fn: GetMemoryMapFn,
    _reserved1: [u64; 21],
    exit_boot_services_fn: ExitBootServicesFn,
    _reserved2: [u64; 10],
    locate_protocol_fn: LocateProtocolFn,
}
const _: () = assert!(offset_of!(EfiBootServicesTable, get_memory_map_fn) == 56);
const _: () = assert!(offset_of!(EfiBootServicesTable, exit_boot_services_fn) == 232);
const _: () = assert!(offset_of!(EfiBootServicesTable, locate_protocol_fn) == 320);

impl EfiBootServicesTable {
    /// Fills `map` with the firmware's current memory map.
    fn get_memory_map(&self, map: &mut MemoryMapHolder) -> EfiStatus {
        let get_memory_map_fn = self.get_memory_map_fn;
        get_memory_map_fn(
            &mut map.memory_map_size,
            map.memory_map_buffer.as_mut_ptr(),
            &mut map.map_key,
            &mut map.descriptor_size,
            &mut map.descriptor_version,
        )
    }

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
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    let vw = vram.width;
    let vh = vram.height;
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    draw_test_pattern(&mut vram);
    let mut w = VramTextWriter::new(&mut vram);
    for i in 0..4 {
        writeln!(w, "i = {i}").unwrap();
    }
    let mut memory_map = MemoryMapHolder::new();
    let status = efi_system_table
        .boot_services
        .get_memory_map(&mut memory_map);
    writeln!(w, "{status:?}").unwrap();
    let mut total_memory_pages = 0;
    for e in memory_map.iter() {
        if e.memory_type != EfiMemoryType::CONVENTIONAL_MEMORY {
            continue;
        }
        total_memory_pages += e.number_of_pages;
        writeln!(w, "{e:?}").unwrap();
    }
    let total_memory_size_mib = total_memory_pages * 4096 / 1024 / 1024;
    writeln!(
        w,
        "Total: {total_memory_pages} pages = {total_memory_size_mib} MiB"
    )
    .unwrap();
    exit_from_efi_boot_services(image_handle, efi_system_table, &mut memory_map);
    writeln!(w, "Hello, Non-UEFI world!").unwrap();
    loop {
        hlt()
    }
}

/// Hands control over from the firmware to us: after this returns, boot services
/// (including `GetMemoryMap`) must not be called any more.
///
/// The memory map can change between the two calls, so re-fetch it until
/// `ExitBootServices` succeeds.
fn exit_from_efi_boot_services(
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

#[derive(Clone, Copy)]
struct VramBufferInfo {
    buf: *mut u8,
    width: i64,
    height: i64,
    pixels_per_line: i64,
}

impl Bitmap for VramBufferInfo {
    fn bytes_per_pixel(&self) -> i64 {
        4
    }
    fn pixels_per_line(&self) -> i64 {
        self.pixels_per_line
    }
    fn width(&self) -> i64 {
        self.width
    }
    fn height(&self) -> i64 {
        self.height
    }
    fn buf_mut(&mut self) -> *mut u8 {
        self.buf
    }
}

fn init_vram(efi_system_table: &EfiSystemTable) -> Result<VramBufferInfo> {
    let gp = locate_graphic_protocol(efi_system_table)?;
    Ok(VramBufferInfo {
        buf: gp.mode.frame_buffer_base as *mut u8,
        width: gp.mode.info.horizontal_resolution as i64,
        height: gp.mode.info.vertical_resolution as i64,
        pixels_per_line: gp.mode.info.pixels_per_scan_line as i64,
    })
}

struct VramTextWriter<'a> {
    vram: &'a mut VramBufferInfo,
    cursor_x: i64,
    cursor_y: i64,
}
impl<'a> VramTextWriter<'a> {
    fn new(vram: &'a mut VramBufferInfo) -> Self {
        Self {
            vram,
            cursor_x: 0,
            cursor_y: 0,
        }
    }
}
impl fmt::Write for VramTextWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                self.cursor_y += 16;
                self.cursor_x = 0;
                continue;
            }
            draw_font_fg(self.vram, self.cursor_x, self.cursor_y, 0xffffff, c);
            self.cursor_x += 8;
        }
        Ok(())
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        hlt()
    }
}

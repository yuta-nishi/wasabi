#![no_std]
#![no_main]

use core::fmt::Write;
use core::panic::PanicInfo;
use core::writeln;
use wasabi::allocator::ALLOCATOR;
use wasabi::graphics::Bitmap;
use wasabi::graphics::draw_test_pattern;
use wasabi::graphics::fill_rect;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiMemoryType;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;
use wasabi::uefi::MemoryMapHolder;
use wasabi::uefi::VramTextWriter;
use wasabi::uefi::exit_from_efi_boot_services;
use wasabi::uefi::init_vram;
use wasabi::x86::hlt;

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    let mut vram = init_vram(efi_system_table).expect("init_vram failed");
    let vw = vram.width();
    let vh = vram.height();
    fill_rect(&mut vram, 0x000000, 0, 0, vw, vh).expect("fill_rect failed");
    draw_test_pattern(&mut vram);
    let mut w = VramTextWriter::new(&mut vram);
    for i in 0..4 {
        writeln!(w, "i = {i}").unwrap();
    }
    let mut memory_map = MemoryMapHolder::new();
    let status = efi_system_table
        .boot_services()
        .get_memory_map(&mut memory_map);
    writeln!(w, "{status:?}").unwrap();
    let mut total_memory_pages = 0;
    for e in memory_map.iter() {
        if e.memory_type() != EfiMemoryType::CONVENTIONAL_MEMORY {
            continue;
        }
        total_memory_pages += e.number_of_pages();
        writeln!(w, "{e:?}").unwrap();
    }
    let total_memory_size_mib = total_memory_pages * 4096 / 1024 / 1024;
    writeln!(
        w,
        "Total: {total_memory_pages} pages = {total_memory_size_mib} MiB"
    )
    .unwrap();
    exit_from_efi_boot_services(image_handle, efi_system_table, &mut memory_map);
    // Until ExitBootServices the firmware may still allocate from conventional
    // memory, so the heap is only carved out of the now-frozen map.
    ALLOCATOR.init_with_mmap(&memory_map);
    writeln!(w, "Hello, Non-UEFI world!").unwrap();
    loop {
        hlt()
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    exit_qemu(QemuExitCode::Fail);
}

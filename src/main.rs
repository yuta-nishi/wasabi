#![no_std]
#![no_main]

use core::panic::PanicInfo;
use wasabi::demo;
use wasabi::error;
use wasabi::exceptions;
use wasabi::info;
use wasabi::init::init_basic_runtime;
use wasabi::init::init_interrupts;
use wasabi::init::init_paging;
use wasabi::init::init_pci;
use wasabi::init::init_screen;
use wasabi::init::init_serial;
use wasabi::init::init_timer;
use wasabi::paging;
use wasabi::println;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiMemoryType;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;
use wasabi::uefi::locate_loaded_image_protocol;

#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(
    image_handle: EfiHandle,
    efi_system_table: &EfiSystemTable,
) -> EfiStatus {
    init_serial().expect("initializing the serial port failed");
    init_screen(efi_system_table).expect("init_screen failed");
    let loaded_image = locate_loaded_image_protocol(image_handle, efi_system_table)
        .expect("locate_loaded_image_protocol failed");
    info!(
        "image: base={:#018x} size={:#x}",
        loaded_image.image_base as usize, loaded_image.image_size
    );
    let memory_map = init_basic_runtime(image_handle, efi_system_table);
    let mut total_memory_pages = 0;
    for e in memory_map.iter() {
        if e.memory_type() != EfiMemoryType::CONVENTIONAL_MEMORY {
            continue;
        }
        total_memory_pages += e.number_of_pages();
        println!("{e:?}");
    }
    let total_memory_size_mib = total_memory_pages * 4096 / 1024 / 1024;
    println!("Total: {total_memory_pages} pages = {total_memory_size_mib} MiB");
    println!("Hello, Non-UEFI world!");
    let _exceptions = exceptions::init();
    paging::dump_root();
    paging::dump_translation(0);
    paging::dump_translation(efi_main as *const () as usize as u64);

    init_timer(efi_system_table).expect("initializing the timer failed");

    let mut tables = init_paging(&memory_map);
    tables.switch();
    info!("paging: switched to our own tables");
    paging::dump_root();

    init_pci(efi_system_table, &mut tables).expect("listing the PCI devices failed");

    init_interrupts(1_000).expect("initializing the timer interrupt failed");

    demo::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("{info}");
    exit_qemu(QemuExitCode::Fail);
}

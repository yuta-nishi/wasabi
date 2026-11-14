#![no_std]
#![no_main]

use core::fmt::Write;
use core::panic::PanicInfo;
use core::writeln;
use wasabi::error;
use wasabi::exceptions;
use wasabi::executor::Executor;
use wasabi::executor::yield_now;
use wasabi::graphics::Bitmap;
use wasabi::graphics::draw_test_pattern;
use wasabi::graphics::fill_rect;
use wasabi::info;
use wasabi::init::init_basic_runtime;
use wasabi::init::init_paging;
use wasabi::paging;
use wasabi::println;
use wasabi::qemu::QemuExitCode;
use wasabi::qemu::exit_qemu;
use wasabi::uefi::EfiHandle;
use wasabi::uefi::EfiMemoryType;
use wasabi::uefi::EfiStatus;
use wasabi::uefi::EfiSystemTable;
use wasabi::uefi::VramTextWriter;
use wasabi::uefi::init_vram;
use wasabi::uefi::locate_loaded_image_protocol;

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
        writeln!(w, "{e:?}").unwrap();
    }
    let total_memory_size_mib = total_memory_pages * 4096 / 1024 / 1024;
    writeln!(
        w,
        "Total: {total_memory_pages} pages = {total_memory_size_mib} MiB"
    )
    .unwrap();
    writeln!(w, "Hello, Non-UEFI world!").unwrap();
    println!("Hello, Non-UEFI world!");
    let _exceptions = exceptions::init();
    paging::dump_root();
    paging::dump_translation(0);
    paging::dump_translation(efi_main as *const () as usize as u64);

    let tables = init_paging(&memory_map);
    tables.switch();
    info!("paging: switched to our own tables");
    paging::dump_root();

    EXECUTOR.spawn(async {
        for step in 0..3 {
            info!("task a: step {step}");
            yield_now().await;
        }
        info!("task a: done");
    });
    EXECUTOR.spawn(async {
        for step in 0..3 {
            info!("task b: step {step}");
            yield_now().await;
        }
        info!("task b: done");
    });
    EXECUTOR.run()
}

/// The executor the OS runs its tasks on.
static EXECUTOR: Executor = Executor::new();

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    error!("{info}");
    exit_qemu(QemuExitCode::Fail);
}

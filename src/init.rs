use crate::allocator::ALLOCATOR;
use crate::console;
use crate::graphics::Bitmap;
use crate::graphics::draw_test_pattern;
use crate::graphics::fill_rect;
use crate::info;
use crate::interrupts;
use crate::paging::PAGE_SIZE;
use crate::paging::PageTables;
use crate::pci::Bar;
use crate::pci::ConfigSpace;
use crate::result::Result;
use crate::serial::Serial;
use crate::timer;
use crate::uefi::EfiHandle;
use crate::uefi::EfiMemoryType;
use crate::uefi::EfiSystemTable;
use crate::uefi::MemoryMapHolder;
use crate::uefi::exit_from_efi_boot_services;
use crate::uefi::init_vram;
use crate::warn;
use crate::x86;
use crate::xhci::HostController;
use crate::xhci::Registers;
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

/// Finds the ECAM window described by ACPI, lists the functions behind it, maps
/// and enables the xHCI controller, and returns its register groups.
pub fn init_pci(
    efi_system_table: &EfiSystemTable,
    tables: &mut PageTables,
) -> Result<Registers> {
    let rsdp = efi_system_table
        .acpi_rsdp()
        .ok_or("the firmware has no ACPI table")?;
    let pci = ConfigSpace::from_acpi(rsdp)?;
    let xhci = pci.probe().ok_or("no xHCI controller on the PCI bus")?;
    let Some(Bar::Memory {
        address,
        size,
        prefetchable,
    }) = pci.bar(xhci, 0)
    else {
        return Err("the xHCI controller has no memory BAR0");
    };
    if prefetchable {
        // Prefetchable promises that reads have no side effects, which is the
        // opposite of a register window: the CPU may read it speculatively.
        warn!("pci: the xHCI register window is marked prefetchable");
    }
    let pages = size.div_ceil(PAGE_SIZE);
    info!("pci: xHCI at {xhci}, BAR0 {address:#018x} ({pages} pages, {size:#x} bytes)");
    // The controller's registers are device memory: map the window the way the
    // device describes it, uncached, whether or not the identity map happened to
    // cover it.
    for page in 0..pages {
        let offset = page * PAGE_SIZE;
        tables.map_page_uncached(address + offset, address + offset);
    }
    // Without these bits the controller ignores the register window and cannot
    // read the rings, which is everything the driver does.
    pci.enable(xhci);
    info!("pci: xHCI command register is {:#06x}", pci.command(xhci));
    // SAFETY: the window is mapped above and holds the controller's registers.
    let capability = unsafe { core::ptr::read_volatile(address as *const u32) };
    info!(
        "pci: xHCI capability {capability:#010x} (caplength {:#04x}, version {:#06x})",
        (capability & 0xff) as u8,
        ((capability >> 16) & 0xffff) as u16
    );
    Registers::new(address)
}

/// Resets the xHCI controller, gives it its rings, starts it, and proves the
/// command path with a no-op command.
pub fn init_xhci(registers: Registers, tables: &mut PageTables) -> Result<()> {
    let mut host = HostController::init(registers, tables)?;
    info!("xhci: version {:#06x}", host.version());
    host.enumerate_ports()?;
    Ok(())
}

/// Programs COM1 and proves both directions work by looping a byte back.
pub fn init_serial() -> Result<()> {
    let mut serial = Serial;
    serial.init();
    serial.loopback_test()?;
    info!("serial: COM1 at 115200 8N1 with the FIFO on, loopback test passed");
    Ok(())
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
    let started = timer::nanoseconds();
    let mut tables = PageTables::identity_map(end);
    let elapsed_ms = (timer::nanoseconds() - started) / 1_000_000;
    // Leaving page 0 unmapped turns a null pointer dereference into a page
    // fault instead of a silent read of address zero.
    tables.unmap_page(0);
    info!(
        "paging: identity-mapped {PAGE_SIZE:#x}..{end:#x} in {elapsed_ms} ms, page 0 is unmapped"
    );
    tables
}

/// Takes over the firmware's framebuffer, clears it, paints the test pattern and
/// makes it the screen `print!` writes to.
pub fn init_screen(efi_system_table: &EfiSystemTable) -> Result<()> {
    let mut vram = init_vram(efi_system_table)?;
    let (width, height) = (vram.width(), vram.height());
    fill_rect(&mut vram, 0x000000, 0, 0, width, height)?;
    draw_test_pattern(&mut vram);
    console::install_screen(vram);
    Ok(())
}

/// Starts the timer interrupt: remaps the PIC, drives the PIT at `hz`, enables
/// interrupts, and waits for the first tick.
///
/// Both halves matter. Without the PIT nothing ever wakes the executor, and
/// without `sti` the CPU never even takes the interrupt; either way the tasks
/// would stop after their first poll and the executor would spin in `hlt`.
pub fn init_interrupts(hz: u32) -> Result<()> {
    interrupts::remap_pic();
    interrupts::start_pit(hz);
    // SAFETY: `exceptions::init` has loaded an IDT, and the timer stub sends
    // EOI, so the interrupt is handled and execution resumes.
    x86::enable_interrupts();

    // Prove that a tick arrives, and fail here rather than hanging in `hlt` if
    // it never does. The HPET is a separate clock from the PIT, so it can bound
    // the wait without trusting the interrupt being waited for.
    let start = interrupts::TICKS.load(Ordering::Relaxed);
    let deadline = timer::nanoseconds() + 10 * u64::from(1_000_000_000u32 / hz);
    while interrupts::TICKS.load(Ordering::Relaxed) == start {
        if timer::nanoseconds() >= deadline {
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
    let base = timer::base_from_acpi(rsdp)?;
    info!("timer: HPET registers at {base:#018x}");
    // SAFETY: the ACPI table points at the HPET's memory-mapped registers.
    unsafe { timer::init(base) }
}

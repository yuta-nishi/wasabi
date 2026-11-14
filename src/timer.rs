//! The High Precision Event Timer, and a future that waits on it.
//!
//! The HPET is a free-running 64-bit counter. Its capability register reports
//! the counter period in femtoseconds (10 ns on QEMU), which is all we need to
//! turn ticks into time.
//!
//! The timer is registered once at boot and read through two atomics, so tasks
//! (and later interrupt handlers) can ask for the time without locking.

use crate::acpi::GenericAddress;
use crate::acpi::Rsdp;
use crate::acpi::TableHeader;
use crate::result::Result;
use core::future::Future;
use core::pin::Pin;
use core::ptr::read_volatile;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;
use core::task::Context;
use core::task::Poll;

/// Offset of the capability and ID register.
const CAPABILITY: usize = 0x000;
/// Offset of the general configuration register.
const CONFIGURATION: usize = 0x010;
/// Offset of the free-running main counter.
const MAIN_COUNTER: usize = 0x0f0;
/// Bit 0 of the configuration register enables the counter.
const ENABLE: u64 = 1 << 0;

/// `HPET` (ACPI 6.5, 5.2.25.15): where the timer registers live.
#[repr(C, packed)]
struct HpetDescription {
    _header: TableHeader,
    _event_timer_block_id: u32,
    base_address: GenericAddress,
    _hpet_number: u8,
    _minimum_tick: u16,
    _page_protection: u8,
}

/// The HPET register base the ACPI tables describe.
pub fn base_from_acpi(rsdp: &Rsdp) -> Result<u64> {
    let header = rsdp
        .xsdt()
        .find(b"HPET")
        .ok_or("no HPET table in the XSDT")?;
    // SAFETY: `find` only returns tables whose signature is `HPET`.
    let table = unsafe { &*(header as *const TableHeader as *const HpetDescription) };
    table.base_address.address_in_memory_space()
}

static BASE: AtomicU64 = AtomicU64::new(0);
static PERIOD_FEMTOSECONDS: AtomicU64 = AtomicU64::new(1);

/// Starts the HPET counter at `base` and remembers its tick length, so time can
/// be read from here on. Call this once, at boot.
///
/// # Safety
///
/// `base` must point at the HPET's memory-mapped registers.
pub unsafe fn start(base: u64) -> Result<()> {
    let registers = base as *const u8;
    let capability = unsafe { read_volatile(registers.add(CAPABILITY) as *const u64) };
    // The upper half of the capability register is COUNTER_CLK_PERIOD: how long
    // one tick lasts, in femtoseconds. The lower half only describes the device
    // (revision, comparators, vendor id), so it is not used here.
    let period = capability >> 32;
    if period == 0 {
        return Err("the HPET reports a zero period");
    }
    unsafe {
        let configuration = registers.add(CONFIGURATION) as *mut u64;
        let value = read_volatile(configuration);
        core::ptr::write_volatile(configuration, value | ENABLE);
    }
    BASE.store(base, Ordering::Release);
    PERIOD_FEMTOSECONDS.store(period, Ordering::Release);
    Ok(())
}

/// Ticks of the main counter since the HPET started.
pub fn ticks() -> u64 {
    let base = BASE.load(Ordering::Acquire);
    if base == 0 {
        return 0;
    }
    // SAFETY: `init` stored this address and never changes it again.
    unsafe { read_volatile((base as *const u8).add(MAIN_COUNTER) as *const u64) }
}

/// Nanoseconds since the HPET started.
pub fn nanoseconds() -> u64 {
    let period = PERIOD_FEMTOSECONDS.load(Ordering::Acquire);
    let femtoseconds = u128::from(ticks()) * u128::from(period);
    (femtoseconds / 1_000_000) as u64
}

/// A future that stays pending until its deadline, waking itself to be polled
/// again. Nothing here is asynchronous yet: the executor spins until the time
/// has passed, which is the limitation the next commit in the book fixes.
pub struct Sleep {
    deadline_nanoseconds: u64,
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if nanoseconds() >= self.deadline_nanoseconds {
            Poll::Ready(())
        } else {
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Suspends the calling task for at least `microseconds`.
pub fn sleep(microseconds: u64) -> Sleep {
    Sleep {
        deadline_nanoseconds: nanoseconds() + microseconds * 1_000,
    }
}

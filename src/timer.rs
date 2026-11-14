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

/// How long one tick lasts, as the HPET reports it. The counter is free-running
/// and started by the firmware, so nothing here defines its zero point.
///
/// # Safety
///
/// `base` must point at the HPET's memory-mapped registers.
unsafe fn period_femtoseconds(base: u64) -> u64 {
    // The upper half of the capability register is COUNTER_CLK_PERIOD; the lower
    // half only describes the device (revision, comparators, vendor id).
    let capability =
        unsafe { read_volatile((base as *const u8).add(CAPABILITY) as *const u64) };
    capability >> 32
}

/// Checks that the HPET is usable at `base`, makes sure its counter is running,
/// and remembers where it is. Call this once, at boot.
///
/// # Safety
///
/// `base` must point at the HPET's memory-mapped registers.
pub unsafe fn init(base: u64) -> Result<()> {
    if unsafe { period_femtoseconds(base) } == 0 {
        return Err("the HPET reports a zero period");
    }
    unsafe {
        let configuration = (base as *const u8).add(CONFIGURATION) as *mut u64;
        let value = read_volatile(configuration);
        core::ptr::write_volatile(configuration, value | ENABLE);
    }
    BASE.store(base, Ordering::Release);
    Ok(())
}

/// The HPET register base. Panics if `init` has not run yet: using the timer
/// before that is a boot-order bug, not a runtime condition to paper over.
fn base() -> u64 {
    let base = BASE.load(Ordering::Acquire);
    assert!(base != 0, "timer::init has not run yet");
    base
}

/// Ticks of the free-running main counter.
pub fn ticks() -> u64 {
    let base = base();
    // SAFETY: `init` stored this address and never changes it again.
    unsafe { read_volatile((base as *const u8).add(MAIN_COUNTER) as *const u64) }
}

/// Nanoseconds, counted from whenever the HPET's counter started.
pub fn nanoseconds() -> u64 {
    let base = base();
    // SAFETY: `init` stored this address and never changes it again.
    let period = unsafe { period_femtoseconds(base) };
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

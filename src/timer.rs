use crate::acpi::GenericAddress;
use crate::acpi::Rsdp;
use crate::acpi::TableHeader;
use crate::bitfield;
use crate::mutex::Mutex;
use crate::result::Result;
use core::future::Future;
use core::pin::Pin;
use core::ptr::read_volatile;
use core::task::Context;
use core::task::Poll;
use core::time::Duration;

/// The HPET's memory-mapped registers (IA-PC HPET specification 1.0, 2.3).
#[repr(C)]
struct HpetRegisters {
    capability_and_id: u64,
    _reserved008: u64,
    configuration: u64,
    _reserved018: [u64; 0x1b],
    main_counter: u64,
}
const _: () = assert!(core::mem::offset_of!(HpetRegisters, capability_and_id) == 0x000);
const _: () = assert!(core::mem::offset_of!(HpetRegisters, configuration) == 0x010);
const _: () = assert!(core::mem::offset_of!(HpetRegisters, main_counter) == 0x0f0);

bitfield! {
    /// `GENERAL_CAPABILITIES_ID`: how fast the counter runs.
    CapabilityAndId: u64 {
        counter_clock_period: 63, 32,
    }
}

bitfield! {
    /// `GENERAL_CONFIGURATION`: whether the counter runs.
    Configuration: u64 {
        enable: 0, 0 -> set_enable,
    }
}

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

static HPET: Mutex<Option<Hpet>> = Mutex::new(None);

/// The HPET registers, once [`init`] has found them.
struct Hpet {
    base: u64,
}

impl Hpet {
    /// How long one tick lasts, as the HPET reports it. The counter itself is
    /// free-running and started by the firmware, so nothing here defines its
    /// zero point.
    fn period_femtoseconds(&self) -> u64 {
        // The upper half of the capability register is COUNTER_CLK_PERIOD; the
        // lower half only describes the device (revision, comparators, vendor).
        let capability = unsafe {
            read_volatile(
                (self.base as *const u8)
                    .add(core::mem::offset_of!(HpetRegisters, capability_and_id))
                    as *const u64,
            )
        };
        CapabilityAndId::new(capability).counter_clock_period()
    }

    /// Ticks of the free-running main counter.
    fn ticks(&self) -> u64 {
        // SAFETY: `self.base` points at the HPET's memory-mapped registers.
        unsafe {
            read_volatile(
                (self.base as *const u8)
                    .add(core::mem::offset_of!(HpetRegisters, main_counter))
                    as *const u64,
            )
        }
    }

    /// Nanoseconds, counted from whenever the counter started.
    fn nanoseconds(&self) -> u64 {
        let femtoseconds =
            u128::from(self.ticks()) * u128::from(self.period_femtoseconds());
        (femtoseconds / 1_000_000) as u64
    }
}

/// Checks that the HPET is usable at `base`, makes sure its counter is running,
/// and remembers where it is. Call this once, at boot.
///
/// # Safety
///
/// `base` must point at the HPET's memory-mapped registers.
pub unsafe fn init(base: u64) -> Result<()> {
    let hpet = Hpet { base };
    if hpet.period_femtoseconds() == 0 {
        return Err("the HPET reports a zero period");
    }
    unsafe {
        let configuration = (base as *const u8)
            .add(core::mem::offset_of!(HpetRegisters, configuration))
            as *mut u64;
        let mut value = Configuration::new(read_volatile(configuration));
        value.set_enable(1);
        core::ptr::write_volatile(configuration, value.raw());
    }
    *HPET.lock() = Some(hpet);
    Ok(())
}

/// Nanoseconds, counted from whenever the HPET's counter started.
pub fn nanoseconds() -> u64 {
    let guard = HPET.lock();
    let hpet = guard.as_ref().expect("timer::init has not run yet");
    hpet.nanoseconds()
}

/// A future that stays pending until its deadline. It does not wake itself: the
/// timer interrupt marks every task woken once a millisecond, so the executor
/// can sleep in `hlt` in between.
pub struct Sleep {
    deadline_nanoseconds: u64,
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if nanoseconds() >= self.deadline_nanoseconds {
            Poll::Ready(())
        } else {
            let _ = context;
            Poll::Pending
        }
    }
}

/// Suspends the calling task for at least `duration`.
pub fn sleep(duration: Duration) -> Sleep {
    let delay = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    Sleep {
        deadline_nanoseconds: nanoseconds().saturating_add(delay),
    }
}

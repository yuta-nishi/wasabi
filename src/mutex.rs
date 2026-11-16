//! A spin lock, the simplest way to share data between tasks and interrupts.
//!
//! There is no scheduler involvement: a second locker simply spins until the
//! first one drops its guard. That is fine while critical sections are short,
//! and it is the only option before the OS can block a task.

use crate::x86::disable_interrupts;
use crate::x86::restore_interrupts;
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::Deref;
use core::ops::DerefMut;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;

/// Gives exclusive access to `T` to whoever holds the lock.
pub struct Mutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is only reachable through a guard, and a guard is only
// handed out after winning the atomic swap. `T: Send` because the value moves
// between the lockers.
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Waits until the lock is free and returns a guard that releases it on drop.
    ///
    /// Interrupts are disabled while the guard lives: a timer handler that took
    /// this lock while the interrupted task holds it could never release it, so
    /// the task would spin forever.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        let interrupts = disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }
        MutexGuard {
            mutex: self,
            interrupts,
        }
    }
}

/// Proof that the mutex is held, and the only way to reach the value inside it.
///
/// The guard borrows the mutex for `'a` instead of owning the value, which is
/// what makes the two guarantees fall out of the type system:
///
/// - the value cannot be reached without a guard, and a guard only comes from
///   [`Mutex::lock`];
/// - a reference borrowed out of the guard cannot outlive it, so it cannot
///   outlive the lock either.
///
/// Dropping the guard releases the lock and re-enables interrupts if they were
/// enabled when it was taken.
pub struct MutexGuard<'a, T> {
    /// The mutex being held: `Drop` releases it and `Deref` reaches the value
    /// through it. This reference is what ties the guard's life to the lock.
    mutex: &'a Mutex<T>,
    /// The interrupt flag to put back when the guard goes away.
    interrupts: u64,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard proves this mutex is held, so nothing else can
        // reach the value mutably while it lives.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as above; the exclusive borrow of the guard is enough because
        // no other guard for this mutex can exist.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        // SAFETY: `interrupts` came from `disable_interrupts` in `lock`.
        unsafe { restore_interrupts(self.interrupts) };
    }
}

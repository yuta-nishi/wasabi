//! Memory-mapped device registers.
//!
//! A register is not memory: it changes when the device decides to, and reading
//! it can have side effects. Reaching it only through [`Mmio::read`] and
//! [`Mmio::write`], which are volatile, keeps the compiler from caching a value
//! in a CPU register or from removing a write it thinks is redundant.

use core::marker::PhantomData;
use core::ptr::read_volatile;
use core::ptr::write_volatile;

/// A pointer to device registers shaped like `T`.
pub struct Mmio<T: Copy> {
    address: usize,
    _value: PhantomData<T>,
}

impl<T: Copy> Mmio<T> {
    /// # Safety
    ///
    /// `address` must point at `T`-shaped registers of a device, and the CPU
    /// must not cache them: a cached read answers with the old value, which is
    /// why device windows are mapped with the cache-disable bit.
    pub const unsafe fn at(address: u64) -> Self {
        Self {
            address: address as usize,
            _value: PhantomData,
        }
    }

    /// The register's current value, asked from the device every time.
    pub fn read(&self) -> T {
        // SAFETY: `at` promised a register of this shape.
        unsafe { read_volatile(self.address as *const T) }
    }

    /// Stores a value in the register.
    pub fn write(&self, value: T) {
        // SAFETY: as in `read`.
        unsafe { write_volatile(self.address as *mut T, value) }
    }
}

impl<T: Copy> Clone for Mmio<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Copy> Copy for Mmio<T> {}

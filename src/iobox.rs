//! Memory a device reads and writes by address.
//!
//! The driver fills a structure in memory and tells the controller where it is;
//! the controller then reads those bytes and writes its answers back. The memory
//! must therefore stay put and must not be cached: a copy left in a cache line
//! would be a different view of the same data than the one the device sees.

use crate::paging::PAGE_SIZE;
use crate::paging::PageTables;
use alloc::boxed::Box;
use core::marker::PhantomPinned;
use core::mem::MaybeUninit;
use core::pin::Pin;

/// Types whose all-zero bit pattern is a valid value.
///
/// # Safety
///
/// `T` must accept every bit zero: no non-zero discriminant, no dangling
/// reference, no `bool` that could be neither 0 nor 1.
pub unsafe trait Zeroable: Sized {}

// SAFETY: integers have no invalid bit pattern.
unsafe impl Zeroable for u64 {}
// SAFETY: an array is zeroable when its element is, since every element is.
unsafe impl<T: Zeroable, const N: usize> Zeroable for [T; N] {}

/// A page-aligned structure whose address the device can be given.
///
/// `T` must be valid when it is all zeros, since the memory starts out zeroed.
pub struct IoBox<T: Sized> {
    inner: Pin<Box<IoBoxInner<T>>>,
}

#[repr(align(4096))]
struct IoBoxInner<T: Sized> {
    data: T,
    _pinned: PhantomPinned,
}

impl<T: Zeroable> IoBox<T> {
    /// Allocates zeroed, page-aligned memory for `T` and maps it uncached.
    pub fn new(tables: &mut PageTables) -> Self {
        let inner = Box::pin(IoBoxInner {
            // SAFETY: `Zeroable` promises that all zeros is a valid `T`.
            data: unsafe { MaybeUninit::<T>::zeroed().assume_init() },
            _pinned: PhantomPinned,
        });
        let this = Self { inner };
        tables.map_range_uncached(
            this.address(),
            core::mem::size_of::<T>().max(PAGE_SIZE as usize) as u64,
        );
        this
    }

    /// Where the structure lives. The identity map makes this the physical
    /// address the controller needs.
    pub fn address(&self) -> u64 {
        &*self.inner as *const IoBoxInner<T> as u64
    }

    /// The structure itself, for the driver to read after the device wrote it.
    ///
    /// # Safety
    ///
    /// The device may be writing this memory at any time; only call this when
    /// its state says it is done.
    pub unsafe fn as_mut(&mut self) -> &mut T {
        // SAFETY: the caller keeps the device away while the reference lives.
        unsafe { &mut self.inner.as_mut().get_unchecked_mut().data }
    }
}

impl<T: Zeroable + Copy, const N: usize> IoBox<[T; N]> {
    /// One element, read through a volatile load.
    pub fn read_slot(&self, index: usize) -> Option<T> {
        if index >= N {
            return None;
        }
        // SAFETY: `index` was bounds checked against the array length.
        Some(unsafe { core::ptr::read_volatile((self.address() as *const T).add(index)) })
    }

    /// One element, written through a volatile store.
    pub fn write_slot(&mut self, index: usize, value: T) -> Option<()> {
        if index >= N {
            return None;
        }
        // SAFETY: as in `read_slot`.
        unsafe {
            core::ptr::write_volatile((self.address() as *mut T).add(index), value)
        };
        Some(())
    }
}

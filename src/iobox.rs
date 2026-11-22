//! Memory a device reads and writes by address.
//!
//! The driver fills a structure in memory and tells the controller where it is;
//! the controller then reads those bytes and writes its answers back. The memory
//! must therefore stay put and must not be cached: a copy left in a cache line
//! would be a different view of the same data than the one the device sees.

use crate::paging::PAGE_SIZE;
use crate::paging::PageTables;
use crate::result::Result;
use alloc::alloc::Layout;
use alloc::boxed::Box;
use core::marker::PhantomPinned;
use core::mem::MaybeUninit;
use core::pin::Pin;

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

impl<T: Sized> IoBox<T> {
    /// Allocates zeroed, page-aligned memory for `T` and maps it uncached.
    ///
    /// # Safety
    ///
    /// All zeros must be a valid `T`: the controller reads this memory before
    /// the driver has written all of it.
    pub unsafe fn new(tables: &mut PageTables) -> Self {
        let inner = Box::pin(IoBoxInner {
            // SAFETY: the caller promises that all zeros is a valid `T`.
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

/// Page-aligned, zeroed, uncached memory of `size` bytes, returned as an address
/// for structures whose size is only known at run time.
pub fn allocate_uncached(tables: &mut PageTables, size: usize) -> Result<u64> {
    let page = PAGE_SIZE as usize;
    let size = size.max(page).div_ceil(page) * page;
    let layout = Layout::from_size_align(size, PAGE_SIZE as usize)
        .map_err(|_| "bad allocation layout")?;
    // SAFETY: the layout has a non-zero size, which `alloc_zeroed` requires.
    let pointer = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if pointer.is_null() {
        return Err("out of memory for a device structure");
    }
    let address = pointer as u64;
    tables.map_range_uncached(address, size as u64);
    Ok(address)
}

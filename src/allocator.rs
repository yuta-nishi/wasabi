//! A byte-unit heap allocator.
//!
//! Free space is kept as a singly-linked list of `Header`s, where each header
//! starts its own free region: the region runs from right after the header to
//! [`Header::end_addr`]. Allocation walks the list and splits the first region
//! that can hold the request (first fit), freeing only flips the region's
//! `is_allocated` flag.

extern crate alloc;

use crate::result::Result;
use alloc::alloc::GlobalAlloc;
use alloc::alloc::Layout;
use alloc::boxed::Box;
use core::borrow::BorrowMut;
use core::cell::RefCell;
use core::cmp::max;
use core::fmt;
use core::mem::size_of;
use core::ops::DerefMut;
use core::ptr::null_mut;

/// Rounds `v` up to the next power of two (already powers stay put).
pub fn round_up_to_nearest_pow2(v: usize) -> Result<usize> {
    1usize
        .checked_shl(usize::BITS - v.wrapping_sub(1).leading_zeros())
        .ok_or("Out of range")
}

/// Vertical bar `|` represents the chunk that has a Header
/// before: |-- prev -------|---- self ---------------
/// align:  |--------|-------|-------|-------|-------|
/// after:  |---------------||-------|----------------
pub struct Header {
    next_header: Option<Box<Header>>,
    size: usize,
    is_allocated: bool,
    _reserved: usize,
}
const HEADER_SIZE: usize = size_of::<Header>();
#[allow(clippy::assertions_on_constants)]
const _: () = assert!(HEADER_SIZE == 32);
// Size of Header should be power of 2
const _: () = assert!(HEADER_SIZE.count_ones() == 1);
pub const LAYOUT_PAGE_4K: Layout =
    unsafe { Layout::from_size_align_unchecked(4096, 4096) };

impl Header {
    fn can_provide(&self, size: usize, align: usize) -> bool {
        // This check is rough - actual size needed may be smaller.
        // HEADER_SIZE * 2 => one for allocated region, another for padding.
        self.size >= size + HEADER_SIZE * 2 + align
    }
    fn is_allocated(&self) -> bool {
        self.is_allocated
    }
    fn end_addr(&self) -> usize {
        self as *const Header as usize + self.size
    }
    /// # Safety
    ///
    /// `addr` must point to a writable, `HEADER_SIZE`-aligned region that no
    /// other `Header` (or live allocation) covers.
    unsafe fn new_from_addr(addr: usize) -> Box<Header> {
        let header = addr as *mut Header;
        // SAFETY: the caller guarantees `addr` is writable and unaliased.
        unsafe {
            header.write(Header {
                next_header: None,
                size: 0,
                is_allocated: false,
                _reserved: 0,
            });
        }
        // SAFETY: `header` was just written above, so it is a valid allocation.
        unsafe { Box::from_raw(header) }
    }
    /// Splits a free region to serve `size` bytes aligned to `align`, returning
    /// the address handed to the caller or `None` if this region is too small.
    //
    // Note: std::alloc::Layout doc says:
    // > All layouts have an associated size and a power-of-two alignment.
    fn provide(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let size = max(round_up_to_nearest_pow2(size).ok()?, HEADER_SIZE);
        let align = max(align, HEADER_SIZE);
        if self.is_allocated() || !self.can_provide(size, align) {
            None
        } else {
            // Each char represents 32-byte chunks.
            //
            // |-----|----------------- self ---------|----------
            // |-----|----------------------          |----------
            //                                        ^ self.end_addr()
            //                              |-------|-
            //                              ^ header_for_allocated
            //                               ^ allocated_addr
            //                                      ^ header_for_padding
            //                                      ^
            // header_for_allocated.end_addr() self has enough space
            // to allocate the requested object.

            // Make a Header for the allocated object
            let mut size_used = 0;
            let allocated_addr = (self.end_addr() - size) & !(align - 1);
            let mut header_for_allocated =
                unsafe { Self::new_from_addr(allocated_addr - HEADER_SIZE) };
            header_for_allocated.is_allocated = true;
            header_for_allocated.size = size + HEADER_SIZE;
            size_used += header_for_allocated.size;
            header_for_allocated.next_header = self.next_header.take();
            if header_for_allocated.end_addr() != self.end_addr() {
                // Make a Header for padding
                let mut header_for_padding =
                    unsafe { Self::new_from_addr(header_for_allocated.end_addr()) };
                header_for_padding.is_allocated = false;
                header_for_padding.size =
                    self.end_addr() - header_for_allocated.end_addr();
                size_used += header_for_padding.size;
                header_for_padding.next_header = header_for_allocated.next_header.take();
                header_for_allocated.next_header = Some(header_for_padding);
            }
            // Shrink self
            assert!(self.size >= size_used + HEADER_SIZE);
            self.size -= size_used;
            self.next_header = Some(header_for_allocated);
            Some(allocated_addr as *mut u8)
        }
    }
}
impl Drop for Header {
    fn drop(&mut self) {
        panic!("Header should not be dropped!");
    }
}
impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Header @ {:#018X} {{ size: {:#018X}, is_allocated: {} }}",
            self as *const Header as usize,
            self.size,
            self.is_allocated()
        )
    }
}

pub struct FirstFitAllocator {
    first_header: RefCell<Option<Box<Header>>>,
}

#[global_allocator]
pub static ALLOCATOR: FirstFitAllocator = FirstFitAllocator {
    first_header: RefCell::new(None),
};

// SAFETY: the allocator mutates its free list only through the `RefCell`, and
// every byte it hands out comes from regions carved out of the list, so sharing
// it across threads cannot alias live allocations.
unsafe impl Sync for FirstFitAllocator {}

unsafe impl GlobalAlloc for FirstFitAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc_with_options(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: `ptr` came from `alloc` above, so a live header sits right
        // before it. Only that header's flag changes: the region itself is raw
        // heap memory and stays in place as free space.
        unsafe {
            let header = ptr.sub(HEADER_SIZE) as *mut Header;
            (*header).is_allocated = false;
        }
    }
}

impl FirstFitAllocator {
    pub fn alloc_with_options(&self, layout: Layout) -> *mut u8 {
        let mut header = self.first_header.borrow_mut();
        let mut header = header.deref_mut();
        loop {
            match header {
                Some(e) => match e.provide(layout.size(), layout.align()) {
                    Some(p) => break p,
                    None => {
                        header = e.next_header.borrow_mut();
                        continue;
                    }
                },
                None => {
                    break null_mut::<u8>();
                }
            }
        }
    }
}

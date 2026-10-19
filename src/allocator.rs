//! A byte-unit heap allocator.
//!
//! Every region of the heap is described by a `Header` that lives inside it,
//! and the headers are chained into a singly-linked list (`is_allocated`
//! separates free regions from handed-out ones). Allocation walks the list and
//! splits the first region that can hold the request (first fit); freeing only
//! clears that region's `is_allocated` flag again.

extern crate alloc;

use crate::result::Result;
use crate::uefi::EfiMemoryDescriptor;
use crate::uefi::EfiMemoryType;
use crate::uefi::MemoryMapHolder;
use alloc::alloc::GlobalAlloc;
use alloc::alloc::Layout;
use alloc::boxed::Box;
use core::cell::RefCell;
use core::cmp::max;
use core::fmt;
use core::mem::size_of;
use core::ptr::null_mut;

/// Rounds `v` up to a power of two: exact powers stay put, while `0` and
/// values too large to represent are rejected.
pub fn round_up_to_nearest_pow2(v: usize) -> Result<usize> {
    // `v - 1` keeps exact powers of two unchanged; `checked_shl` then rejects
    // both `v == 0` and shift counts that do not fit in a `usize`.
    let exponent = usize::BITS - v.wrapping_sub(1).leading_zeros();
    1usize.checked_shl(exponent).ok_or("Out of range")
}

/// A free-list node describing one region of the heap.
///
/// A header sits at the start of its own region, and `size` counts that whole
/// region including the header, so `end_addr` gives the region's end. Headers
/// live in the heap memory itself (see `new_from_addr`) and are never
/// allocated, hence dropping one as a Rust value panics.
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
    fn provide(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        // Round up to a power of two, keep at least one header.
        let payload_size = max(round_up_to_nearest_pow2(size).ok()?, HEADER_SIZE);
        let align = max(align, HEADER_SIZE);
        if self.is_allocated() || !self.can_provide(payload_size, align) {
            return None;
        }

        // Split at the end so `self` keeps its address.
        //
        // before:
        //   self                                                                         end_addr
        //   +----------------------------------------------------------------------------+
        //   |                          free (the whole region)                           |
        //   +----------------------------------------------------------------------------+
        //
        // after:
        //   self          allocated_addr   allocated_addr  allocated_addr + payload_size end_addr
        //                 - HEADER_SIZE
        //   |             |                |               |                             |
        //   +-------------+----------------+---------------+-----------------------------+
        //   | free (self) |      header    |    request    |           padding           |
        //   +-------------+----------------+---------------+-----------------------------+
        let end_addr = self.end_addr();
        let allocated_addr = (end_addr - payload_size) & !(align - 1);
        let padding_addr = allocated_addr + payload_size;
        let padding_size = end_addr - padding_addr;
        let consumed_size = HEADER_SIZE + payload_size + padding_size;

        let old_next_header = self.next_header.take();

        let mut allocated_header =
            unsafe { Self::new_from_addr(allocated_addr - HEADER_SIZE) };
        allocated_header.is_allocated = true;
        allocated_header.size = payload_size + HEADER_SIZE;
        allocated_header.next_header = if padding_size == 0 {
            old_next_header
        } else {
            let mut padding_header = unsafe { Self::new_from_addr(padding_addr) };
            padding_header.is_allocated = false;
            padding_header.size = padding_size;
            padding_header.next_header = old_next_header;
            Some(padding_header)
        };

        // `self` must still hold its own header.
        assert!(self.size >= consumed_size + HEADER_SIZE);
        self.size -= consumed_size;
        self.next_header = Some(allocated_header);

        Some(allocated_addr as *mut u8)
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
        let mut head = self.first_header.borrow_mut();
        let mut cursor = head.as_mut();
        while let Some(node) = cursor {
            if let Some(addr) = node.provide(layout.size(), layout.align()) {
                return addr;
            }
            cursor = node.next_header.as_mut();
        }
        null_mut()
    }

    /// Adds every conventional-memory region of `memory_map` to the free list.
    pub fn init_with_mmap(&self, memory_map: &MemoryMapHolder) {
        for e in memory_map.iter() {
            if e.memory_type() != EfiMemoryType::CONVENTIONAL_MEMORY {
                continue;
            }
            self.add_free_from_descriptor(e);
        }
    }

    fn add_free_from_descriptor(&self, desc: &EfiMemoryDescriptor) {
        let mut start_addr = desc.physical_start() as usize;
        let mut size = desc.number_of_pages() as usize * 4096;
        // Never hand out address 0: `alloc` reports failure as null.
        if start_addr == 0 {
            start_addr += 4096;
            size = size.saturating_sub(4096);
        }
        if size <= 4096 {
            return;
        }
        // SAFETY: the descriptor covers `size` bytes of conventional memory at
        // `start_addr`, which the firmware handed over to us, so a header can be
        // written at its start.
        let mut header = unsafe { Header::new_from_addr(start_addr) };
        header.size = size;

        // Regions from the memory map are not contiguous, so they cannot be
        // merged: the order does not matter and new regions go to the front.
        let mut first_header = self.first_header.borrow_mut();
        header.next_header = first_header.take();
        *first_header = Some(header);
    }
}

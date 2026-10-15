//! Tests for the byte-unit heap allocator. They run inside the OS on QEMU, with
//! the heap built from the firmware's memory map (see `tests/runner.rs`).

extern crate alloc;

use alloc::alloc::GlobalAlloc;
use alloc::alloc::Layout;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::null_mut;
use wasabi::allocator::ALLOCATOR;
use wasabi::allocator::round_up_to_nearest_pow2;

pub fn round_up_to_nearest_pow2_cases() {
    assert_eq!(round_up_to_nearest_pow2(0), Err("Out of range"));
    assert_eq!(round_up_to_nearest_pow2(1), Ok(1));
    assert_eq!(round_up_to_nearest_pow2(2), Ok(2));
    assert_eq!(round_up_to_nearest_pow2(3), Ok(4));
    assert_eq!(round_up_to_nearest_pow2(4), Ok(4));
    assert_eq!(round_up_to_nearest_pow2(5), Ok(8));
    assert_eq!(round_up_to_nearest_pow2(6), Ok(8));
    assert_eq!(round_up_to_nearest_pow2(7), Ok(8));
    assert_eq!(round_up_to_nearest_pow2(8), Ok(8));
    assert_eq!(round_up_to_nearest_pow2(9), Ok(16));
}

/// Repeatedly grows and drops `Vec`s, so both the `alloc` and the `dealloc`
/// hooks see plenty of traffic.
pub fn vec_grow_and_drop() {
    for i in 0..1000 {
        let mut v = Vec::new();
        v.resize(i, 10u8);
        assert_eq!(v.len(), i);
        assert_eq!(v.iter().map(|&b| b as usize).sum::<usize>(), i * 10);
    }
}

pub fn allocations_are_aligned() {
    for align in [1, 2, 4, 8, 16, 32, 4096] {
        let mut pointers = [null_mut::<u8>(); 100];
        for p in pointers.iter_mut() {
            *p = ALLOCATOR.alloc_with_options(
                Layout::from_size_align(1234, align).expect("bad layout"),
            );
            assert!(!p.is_null());
            assert_eq!(*p as usize % align, 0);
        }
    }
}

/// Writes a distinct pattern into each allocation, frees every second one,
/// checks the survivors, re-allocates the freed ones and checks everything
/// again: any overlap between regions shows up as a corrupted pattern.
pub fn allocated_objects_have_no_overlap() {
    let allocations = [
        Layout::from_size_align(128, 128).unwrap(),
        Layout::from_size_align(32, 32).unwrap(),
        Layout::from_size_align(8, 8).unwrap(),
        Layout::from_size_align(16, 16).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(4, 4).unwrap(),
        Layout::from_size_align(2, 2).unwrap(),
        Layout::from_size_align(600000, 64).unwrap(),
        Layout::from_size_align(64, 64).unwrap(),
        Layout::from_size_align(1, 1).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(3, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(600000, 64).unwrap(),
        Layout::from_size_align(6000, 64).unwrap(),
        Layout::from_size_align(60000, 64).unwrap(),
        Layout::from_size_align(60000, 64).unwrap(),
        Layout::from_size_align(60000, 64).unwrap(),
        Layout::from_size_align(60000, 64).unwrap(),
    ];
    let mut pointers = vec![null_mut::<u8>(); allocations.len()];
    for (i, (layout, pointer)) in allocations.iter().zip(pointers.iter_mut()).enumerate()
    {
        *pointer = ALLOCATOR.alloc_with_options(*layout);
        assert!(!pointer.is_null());
        for k in 0..layout.size() {
            // SAFETY: `pointer` covers `layout.size()` bytes.
            unsafe { *pointer.add(k) = i as u8 }
        }
    }
    let check = |pointers: &[*mut u8]| {
        for (i, (layout, pointer)) in allocations.iter().zip(pointers.iter()).enumerate()
        {
            for k in 0..layout.size() {
                // SAFETY: as above; the pattern must still be intact.
                assert_eq!(unsafe { *pointer.add(k) }, i as u8);
            }
        }
    };
    check(&pointers);
    for (i, (layout, pointer)) in allocations.iter().zip(pointers.iter_mut()).enumerate()
    {
        if i % 2 == 0 {
            // SAFETY: `pointer` was returned by `alloc_with_options` above.
            unsafe { ALLOCATOR.dealloc(*pointer, *layout) }
        }
    }
    for (i, (_, pointer)) in allocations.iter().zip(pointers.iter()).enumerate() {
        if i % 2 == 1 {
            // The odd-numbered neighbours must be untouched by the frees.
            for k in 0..allocations[i].size() {
                // SAFETY: as above.
                assert_eq!(unsafe { *pointer.add(k) }, i as u8);
            }
        }
    }
    for (i, (layout, pointer)) in allocations.iter().zip(pointers.iter_mut()).enumerate()
    {
        if i % 2 == 0 {
            *pointer = ALLOCATOR.alloc_with_options(*layout);
            assert!(!pointer.is_null());
            for k in 0..layout.size() {
                // SAFETY: `pointer` covers `layout.size()` bytes.
                unsafe { *pointer.add(k) = i as u8 }
            }
        }
    }
    check(&pointers);
}

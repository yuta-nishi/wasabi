//! `GetMemoryMap`: the descriptors and the buffer that holds them.

use super::ffi::EfiMemoryType;

/// One entry of the memory map that `GetMemoryMap` returns.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EfiMemoryDescriptor {
    memory_type: EfiMemoryType,
    physical_start: u64,
    virtual_start: u64,
    number_of_pages: u64,
    attribute: u64,
}
impl EfiMemoryDescriptor {
    pub fn memory_type(&self) -> EfiMemoryType {
        self.memory_type
    }
    pub fn physical_start(&self) -> u64 {
        self.physical_start
    }
    pub fn number_of_pages(&self) -> u64 {
        self.number_of_pages
    }
}

const MEMORY_MAP_BUFFER_SIZE: usize = 0x8000;

/// The caller-provided buffer plus the metadata `GetMemoryMap` fills in.
pub struct MemoryMapHolder {
    pub(super) memory_map_buffer: [u8; MEMORY_MAP_BUFFER_SIZE],
    pub(super) memory_map_size: usize,
    pub(super) map_key: usize,
    pub(super) descriptor_size: usize,
    pub(super) descriptor_version: u32,
}

impl MemoryMapHolder {
    pub const fn new() -> MemoryMapHolder {
        MemoryMapHolder {
            memory_map_buffer: [0; MEMORY_MAP_BUFFER_SIZE],
            memory_map_size: MEMORY_MAP_BUFFER_SIZE,
            map_key: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }
    pub fn iter(&self) -> MemoryMapIterator<'_> {
        MemoryMapIterator { map: self, ofs: 0 }
    }
}
impl Default for MemoryMapHolder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MemoryMapIterator<'a> {
    map: &'a MemoryMapHolder,
    ofs: usize,
}
impl<'a> Iterator for MemoryMapIterator<'a> {
    type Item = &'a EfiMemoryDescriptor;
    fn next(&mut self) -> Option<&'a EfiMemoryDescriptor> {
        if self.ofs >= self.map.memory_map_size {
            None
        } else {
            // SAFETY: `ofs` stays inside the buffer because it is advanced by
            // `descriptor_size` until it reaches `memory_map_size`.
            let e: &EfiMemoryDescriptor = unsafe {
                &*(self.map.memory_map_buffer.as_ptr().add(self.ofs)
                    as *const EfiMemoryDescriptor)
            };
            self.ofs += self.map.descriptor_size;
            Some(e)
        }
    }
}

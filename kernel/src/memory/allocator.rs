use crate::logging;
use linked_list_allocator::LockedHeap;
use uefi::prelude::*;
use uefi::table::boot::{AllocateType, MemoryType};

use super::paging::PAGE_SIZE;
use super::physical::PhysicalMemoryManager;

pub const HEAP_SIZE_BYTES: usize = 64 * 1024 * 1024;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();

pub struct HeapInfo {
    size_bytes: usize,
    base: u64,
}

impl HeapInfo {
    pub fn log(&self) {
        logging::print(format_args!("Heap Size: {} MB\n", self.size_bytes / 1024 / 1024));
        logging::print(format_args!("Heap Base: 0x{:x}\n", self.base));
        logging::print(format_args!("Heap End: 0x{:x}\n", self.base + self.size_bytes as u64));
    }
}

pub fn initialize(
    system_table: &SystemTable<Boot>,
    physical: &mut PhysicalMemoryManager,
) -> HeapInfo {
    let boot_services = system_table.boot_services();
    let heap_pages = HEAP_SIZE_BYTES.div_ceil(PAGE_SIZE as usize);

    let heap_address = boot_services
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, heap_pages)
        .expect("failed to allocate kernel heap pages");

    physical.reserve_range(heap_address / PAGE_SIZE, heap_pages);

    unsafe {
        // SAFETY: The reserved heap range is a valid, contiguous LOADER_DATA allocation.
        HEAP_ALLOCATOR.lock().init(heap_address as *mut u8, heap_pages * PAGE_SIZE as usize);
    }

    HeapInfo { size_bytes: heap_pages * PAGE_SIZE as usize, base: heap_address }
}

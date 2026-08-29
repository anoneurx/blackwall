use crate::logging;
use core::ptr::NonNull;
use uefi::prelude::*;
use uefi::table::boot::{AllocateType, MemoryDescriptor, MemoryType};

use super::paging::PAGE_SIZE;

#[derive(Clone, Copy)]
pub struct PhysFrame {
    number: u64,
}

impl PhysFrame {
    pub fn address(self) -> u64 {
        self.number * PAGE_SIZE
    }
}

pub struct PhysicalMemoryManager {
    bitmap: NonNull<u64>,
    bitmap_words: usize,
    total_frames: usize,
    free_frames: usize,
    available_ram_mb: u64,
    reserved_regions: usize,
}

impl PhysicalMemoryManager {
    pub fn initialize(system_table: &SystemTable<Boot>) -> Self {
        let boot_services = system_table.boot_services();
        let memory_map = boot_services
            .memory_map(MemoryType::LOADER_DATA)
            .expect("UEFI memory map retrieval failed");

        let mut highest_address = 0u64;
        let mut available_pages = 0usize;
        let mut reserved_regions = 0usize;

        for descriptor in memory_map.entries() {
            highest_address = highest_address.max(region_end(descriptor));

            if descriptor.ty == MemoryType::CONVENTIONAL {
                available_pages = available_pages.saturating_add(descriptor.page_count as usize);
            } else {
                reserved_regions = reserved_regions.saturating_add(1);
            }
        }

        let total_frames = highest_address
            .checked_add(PAGE_SIZE - 1)
            .unwrap_or(highest_address)
            .div_ceil(PAGE_SIZE) as usize;
        let bitmap_bytes = total_frames.div_ceil(8);
        let bitmap_pages = bitmap_bytes.div_ceil(PAGE_SIZE as usize).max(1);

        let bitmap_address = boot_services
            .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, bitmap_pages)
            .expect("failed to allocate PMM bitmap pages");

        let bitmap_ptr =
            NonNull::new(bitmap_address as *mut u64).expect("UEFI returned a null bitmap pointer");
        unsafe {
            // SAFETY: The bitmap lives in freshly allocated LOADER_DATA pages.
            core::ptr::write_bytes(
                bitmap_ptr.as_ptr() as *mut u8,
                0xff,
                bitmap_pages * PAGE_SIZE as usize,
            );
        }

        let mut manager = Self {
            bitmap: bitmap_ptr,
            bitmap_words: (bitmap_pages * PAGE_SIZE as usize) / 8,
            total_frames,
            free_frames: 0,
            available_ram_mb: (available_pages as u64 * PAGE_SIZE) / 1024 / 1024,
            reserved_regions,
        };

        for descriptor in memory_map.entries() {
            if descriptor.ty == MemoryType::CONVENTIONAL {
                manager.release_range(
                    descriptor.phys_start / PAGE_SIZE,
                    descriptor.page_count as usize,
                );
            }
        }

        manager.reserve_range(0, 1);
        manager.reserve_range(bitmap_address / PAGE_SIZE, bitmap_pages);
        manager
    }

    pub fn log(&self) {
        logging::print(format_args!("Available RAM: {} MB\n", self.available_ram_mb));
        logging::print(format_args!("Reserved Regions: {}\n", self.reserved_regions));
        logging::print(format_args!("Free Frames: {}\n", self.free_frames));
    }

    pub fn available_ram_mb(&self) -> u64 {
        self.available_ram_mb
    }

    pub fn free_frames(&self) -> usize {
        self.free_frames
    }

    pub fn reserve_range(&mut self, start_frame: u64, frame_count: usize) {
        for frame_index in start_frame as usize..start_frame as usize + frame_count {
            if self.set_allocated(frame_index) {
                self.free_frames = self.free_frames.saturating_sub(1);
            }
        }
    }

    pub fn release_range(&mut self, start_frame: u64, frame_count: usize) {
        for frame_index in start_frame as usize..start_frame as usize + frame_count {
            if self.clear_allocated(frame_index) {
                self.free_frames = self.free_frames.saturating_add(1);
            }
        }
    }

    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        for word_index in 0..self.bitmap_words {
            let word = unsafe {
                // SAFETY: The bitmap points to valid LOADER_DATA pages reserved for this manager.
                *self.bitmap.as_ptr().add(word_index)
            };

            if word != u64::MAX {
                let bit_index = (!word).trailing_zeros() as usize;
                let frame_index = word_index * 64 + bit_index;
                if frame_index >= self.total_frames {
                    continue;
                }

                if self.set_allocated(frame_index) {
                    self.free_frames = self.free_frames.saturating_sub(1);
                    return Some(PhysFrame { number: frame_index as u64 });
                }
            }
        }

        None
    }

    pub fn free_frame(&mut self, frame: PhysFrame) {
        if self.clear_allocated(frame.number as usize) {
            self.free_frames = self.free_frames.saturating_add(1);
        }
    }

    fn set_allocated(&mut self, frame_index: usize) -> bool {
        let (word_index, mask) = bitmap_mask(frame_index);
        if word_index >= self.bitmap_words {
            return false;
        }

        unsafe {
            // SAFETY: The word index is within the allocated bitmap buffer.
            let word = self.bitmap.as_ptr().add(word_index);
            let current = word.read();
            if current & mask != 0 {
                return false;
            }
            word.write(current | mask);
        }

        true
    }

    fn clear_allocated(&mut self, frame_index: usize) -> bool {
        let (word_index, mask) = bitmap_mask(frame_index);
        if word_index >= self.bitmap_words {
            return false;
        }

        unsafe {
            // SAFETY: The word index is within the allocated bitmap buffer.
            let word = self.bitmap.as_ptr().add(word_index);
            let current = word.read();
            if current & mask == 0 {
                return false;
            }
            word.write(current & !mask);
        }

        true
    }
}

fn bitmap_mask(frame_index: usize) -> (usize, u64) {
    let word_index = frame_index / 64;
    let bit_index = frame_index % 64;
    (word_index, 1u64 << bit_index)
}

fn region_end(descriptor: &MemoryDescriptor) -> u64 {
    descriptor.phys_start + descriptor.page_count as u64 * PAGE_SIZE
}

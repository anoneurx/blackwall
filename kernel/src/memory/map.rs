#[cfg(target_os = "uefi")]
use crate::logging;
#[cfg(target_os = "uefi")]
use uefi::prelude::*;
#[cfg(target_os = "uefi")]
use uefi::table::boot::MemoryType;

#[cfg(target_os = "uefi")]
pub struct MemoryReport {
    pub ram_mb: u64,
    pub reserved_regions: usize,
    pub available_mb: u64,
}

#[cfg(target_os = "uefi")]
impl MemoryReport {
    pub fn log(&self) {
        logging::print(format_args!("RAM Detected: {} MB\n", self.ram_mb));
        logging::print(format_args!("Reserved Regions: {}\n", self.reserved_regions));
        logging::print(format_args!("Available Memory: {} MB\n", self.available_mb));
    }
}

#[cfg(target_os = "uefi")]
pub fn detect(system_table: &SystemTable<Boot>) -> MemoryReport {
    let memory_map = system_table
        .boot_services()
        .memory_map(MemoryType::LOADER_DATA)
        .expect("UEFI memory map retrieval failed");

    let mut total_pages = 0u64;
    let mut available_pages = 0u64;
    let mut reserved_regions = 0usize;

    for descriptor in memory_map.entries() {
        total_pages = total_pages.saturating_add(descriptor.page_count as u64);

        if descriptor.ty == MemoryType::CONVENTIONAL {
            available_pages = available_pages.saturating_add(descriptor.page_count as u64);
        } else {
            reserved_regions = reserved_regions.saturating_add(1);
        }
    }

    MemoryReport {
        ram_mb: total_pages.saturating_mul(4096) / 1024 / 1024,
        reserved_regions,
        available_mb: available_pages.saturating_mul(4096) / 1024 / 1024,
    }
}

#[cfg(not(target_os = "uefi"))]
pub struct MemoryReport;

#[cfg(not(target_os = "uefi"))]
pub fn detect() -> MemoryReport {
    MemoryReport
}

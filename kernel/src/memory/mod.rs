#![cfg(target_os = "uefi")]

pub mod allocator;
pub mod faults;
pub mod map;
pub mod paging;
pub mod physical;
pub mod protection;
pub mod r#virtual;

use crate::logging;
use blackwall_shared::messages;
use uefi::prelude::*;

pub struct MemoryState {
    pub physical: physical::PhysicalMemoryManager,
    pub virtual_memory: r#virtual::VirtualMemoryManager,
    heap: allocator::HeapInfo,
}

impl MemoryState {
    pub fn log(&self) {
        serial_ready(messages::PHYSICAL_MEMORY_MANAGER_READY);
        self.physical.log();

        serial_ready(messages::VIRTUAL_MEMORY_MANAGER_READY);
        self.virtual_memory.log();

        serial_ready(messages::HEAP_INITIALIZED);
        self.heap.log();

        serial_ready(messages::MEMORY_PROTECTION_ENABLED);
    }
}

pub fn init(system_table: &SystemTable<Boot>) -> MemoryState {
    let mut physical = physical::PhysicalMemoryManager::initialize(system_table);
    let virtual_memory = r#virtual::VirtualMemoryManager::initialize(&mut physical);
    let heap = allocator::initialize(system_table, &mut physical);

    protection::enable();
    faults::install();

    MemoryState { physical, virtual_memory, heap }
}

fn serial_ready(message: &str) {
    logging::print(format_args!("{message}\n"));
}

#![cfg(target_os = "uefi")]

pub mod allocator;
pub mod faults;
pub mod map;
pub mod paging;
pub mod physical;
pub mod protection;
pub mod user_access;
pub mod r#virtual;

use crate::logging;
use blackwall_shared::messages;
use uefi::prelude::*;

pub struct MemoryState {
    pub physical: physical::PhysicalMemoryManager,
    pub virtual_memory: r#virtual::VirtualMemoryManager,
    heap: allocator::HeapInfo,
}

// SAFETY: `MemoryState` owns raw page-table/frame pointers (`NonNull` inside
// the physical allocator) that are only ever touched from kernel code while
// holding [`MEMORY`]'s lock or during single-threaded early boot.
unsafe impl Send for MemoryState {}

/// Global handle to the physical + virtual memory managers.
///
/// Published by [`init`] toward the end of boot, before interrupts and the
/// scheduler are enabled.  Syscalls (`fork`, `execve`) need to allocate
/// frames/address spaces after boot, so the managers must be reachable from
/// syscall context — they used to be locals of `init::start`.
pub static MEMORY: crate::sync::spin::SpinLock<Option<MemoryState>> =
    crate::sync::spin::SpinLock::new(None);

/// Physical address of the permanent boot page-table root (the firmware CR3).
///
/// Kernel threads run on it for the lifetime of the system, so
/// [`crate::process::loader::destroy_address_space`] must never free it.
/// Captured at the start of [`crate::init::start`], before any CR3 switch.
pub static BOOT_CR3: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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

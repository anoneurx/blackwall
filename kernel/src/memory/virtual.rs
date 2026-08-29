use crate::logging;

use super::paging::{KERNEL_SPACE_START, KERNEL_VIRTUAL_BASE};
use super::physical::{PhysFrame, PhysicalMemoryManager};

pub struct VirtualMemoryManager {
    root_frame: PhysFrame,
    kernel_virtual_base: u64,
    identity_mapping_enabled: bool,
}

impl VirtualMemoryManager {
    pub fn initialize(physical: &mut PhysicalMemoryManager) -> Self {
        let root_frame =
            physical.allocate_frame().expect("failed to allocate virtual memory root frame");

        unsafe {
            // SAFETY: The frame was reserved from the boot-time physical allocator and is usable.
            core::ptr::write_bytes(root_frame.address() as *mut u8, 0, 4096);
        }

        Self {
            root_frame,
            kernel_virtual_base: KERNEL_VIRTUAL_BASE,
            identity_mapping_enabled: true,
        }
    }

    pub fn log(&self) {
        logging::print(format_args!("Kernel mapped at 0x{:x}\n", self.kernel_virtual_base));
        logging::print(format_args!("Kernel Space Start: 0x{:x}\n", KERNEL_SPACE_START));
    }

    pub fn kernel_virtual_base(&self) -> u64 {
        self.kernel_virtual_base
    }

    pub fn root_frame(&self) -> PhysFrame {
        self.root_frame
    }

    pub fn identity_mapping_enabled(&self) -> bool {
        self.identity_mapping_enabled
    }

    /// Map a 4 KiB `page` virtual address to a `frame` physical address.
    ///
    /// # Safety
    ///
    /// * `self.root_frame` must be a valid, exclusively-owned PML4 physical
    ///   frame zeroed at construction time. No other CPU may be walking this
    ///   page table concurrently (caller must hold appropriate locks).
    /// * `page` must be canonical (bits 48–63 sign-extended from bit 47).
    /// * `frame` must be a valid physical address returned by the allocator.
    /// * After returning, the caller must flush the TLB entry (`invlpg page`)
    ///   if the page table is active on any CPU.
    pub unsafe fn map_page(
        &mut self,
        page: u64,
        frame: u64,
        flags: u64,
        allocator: &mut PhysicalMemoryManager,
    ) -> Result<(), &'static str> {
        use super::paging::{PageTable, PAGE_PRESENT};

        // SAFETY: root_frame was allocated and zeroed during VMM init; we have
        // exclusive mutable access via &mut self.
        let p4_ptr = self.root_frame.address() as *mut PageTable;
        let p4 = &mut *p4_ptr;

        let p4_idx = ((page >> 39) & 0x1ff) as usize;
        let p3_idx = ((page >> 30) & 0x1ff) as usize;
        let p2_idx = ((page >> 21) & 0x1ff) as usize;
        let p1_idx = ((page >> 12) & 0x1ff) as usize;

        let p3_ptr = Self::get_or_create_table(&mut p4.entries[p4_idx], allocator, flags)?;
        let p3 = &mut *p3_ptr;

        let p2_ptr = Self::get_or_create_table(&mut p3.entries[p3_idx], allocator, flags)?;
        let p2 = &mut *p2_ptr;

        let p1_ptr = Self::get_or_create_table(&mut p2.entries[p2_idx], allocator, flags)?;
        let p1 = &mut *p1_ptr;

        p1.entries[p1_idx].set(frame, flags | PAGE_PRESENT);

        Ok(())
    }

    /// Resolve or allocate the next-level page table from a PTE.
    ///
    /// # Safety
    ///
    /// * `entry` must be part of a valid, exclusively-owned page table frame.
    /// * When creating a new table, `allocator.allocate_frame()` returns a
    ///   physical frame not aliased by any other live reference.
    /// * The returned raw pointer is valid for the duration the page table
    ///   structure exists and is exclusively owned by this VMM.
    unsafe fn get_or_create_table(
        entry: &mut super::paging::PageTableEntry,
        allocator: &mut PhysicalMemoryManager,
        flags: u64,
    ) -> Result<*mut super::paging::PageTable, &'static str> {
        use super::paging::{PageTable, PAGE_PRESENT, PAGE_USER_ACCESSIBLE, PAGE_WRITABLE};

        if entry.is_present() {
            // Ensure intermediate tables have appropriate flags (e.g., USER_ACCESSIBLE)
            let current_flags = entry.flags();
            let new_flags = current_flags | (flags & (PAGE_WRITABLE | PAGE_USER_ACCESSIBLE));
            entry.set(entry.address(), new_flags);
            // SAFETY: The entry's address points to a valid page table frame
            // allocated and zeroed previously in this function or during boot.
            Ok(entry.address() as *mut PageTable)
        } else {
            let frame = allocator.allocate_frame().ok_or("Out of physical memory")?;
            let ptr = frame.address() as *mut PageTable;
            // SAFETY: ptr points to a freshly allocated physical frame; zeroing
            // 4096 bytes via write_bytes is valid for any *mut u8.
            core::ptr::write_bytes(ptr as *mut u8, 0, 4096);
            entry.set(frame.address(), flags | PAGE_PRESENT);
            Ok(ptr)
        }
    }
}

use super::elf::{ElfLoader, PF_W, PF_X, R_X86_64_RELATIVE};
use crate::arch::x86_64::serial;
use crate::memory::{
    paging::{
        align_up, PageTable, PageTableEntry, PAGE_HUGE, PAGE_NO_EXECUTE, PAGE_PRESENT, PAGE_SIZE,
        PAGE_USER_ACCESSIBLE, PAGE_WRITABLE,
    },
    physical::PhysicalMemoryManager,
    r#virtual::VirtualMemoryManager,
};
use alloc::vec::Vec;
use core::arch::asm;

/// Virtual address where the user stack top is placed.
pub const USER_STACK_TOP: u64 = 0x0000_7fff_ffff_0000;
/// Size of the user stack: 64 KiB.
pub const USER_STACK_SIZE: u64 = 64 * 1024;
/// For PIE binaries, the virtual base we map them at.
pub const USER_LOAD_BASE: u64 = 0x0000_0000_0040_0000; // 4 MiB

/// Everything the scheduler needs to start a loaded user process.
pub struct UserImage {
    /// Physical address of the process page table (to be loaded into CR3).
    pub cr3: u64,
    /// Ring-3 entry point.
    pub entry: u64,
    /// Initial ring-3 stack pointer.
    pub user_rsp: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers for walking / building the VMM's *own* PML4
// (separate from the boot-time CR3 walk)
// ─────────────────────────────────────────────────────────────────────────────

/// Map `virt` → `phys` with `flags` into the page table rooted at `p4_phys`.
///
/// Intermediate page-table frames are allocated from `pmm` as needed.
/// This does NOT switch CR3.
///
/// # Safety
/// - `p4_phys` must be a valid, writable, zeroed 4-KiB-aligned physical address.
/// - Caller must be in ring 0.
unsafe fn map_into_table(
    p4_phys: u64,
    virt: u64,
    phys: u64,
    flags: u64,
    pmm: &mut PhysicalMemoryManager,
) {
    let p4 = &mut *(p4_phys as *mut PageTable);

    let p4i = ((virt >> 39) & 0x1FF) as usize;
    let p3i = ((virt >> 30) & 0x1FF) as usize;
    let p2i = ((virt >> 21) & 0x1FF) as usize;
    let p1i = ((virt >> 12) & 0x1FF) as usize;

    let p3 = get_or_alloc_entry(&mut p4.entries[p4i], flags, pmm);
    let p2 = get_or_alloc_entry(&mut (*p3).entries[p3i], flags, pmm);
    let p1 = get_or_alloc_entry(&mut (*p2).entries[p2i], flags, pmm);

    (*p1).entries[p1i].set(phys, flags | PAGE_PRESENT);
}

/// Walk a page-table entry; allocate the next-level table if absent.
///
/// If `entry` is a huge page (2 MiB at P2, 1 GiB at P3), it is *split* into a
/// fresh table of 4 KiB pages so the leaf we install does not alias a larger
/// region owned by the boot identity map.
unsafe fn get_or_alloc_entry(
    entry: &mut PageTableEntry,
    flags: u64,
    pmm: &mut PhysicalMemoryManager,
) -> *mut PageTable {
    if entry.is_present() {
        if entry.flags() & PAGE_HUGE != 0 {
            let region_base = entry.address();
            let child_flags = entry.flags() & !PAGE_HUGE;
            let frame = pmm.allocate_frame().expect("OOM: split huge page");
            let ptr = frame.address() as *mut PageTable;
            core::ptr::write_bytes(ptr as *mut u8, 0, 4096);
            for i in 0..512 {
                (*ptr).entries[i]
                    .set(region_base + (i as u64) * PAGE_SIZE, child_flags | PAGE_PRESENT);
            }
            entry.set(frame.address(), flags | PAGE_PRESENT);
            ptr
        } else {
            let new_flags =
                entry.flags() | (flags & (PAGE_WRITABLE | PAGE_USER_ACCESSIBLE)) | PAGE_PRESENT;
            entry.set(entry.address(), new_flags);
            entry.address() as *mut PageTable
        }
    } else {
        let frame = pmm.allocate_frame().expect("OOM: page-table frame");
        let ptr = frame.address() as *mut PageTable;
        core::ptr::write_bytes(ptr as *mut u8, 0, 4096);
        entry.set(frame.address(), flags | PAGE_PRESENT);
        ptr
    }
}

/// Recursively deep-copy the boot page-table hierarchy rooted at `src_phys`
/// into the already-zeroed root table at `dst_phys`.
unsafe fn deep_clone_table(
    src_phys: u64,
    dst_phys: u64,
    pmm: &mut PhysicalMemoryManager,
    depth: u32,
) {
    if depth > 4 {
        return;
    }
    let dst = dst_phys as *mut PageTable;
    let src = src_phys as *const PageTable;

    for i in 0..512 {
        let e = (*src).entries[i];
        if !e.is_present() {
            continue;
        }
        if e.flags() & PAGE_HUGE != 0 {
            (*dst).entries[i] = e;
        } else if depth >= 3 {
            (*dst).entries[i] = e;
        } else {
            let frame = pmm.allocate_frame().expect("OOM: clone table");
            let child_phys = frame.address();
            let mut ne = e;
            ne.set(child_phys, e.flags());
            (*dst).entries[i] = ne;
            deep_clone_table(e.address(), child_phys, pmm, depth + 1);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Load a user process from an ELF byte slice into a fresh address space.
///
/// Returns the page-table root, entry point and initial stack pointer; it does
/// **not** switch to ring 3.  The scheduler starts the process later, through
/// the normal trap-frame restore, which is what makes it a first-class task.
///
/// # Safety
/// - `vmm` and `pmm` must be valid and exclusively owned.
pub unsafe fn build_user_image(
    elf_bytes: &[u8],
    vmm: &mut VirtualMemoryManager,
    pmm: &mut PhysicalMemoryManager,
) -> UserImage {
    let loader = ElfLoader::parse(elf_bytes).expect("Failed to parse init ELF");
    let load_base = if loader.is_pie() { USER_LOAD_BASE } else { 0 };
    let entry = loader.entry(load_base);
    serial::line(&alloc::format!(
        "[DBG] ELF parsed: pie={}, entry=0x{:x}, segments={}",
        loader.is_pie(),
        entry,
        loader.load_segments().count()
    ));

    let user_p4_phys = vmm.root_frame().address();

    // ── Copy the kernel's boot tables into the new PML4 ──────────────────────
    // The kernel keeps executing through the switch, so its image and stacks
    // must stay mapped in the new address space.
    {
        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        let boot_p4_phys = cr3 & !0xFFF_u64;
        deep_clone_table(boot_p4_phys, user_p4_phys, pmm, 0);
    }

    // ── Pass 1: collect per-page permissions ─────────────────────────────────
    // Two LOAD segments may share a page (for example an RX segment ending and
    // an RO segment beginning inside the same 4 KiB page).  The permissions of
    // such a page must be the *union* of both segments' wishes; mapping them in
    // one pass would let the second segment clobber the first and, in the worst
    // case, strip execute permission from a page holding code (the original
    // Ring-3 page-fault bug).
    let mut pages: Vec<(u64, u64)> = Vec::new();
    for ph in loader.load_segments() {
        if ph.p_memsz == 0 {
            continue;
        }
        let virt_base = load_base + ph.p_vaddr;
        let virt_end = virt_base + ph.p_memsz;
        let first_page = virt_base & !(PAGE_SIZE - 1);
        let last_page = align_up(virt_end);

        let mut flags = PAGE_PRESENT | PAGE_USER_ACCESSIBLE;
        if ph.p_flags & PF_W != 0 {
            flags |= PAGE_WRITABLE;
        }
        if ph.p_flags & PF_X == 0 {
            flags |= PAGE_NO_EXECUTE;
        }

        let mut page = first_page;
        while page < last_page {
            match pages.iter_mut().find(|(v, _)| *v == page) {
                Some((_, existing)) => {
                    let writable = (*existing & PAGE_WRITABLE != 0)
                        || (flags & PAGE_WRITABLE != 0);
                    let no_exec = (*existing & PAGE_NO_EXECUTE != 0)
                        && (flags & PAGE_NO_EXECUTE != 0);
                    let mut merged = *existing | flags;
                    if writable {
                        merged |= PAGE_WRITABLE;
                    } else {
                        merged &= !PAGE_WRITABLE;
                    }
                    if no_exec {
                        merged |= PAGE_NO_EXECUTE;
                    } else {
                        merged &= !PAGE_NO_EXECUTE;
                    }
                    *existing = merged;
                }
                None => pages.push((page, flags)),
            }
            page += PAGE_SIZE;
        }
    }

    // ── Pass 2: allocate, populate and map each page exactly once ────────────
    // We also record each page's backing frame so PIE relocations can be
    // applied afterwards through the kernel's identity map.
    let mut page_frames: Vec<(u64, u64)> = Vec::new();
    for &(page, flags) in &pages {
        let frame = pmm.allocate_frame().expect("OOM: user segment");
        let dst = frame.address();
        page_frames.push((page, dst));
        core::ptr::write_bytes(dst as *mut u8, 0, PAGE_SIZE as usize);

        for ph in loader.load_segments() {
            if ph.p_filesz == 0 || ph.p_memsz == 0 {
                continue;
            }
            let virt_base = load_base + ph.p_vaddr;
            let virt_end = virt_base + ph.p_memsz;
            let file_end = virt_base + ph.p_filesz;

            // Intersection of [this segment] ∩ [this page] ∩ [file-backed part].
            let start = virt_base.max(page);
            let end = virt_end.min(page + PAGE_SIZE).min(file_end);
            if start >= end {
                continue;
            }

            let file_off = (ph.p_offset + (start - virt_base)) as usize;
            let dst_off = (start - page) as usize;
            let len = (end - start) as usize;
            if file_off + len > elf_bytes.len() {
                continue;
            }
            core::ptr::copy_nonoverlapping(
                elf_bytes.as_ptr().add(file_off),
                (dst as *mut u8).add(dst_off),
                len,
            );
        }

        map_into_table(user_p4_phys, page, dst, flags, pmm);
    }

    // ── Allocate and map the user stack ──────────────────────────────────────
    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    let stack_flags = PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER_ACCESSIBLE | PAGE_NO_EXECUTE;
    let mut page = stack_bottom;
    while page < USER_STACK_TOP {
        let frame = pmm.allocate_frame().expect("OOM: user stack");
        core::ptr::write_bytes(frame.address() as *mut u8, 0, PAGE_SIZE as usize);
        map_into_table(user_p4_phys, page, frame.address(), stack_flags, pmm);
        page += PAGE_SIZE;
    }

    // ── Apply dynamic relocations (position-independent executable) ──────────
    // The init binary is a PIE whose GOT slots are filled by R_X86_64_RELATIVE
    // relocations.  At a non-zero load base every slot must become
    // `load_base + addend`; leaving them zero makes every indirect call jump to
    // address 0 (the Ring-3 instruction-fetch fault we used to see on `cat`).
    let mut reloc_count = 0usize;
    for rela in loader.relocations() {
        if rela.r_type() != R_X86_64_RELATIVE {
            continue;
        }
        let target = load_base.wrapping_add(rela.r_offset);
        let value = load_base.wrapping_add(rela.r_addend as u64);
        if let Some(&(_, phys)) =
            page_frames.iter().find(|(v, _)| target >= *v && target < *v + PAGE_SIZE)
        {
            let phys_addr = phys + (target & (PAGE_SIZE - 1));
            core::ptr::write_unaligned(phys_addr as *mut u64, value);
            reloc_count += 1;
        }
    }

    serial::line(&alloc::format!(
        "Userspace: image ready (segments + stack mapped, {} relocations applied)",
        reloc_count
    ));

    UserImage {
        cr3: user_p4_phys,
        entry,
        user_rsp: USER_STACK_TOP - 16,
    }
}

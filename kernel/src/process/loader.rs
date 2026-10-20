use super::elf::{ElfLoader, PF_W, PF_X, R_X86_64_RELATIVE};
use crate::arch::x86_64::serial;
use crate::memory::{
    paging::{
        align_up, PageTable, PageTableEntry, PAGE_HUGE, PAGE_NO_EXECUTE, PAGE_OWNED, PAGE_PRESENT,
        PAGE_SIZE, PAGE_USER_ACCESSIBLE, PAGE_WRITABLE,
    },
    physical::{PhysFrame, PhysicalMemoryManager},
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

/// Set to `true` to print ELF-parse / image-ready diagnostics on the serial
/// console.  Off by default: one line per `execve` is noise in normal boots.
const LOADER_DEBUG: bool = false;

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

/// A shareable copy of `e`: the identical mapping, but with [`PAGE_OWNED`]
/// cleared so the receiving address space never frees a frame it does not
/// own.  Used on every share branch of [`clone_space`] and
/// [`share_kernel_root`].
#[inline]
fn shared_entry(e: PageTableEntry) -> PageTableEntry {
    let mut ne = e;
    ne.set(e.address(), e.flags() & !PAGE_OWNED);
    ne
}

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

    (*p1).entries[p1i].set(phys, flags | PAGE_PRESENT | PAGE_OWNED);
}

/// Walk a page-table entry; allocate the next-level table if absent.
///
/// Ownership rules (the invariant [`destroy_address_space`] relies on):
/// a newly allocated or copied table frame is always marked [`PAGE_OWNED`],
/// and a table that is merely *shared* from another address space (the boot
/// root's kernel/identity mappings) is copied before it is mutated, so no two
/// address spaces ever reference the same OWNED frame.
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
            // The split table frame belongs to this address space; the 4 KiB
            // identity entries inside it do not (they map kernel physical
            // memory, so they carry no OWNED bit and are never freed).
            entry.set(frame.address(), flags | PAGE_PRESENT | PAGE_OWNED);
            ptr
        } else if entry.flags() & PAGE_OWNED != 0 {
            // Private table: mutate in place, preserving ownership.
            let new_flags =
                entry.flags() | (flags & (PAGE_WRITABLE | PAGE_USER_ACCESSIBLE)) | PAGE_PRESENT;
            entry.set(entry.address(), new_flags);
            entry.address() as *mut PageTable
        } else {
            // Shared kernel/firmware table: never mutate it in place — other
            // address spaces and the kernel itself walk the same frames.  Take
            // a private copy and mark it owned.
            let frame = pmm.allocate_frame().expect("OOM: copy shared table");
            let ptr = frame.address() as *mut PageTable;
            core::ptr::copy_nonoverlapping(
                entry.address() as *const u8,
                ptr as *mut u8,
                PAGE_SIZE as usize,
            );
            let new_flags = entry.flags()
                | (flags & (PAGE_WRITABLE | PAGE_USER_ACCESSIBLE))
                | PAGE_PRESENT
                | PAGE_OWNED;
            entry.set(frame.address(), new_flags);
            ptr
        }
    } else {
        let frame = pmm.allocate_frame().expect("OOM: page-table frame");
        let ptr = frame.address() as *mut PageTable;
        core::ptr::write_bytes(ptr as *mut u8, 0, 4096);
        entry.set(frame.address(), flags | PAGE_PRESENT | PAGE_OWNED);
        ptr
    }
}

/// Copy the boot PML4's entries verbatim into the root at `dst_phys`,
/// clearing [`PAGE_OWNED`] on each so the new space never claims the
/// firmware's permanent tables.
///
/// Sharing (instead of deep-cloning) keeps `execve` cheap and avoids
/// duplicating the firmware's 4-KiB identity map: the kernel and identity
/// mappings behind those entries trace back to the permanent boot root
/// (captured in [`crate::memory::BOOT_CR3`]), which is never freed.
///
/// # Safety
/// `boot_p4_phys` must be the live boot page-table root; `dst_phys` a valid
/// writable PML4 frame.
unsafe fn share_kernel_root(boot_p4_phys: u64, dst_phys: u64) {
    let src = boot_p4_phys as *const PageTable;
    let dst = dst_phys as *mut PageTable;
    for i in 0..512 {
        (*dst).entries[i] = shared_entry((*src).entries[i]);
    }
}

/// Whether user leaf pages are copied into the new address space.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UserLeaves {
    /// `fork`: give every user page a private copy of its data frame.
    Copy,
    /// `execve`: leave the user range empty; the new image is mapped later.
    Drop,
}

/// Create a fresh address space for a new process by cloning the *kernel*
/// mappings (identity map, kernel image, heap) and, optionally, copying the
/// user pages.
///
/// Only subtrees that can intersect the user range
/// `[USER_LOAD_BASE, USER_STACK_TOP)` are walked and given new table frames;
/// everything else is shared verbatim.  Kernel mappings are never mutated
/// after boot, so sharing their tables is safe and keeps `fork` cheap — a
/// naive full deep-clone of the firmware's 4 KiB identity map both wastes
/// thousands of frames and trips over firmware table entries.
///
/// # Safety
/// `src_cr3` must be the physical address of a live page-table root and `pmm`
/// must be valid and exclusively owned.
pub unsafe fn clone_address_space(src_cr3: u64, pmm: &mut PhysicalMemoryManager) -> u64 {
    let frame = pmm.allocate_frame().expect("OOM: fork address space");
    let dst = frame.address();
    core::ptr::write_bytes(dst as *mut u8, 0, PAGE_SIZE as usize);
    clone_space(src_cr3, dst, pmm, 0, 0, UserLeaves::Copy);
    dst
}

/// Like [`clone_address_space`] but with an empty user range (used by
/// `execve`, which maps a brand-new image on top).
///
/// # Safety
/// Same as [`clone_address_space`].
pub unsafe fn create_address_space(pmm: &mut PhysicalMemoryManager) -> u64 {
    let cr3: u64;
    asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    let src = cr3 & !0xFFF_u64;

    let frame = pmm.allocate_frame().expect("OOM: exec address space");
    let dst = frame.address();
    core::ptr::write_bytes(dst as *mut u8, 0, PAGE_SIZE as usize);
    clone_space(src, dst, pmm, 0, 0, UserLeaves::Drop);
    dst
}

/// Top of the window that may contain the loaded user image (code, rodata,
/// data, bss).  Anything else within the low canonical half — the kernel's
/// identity map, firmware MMIO mappings at 512 GiB and above — is *not* user
/// memory, even though it lives below [`USER_STACK_TOP`].
const USER_IMAGE_TOP: u64 = USER_LOAD_BASE + 0x4000_0000; // 4 MiB .. 1 GiB + 4 MiB

/// True when the whole virtual range `[lo, hi)` can hold user pages.
fn in_user_window(lo: u64, hi: u64) -> bool {
    let image = lo < USER_IMAGE_TOP && hi > USER_LOAD_BASE;
    let stack = lo < USER_STACK_TOP && hi > USER_STACK_TOP - USER_STACK_SIZE;
    image || stack
}

/// Read-only scan: does the subtree rooted at `src_table` (starting at level
/// `depth` covering `virt_base`) contain any user-accessible leaf page?
///
/// This lets [`clone_space`] share whole kernel subtrees verbatim instead of
/// allocating private copies of the firmware identity map.  Only subtrees that
/// intersect a user window are descended into, so firmware MMIO tables are
/// never touched.
unsafe fn has_user_page(src_table: u64, depth: u32, virt_base: u64) -> bool {
    if depth > 3 {
        return false;
    }
    let shift = 39 - 9 * depth;
    let slot_size = 1u64 << shift;
    let src = src_table as *const PageTable;

    for i in 0..512usize {
        let virt = virt_base | ((i as u64) << shift);
        if !in_user_window(virt, virt.saturating_add(slot_size)) {
            continue;
        }
        let e = (*src).entries[i];
        if !e.is_present() || e.flags() & PAGE_HUGE != 0 {
            continue;
        }
        if depth == 3 {
            if e.flags() & PAGE_USER_ACCESSIBLE != 0 {
                return true;
            }
        } else if has_user_page(e.address(), depth + 1, virt) {
            return true;
        }
    }
    false
}

/// Recursive worker for [`clone_address_space`]/[`create_address_space`].
unsafe fn clone_space(
    src_table: u64,
    dst_table: u64,
    pmm: &mut PhysicalMemoryManager,
    depth: u32,
    virt_base: u64,
    mode: UserLeaves,
) {
    let shift = 39 - 9 * depth;
    let slot_size = 1u64 << shift;
    let src = src_table as *const PageTable;
    let dst = dst_table as *mut PageTable;

    for i in 0..512usize {
        let virt = virt_base | ((i as u64) << shift);
        let subtree_hi = virt.saturating_add(slot_size);
        let e = (*src).entries[i];

        // Subtrees that cannot hold user pages are kernel mappings: share the
        // table verbatim (no new frames, no mutation).  `shared_entry` clears
        // PAGE_OWNED so this space never frees a frame it does not own.
        if !in_user_window(virt, subtree_hi) {
            (*dst).entries[i] = shared_entry(e);
            continue;
        }

        if !e.is_present() {
            continue;
        }
        // Huge kernel pages inside the window (should not normally happen) are
        // shared as-is rather than split.
        if e.flags() & PAGE_HUGE != 0 {
            (*dst).entries[i] = shared_entry(e);
            continue;
        }

        if depth == 3 {
            // Leaf: privatise user pages, share kernel pages.
            if e.flags() & PAGE_USER_ACCESSIBLE != 0 {
                match mode {
                    UserLeaves::Drop => { /* leave slot empty */ }
                    UserLeaves::Copy => {
                        let nf = pmm.allocate_frame().expect("OOM: fork user page");
                        core::ptr::copy_nonoverlapping(
                            e.address() as *const u8,
                            nf.address() as *mut u8,
                            PAGE_SIZE as usize,
                        );
                        let mut ne = e;
                        ne.set(nf.address(), e.flags() | PAGE_OWNED);
                        (*dst).entries[i] = ne;
                    }
                }
            } else {
                (*dst).entries[i] = shared_entry(e);
            }
        } else if !has_user_page(e.address(), depth + 1, virt) {
            // Kernel-only subtree inside the window (e.g. the identity map):
            // share it verbatim so we do not duplicate its tables.
            (*dst).entries[i] = shared_entry(e);
        } else {
            // Subtree that really contains user pages: give the child its own
            // table frame so the two address spaces never alias a table.
            let nf = pmm.allocate_frame().expect("OOM: clone table");
            let child_phys = nf.address();
            let mut ne = e;
            ne.set(child_phys, e.flags() | PAGE_OWNED);
            (*dst).entries[i] = ne;
            clone_space(e.address(), child_phys, pmm, depth + 1, virt, mode);
        }
    }
}

/// Free every frame owned by the address space rooted at `root_phys`, plus
/// the root frame itself.
///
/// Only frames marked [`PAGE_OWNED`] are freed: shared kernel/firmware tables
/// trace back to the permanent boot root ([`crate::memory::BOOT_CR3`]) and are
/// never touched.  The boot root itself is refused outright.
///
/// # Safety
/// `root_phys` must be a live page-table root that is no longer loaded in any
/// task's CR3; `pmm` must be valid and exclusively owned.
pub unsafe fn destroy_address_space(root_phys: u64, pmm: &mut PhysicalMemoryManager) {
    let root = root_phys & !0xFFF_u64;
    // The firmware's root is permanent: kernel threads still run on it.
    if root == crate::memory::BOOT_CR3.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    free_owned_table(root, 0, pmm);
    pmm.free_frame(PhysFrame::containing(root));
}

/// Recursively free the OWNED frames of the subtree rooted at `table_phys`
/// (level `depth`, 0 = PML4).  The table frame itself is freed by the caller;
/// at `depth == 3` the entries are data pages and are freed directly.
unsafe fn free_owned_table(table_phys: u64, depth: u32, pmm: &mut PhysicalMemoryManager) {
    let table = table_phys as *const PageTable;
    for i in 0..512usize {
        let e = (*table).entries[i];
        if !e.is_present() || e.flags() & PAGE_HUGE != 0 {
            continue;
        }
        if e.flags() & PAGE_OWNED == 0 {
            continue;
        }
        if depth == 3 {
            pmm.free_frame(PhysFrame::containing(e.address()));
        } else {
            free_owned_table(e.address(), depth + 1, pmm);
            pmm.free_frame(PhysFrame::containing(e.address()));
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
    let user_p4_phys = vmm.root_frame().address();

    // ── Share the kernel's boot tables with the new PML4 ─────────────────────
    // The kernel keeps executing through the switch, so its image and stacks
    // must stay mapped in the new address space.  The boot root's entries are
    // copied verbatim (OWNED cleared): its tables are permanent and shared by
    // every address space, never freed.
    {
        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        let boot_p4_phys = cr3 & !0xFFF_u64;
        share_kernel_root(boot_p4_phys, user_p4_phys);
    }

    build_user_image_into(elf_bytes, user_p4_phys, pmm)
}

/// Load an ELF into an **already-created** address space root.
///
/// `p4_phys` must be a live PML4 that already maps the kernel image (see
/// [`create_address_space`]); this function only populates user segments, the
/// user stack and PIE relocations.
///
/// # Safety
/// - `p4_phys` must be a valid, writable page-table root and `pmm` valid and
///   exclusively owned.
pub unsafe fn build_user_image_into(
    elf_bytes: &[u8],
    user_p4_phys: u64,
    pmm: &mut PhysicalMemoryManager,
) -> UserImage {
    let loader = ElfLoader::parse(elf_bytes).expect("Failed to parse init ELF");
    let load_base = if loader.is_pie() { USER_LOAD_BASE } else { 0 };
    let entry = loader.entry(load_base);
    if LOADER_DEBUG {
        serial::line(&alloc::format!(
            "[DBG] ELF parsed: pie={}, entry=0x{:x}, segments={}",
            loader.is_pie(),
            entry,
            loader.load_segments().count()
        ));
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
                    let writable = (*existing & PAGE_WRITABLE != 0) || (flags & PAGE_WRITABLE != 0);
                    let no_exec =
                        (*existing & PAGE_NO_EXECUTE != 0) && (flags & PAGE_NO_EXECUTE != 0);
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

    if LOADER_DEBUG {
        serial::line(&alloc::format!(
            "Userspace: image ready (segments + stack mapped, {} relocations applied)",
            reloc_count
        ));
    }

    UserImage { cr3: user_p4_phys, entry, user_rsp: USER_STACK_TOP - 16 }
}

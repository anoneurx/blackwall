use super::elf::{ElfLoader, PF_W, PF_X};
use crate::arch::x86_64::{gdt, serial, syscall};
use crate::memory::{
    paging::{
        align_up, PageTable, PageTableEntry, PAGE_HUGE, PAGE_NO_EXECUTE, PAGE_PRESENT, PAGE_SIZE,
        PAGE_USER_ACCESSIBLE, PAGE_WRITABLE,
    },
    physical::PhysicalMemoryManager,
    r#virtual::VirtualMemoryManager,
};
use core::arch::asm;

/// Virtual address where the user stack top is placed.
pub const USER_STACK_TOP: u64 = 0x0000_7fff_ffff_0000;
/// Size of the user stack: 64 KiB.
pub const USER_STACK_SIZE: u64 = 64 * 1024;

/// For PIE binaries, the virtual base we map them at.
pub const USER_LOAD_BASE: u64 = 0x0000_0000_0040_0000; // 4 MiB

/// Segment selectors (index << 3 | RPL).
/// See gdt.rs: User Data = entry 3, User Code = entry 4, both with RPL=3.
const USER_DATA_SEL: u64 = (3 << 3) | 3;
const USER_CODE_SEL: u64 = (4 << 3) | 3;

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
            // Split the huge page into 512 x 4 KiB entries pointing at the
            // same underlying 2 MiB / 1 GiB region, then keep descending.
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
///
/// This yields a page table *independent* of the kernel's active tables, so the
/// user loader can add ring-3 mappings and split huge pages without mutating
/// (and corrupting) the tables the kernel is currently executing from.
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
            // Huge pages (1 GiB at P3, 2 MiB at P2) are kept as-is; they are
            // read-only-shared with the boot tables (overlays that touch them
            // split them later).
            (*dst).entries[i] = e;
        } else if depth >= 3 {
            // P1 tables hold leaf 4 KiB mappings — copy directly, no child.
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

/// Load a user process from an ELF byte slice and jump to Ring 3.
///
/// # Safety
/// - `vmm` and `pmm` must be valid and exclusively owned.
/// - `kernel_stack_top` must point to a valid kernel stack.
/// - This function does **not** return; it drops to Ring 3 via `iretq`.
pub unsafe fn spawn_user_process(
    elf_bytes: &[u8],
    vmm: &mut VirtualMemoryManager,
    pmm: &mut PhysicalMemoryManager,
    kernel_stack_top: u64,
) -> ! {
    // ── Parse ELF ────────────────────────────────────────────────────────────
    serial::line("[DBG] ElfLoader::parse...");
    let loader = ElfLoader::parse(elf_bytes).expect("Failed to parse init ELF");
    let load_base = if loader.is_pie() { USER_LOAD_BASE } else { 0 };
    let entry = loader.entry(load_base);
    serial::line(&alloc::format!(
        "[DBG] parse ok, is_pie={}, entry=0x{:x}, segs={}",
        loader.is_pie(),
        entry,
        loader.load_segments().count()
    ));

    serial::line("Userspace: Loading init binary...");

    // The VMM owns a freshly-zeroed PML4 frame for the user process.
    let user_p4_phys = vmm.root_frame().address();
    serial::line(&alloc::format!("[DBG] user_p4_phys=0x{:x}", user_p4_phys));

    // ── Copy kernel PML4 entries into the new page table ─────────────────────
    // The kernel currently runs identity-mapped under UEFI paging (PE loaded at
    // a low address). We deep-clone the FULL boot CR3 page table hierarchy (all
    // 512 slots) so that code executing right after `mov cr3` stays reachable,
    // while keeping the kernel's own live tables untouched.
    {
        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        let boot_p4_phys = cr3 & !0xFFF_u64;
        serial::line(&alloc::format!("[DBG] cloning boot tables from 0x{:x}", boot_p4_phys));
        deep_clone_table(boot_p4_phys, user_p4_phys, pmm, 0);
        serial::line("[DBG] clone done");
    }

    // ── Map each LOAD segment into the user PML4 ─────────────────────────────
    for ph in loader.load_segments() {
        if ph.p_memsz == 0 {
            continue;
        }

        serial::line(&alloc::format!(
            "[DBG] LOAD seg vaddr=0x{:x} filesz=0x{:x} memsz=0x{:x} flags=0x{:x}",
            ph.p_vaddr,
            ph.p_filesz,
            ph.p_memsz,
            ph.p_flags
        ));

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
            let frame = pmm.allocate_frame().expect("OOM: user segment");
            serial::line(&alloc::format!(
                "[DBG] frame=0x{:x} for virt page 0x{:x}",
                frame.address(),
                page
            ));
            core::ptr::write_bytes(frame.address() as *mut u8, 0, PAGE_SIZE as usize);
            serial::line("[DBG] zeroed");

            // Copy file data that overlaps this page.
            let page_offset_in_seg = page.saturating_sub(virt_base);
            let file_start = (ph.p_offset + page_offset_in_seg) as usize;
            let file_end = (ph.p_offset + ph.p_filesz) as usize;

            if file_start < file_end && file_start < elf_bytes.len() {
                let copy_len = (file_end - file_start)
                    .min(PAGE_SIZE as usize)
                    .min(elf_bytes.len() - file_start);
                core::ptr::copy_nonoverlapping(
                    elf_bytes.as_ptr().add(file_start),
                    frame.address() as *mut u8,
                    copy_len,
                );
            }
            serial::line("[DBG] copied");

            map_into_table(user_p4_phys, page, frame.address(), flags, pmm);
            serial::line("[DBG] mapped");
            page += PAGE_SIZE;
        }
        serial::line("[DBG] segment done");
    }

    serial::line("Userspace: Segments mapped");

    // ── Allocate and map user stack ───────────────────────────────────────────
    let stack_bottom = USER_STACK_TOP - USER_STACK_SIZE;
    let stack_flags = PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER_ACCESSIBLE | PAGE_NO_EXECUTE;

    let mut page = stack_bottom;
    while page < USER_STACK_TOP {
        let frame = pmm.allocate_frame().expect("OOM: user stack");
        core::ptr::write_bytes(frame.address() as *mut u8, 0, PAGE_SIZE as usize);
        map_into_table(user_p4_phys, page, frame.address(), stack_flags, pmm);
        page += PAGE_SIZE;
    }

    serial::line("Userspace: Stack mapped");

    // ── Update TSS + MSR syscall kernel stack ─────────────────────────────────
    gdt::set_tss_stack(kernel_stack_top);
    syscall::SYSCALL_KERNEL_STACK = kernel_stack_top;

    // ── Switch CR3 to the user process page table ────────────────────────────
    serial::line("Userspace: Switching CR3...");
    asm!("mov cr3, {}", in(reg) user_p4_phys, options(nostack, preserves_flags));

    serial::line("Userspace: Entering Ring 3 via iretq...");

    // ── Build iretq frame and drop to Ring 3 ─────────────────────────────────
    // Stack layout pushed in reverse: SS, RSP, RFLAGS, CS, RIP
    let user_rsp = USER_STACK_TOP - 8; // 16-byte aligned after the `call` that iretq fakes.
    let rflags: u64 = 0x202; // IF=1, reserved bit 1.

    asm!(
        "mov rsp, {kstack}",
        "push {ss}",
        "push {user_rsp}",
        "push {rflags}",
        "push {cs}",
        "push {entry}",
        // Zero all GPRs to avoid leaking kernel data into userspace.
        "xor rax, rax", "xor rbx, rbx", "xor rcx, rcx", "xor rdx, rdx",
        "xor rsi, rsi", "xor rdi, rdi",
        "xor r8,  r8",  "xor r9,  r9",  "xor r10, r10", "xor r11, r11",
        "xor r12, r12", "xor r13, r13", "xor r14, r14", "xor r15, r15",
        "xor rbp, rbp",
        "iretq",
        kstack   = in(reg) kernel_stack_top - 8,
        ss       = in(reg) USER_DATA_SEL,
        user_rsp = in(reg) user_rsp,
        rflags   = in(reg) rflags,
        cs       = in(reg) USER_CODE_SEL,
        entry    = in(reg) entry,
        options(noreturn)
    )
}

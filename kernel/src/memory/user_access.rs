//! User-pointer validation for the syscall layer.
//!
//! Syscalls receive raw virtual addresses from userspace. Before the kernel
//! dereferences one it must prove the whole range is a mapped *user* page:
//! the kernel identity-maps all physical RAM (heap, page tables, PMM bitmap)
//! into every address space, so a plain "below USER_SPACE_END" check is not
//! enough — a "user" address can alias kernel memory. We walk the live CR3
//! and require `PAGE_USER_ACCESSIBLE` on every leaf (plus `PAGE_WRITABLE`
//! for output buffers).

use crate::arch::x86_64::trap::current_cr3;
use crate::memory::paging::{
    PageTable, PageTableEntry, PAGE_HUGE, PAGE_USER_ACCESSIBLE, PAGE_WRITABLE,
    USER_SPACE_END,
};

/// Check that `[ptr, ptr + len)` is a fully-mapped user range under the
/// active page tables. `write` additionally requires every leaf page to be
/// writable (for kernel→user output buffers).
///
/// # Safety
/// Walks raw page tables via the kernel's identity map; safe to call from
/// syscall context (single-threaded per-CR3, no concurrent unmap exists).
pub unsafe fn user_range_ok(ptr: u64, len: u64, write: bool) -> bool {
    if len == 0 {
        // Zero-length transfers touch nothing; only reject null-ish ranges
        // for consistency with the non-empty case.
        return true;
    }
    // Reject the null page and anything that cannot be a user address.
    if ptr < 0x1000 {
        return false;
    }
    let Some(end) = ptr.checked_add(len) else {
        return false;
    };
    if end > USER_SPACE_END {
        return false;
    }

    let pml4_phys = current_cr3();
    if pml4_phys == 0 {
        return false;
    }

    let page = 0x1000u64;
    let mut va = ptr & !(page - 1);
    let last = (end - 1) & !(page - 1);
    while va <= last {
        if !page_present_user(va, write) {
            return false;
        }
        // `va + page` cannot overflow: `va <= last < end <= USER_SPACE_END`.
        va += page;
    }
    true
}

/// Walk the four-level table for `va`, requiring present+user leaves.
unsafe fn page_present_user(va: u64, write: bool) -> bool {
    let pml4 = current_cr3() as *const PageTable;
    let entry = |table: *const PageTable, idx: usize| -> PageTableEntry { (*table).entries[idx] };

    let p4e = entry(pml4, ((va >> 39) & 0x1ff) as usize);
    if !p4e.is_present() {
        return false;
    }
    let p3 = p4e.address() as *const PageTable;
    let p3e = entry(p3, ((va >> 30) & 0x1ff) as usize);
    if !p3e.is_present() {
        return false;
    }
    if p3e.flags() & PAGE_HUGE != 0 {
        return leaf_ok(&p3e, write);
    }
    let p2 = p3e.address() as *const PageTable;
    let p2e = entry(p2, ((va >> 21) & 0x1ff) as usize);
    if !p2e.is_present() {
        return false;
    }
    if p2e.flags() & PAGE_HUGE != 0 {
        return leaf_ok(&p2e, write);
    }
    let p1 = p2e.address() as *const PageTable;
    let p1e = entry(p1, ((va >> 12) & 0x1ff) as usize);
    if !p1e.is_present() {
        return false;
    }
    leaf_ok(&p1e, write)
}

/// A leaf (or huge) entry must be user-accessible; writes also need W.
fn leaf_ok(e: &PageTableEntry, write: bool) -> bool {
    let f = e.flags();
    if f & PAGE_USER_ACCESSIBLE == 0 {
        return false;
    }
    if write && f & PAGE_WRITABLE == 0 {
        return false;
    }
    true
}

/// Validate a user range for reading (`write = false`) or writing
/// (`write = true`), returning `false` on any violation.
pub fn access_ok(ptr: u64, len: u64, write: bool) -> bool {
    // SAFETY: read-only page-table walk of the active CR3; see module docs.
    unsafe { user_range_ok(ptr, len, write) }
}

//! Construction of the initial trap frames new tasks are started from.
//!
//! Every task — kernel thread or user process — is entered by restoring an
//! [`IrqFrame`] and executing `iretq`, exactly like a task that was
//! preempted.  That keeps a single code path for context switching: there is
//! no special "first run" logic anywhere in the scheduler.

use crate::arch::x86_64::trap::{
    IrqFrame, KERNEL_CODE_SEL, KERNEL_DATA_SEL, RFLAGS_IF, RFLAGS_RESERVED, USER_CODE_SEL,
    USER_DATA_SEL,
};
use core::arch::global_asm;

global_asm!(
    r#"
    // Entry stub every kernel thread starts at.  The initial trap frame is
    // built with r12 holding the thread entry function.
    .global blackwall_thread_trampoline
blackwall_thread_trampoline:
    sti
    // UEFI/AArch64-style (Win64) ABI: first argument in RCX, 32-byte shadow space.
    sub rsp, 32
    mov rcx, r12
    call blackwall_thread_entry_rust
    // The entry function never returns (it terminates the thread); park the
    // CPU in case that contract is ever broken.
9010:
    hlt
    jmp 9010b
"#
);

extern "C" {
    /// Assembly trampoline every kernel thread starts at (see `global_asm!`).
    pub fn blackwall_thread_trampoline();
}

/// Called from [`blackwall_thread_trampoline`] with the thread entry function
/// pointer that was stashed in `r12` of the initial frame.
#[no_mangle]
extern "C" fn blackwall_thread_entry_rust(entry: usize) {
    let entry: fn() = unsafe { core::mem::transmute(entry) };
    entry();

    // The thread function returned — terminate this task and let the
    // scheduler pick someone else.
    if let Some(mut guard) = crate::scheduler::SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            let pid = sched.current_pid;
            sched.terminate_task(pid);
        }
    }
    loop {
        crate::arch::x86_64::trap::yield_now();
    }
}

/// Build the initial frame for a **kernel thread**.
///
/// Restoring it lands in `blackwall_thread_trampoline` running in ring 0 with
/// a fresh stack.
///
/// # Safety
/// `stack_top` must point one past the end of a writable, live stack region of
/// at least [`initial_frame_size`] bytes below it.
pub unsafe fn init_kernel_frame(stack_top: u64, entry: fn()) -> u64 {
    let frame_ptr = stack_top - core::mem::size_of::<IrqFrame>() as u64;
    let frame = &mut *(frame_ptr as *mut IrqFrame);

    *frame = IrqFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: entry as *const () as usize as u64,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rdi: 0,
        rsi: 0,
        rbp: 0,
        rbx: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        err: 0,
        rip: blackwall_thread_trampoline as *const () as u64,
        cs: KERNEL_CODE_SEL,
        rflags: RFLAGS_IF | RFLAGS_RESERVED,
        rsp: stack_top,
        ss: KERNEL_DATA_SEL,
    };

    frame_ptr
}

/// Build the initial frame for a **user process**.
///
/// Restoring it executes `iretq` into ring 3 at `entry` with `user_rsp`.
///
/// # Safety
/// `kernel_stack_top` must point one past a live, writable kernel stack and
/// the target ring-3 address space must already be mapped in `cr3`.
pub unsafe fn init_user_frame(
    kernel_stack_top: u64,
    entry: u64,
    user_rsp: u64,
    arg: u64,
) -> u64 {
    let frame_ptr = kernel_stack_top - core::mem::size_of::<IrqFrame>() as u64;
    let frame = &mut *(frame_ptr as *mut IrqFrame);

    *frame = IrqFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rdi: arg,
        rsi: 0,
        rbp: 0,
        rbx: 0,
        rdx: 0,
        rcx: 0,
        rax: 0,
        err: 0,
        rip: entry,
        cs: USER_CODE_SEL,
        rflags: RFLAGS_IF | RFLAGS_RESERVED,
        rsp: user_rsp,
        ss: USER_DATA_SEL,
    };

    frame_ptr
}

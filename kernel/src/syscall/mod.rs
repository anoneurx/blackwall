//! System-call dispatch.
//!
//! The entry stub in [`crate::arch::x86_64::trap`] hands us the live trap
//! frame, so a syscall can read its arguments, write its return value and —
//! when it has to block — ask the scheduler for a different frame to resume.

use crate::arch::x86_64::trap::{self, IrqFrame};
use blackwall_shared::syscall::*;

pub mod fs;
pub mod process;

/// Dispatch the syscall described by the current frame's `rax`, leaving the
/// return value in the frame's `rax`.
///
/// Returns a replacement frame when the call put the task to sleep or
/// terminated it; `null` means "resume the caller".
pub fn dispatch(frame: *mut IrqFrame) -> *mut IrqFrame {
    let f = unsafe { &mut *frame };
    let number = f.rax;
    let a0 = f.rdi;
    let a1 = f.rsi;
    let a2 = f.rdx;

    let result = match number {
        SYS_READ => fs::sys_read(a0, a1, a2),
        SYS_WRITE => fs::sys_write(a0, a1, a2),
        SYS_OPEN => fs::sys_open(a0, a1),
        SYS_CLOSE => fs::sys_close(a0),

        SYS_EXIT => return process::sys_exit(frame, a0),
        SYS_FORK => process::sys_fork(frame),
        SYS_EXECVE => process::sys_execve(a0, a1, a2),

        SYS_GETPID => crate::scheduler::current_pid() as u64,
        SYS_GETPPID => process::sys_getppid(),
        SYS_YIELD => return trap::reschedule_current(frame, false),
        SYS_SLEEP => return process::sys_sleep(frame, a0),

        _ => ENOSYS as u64,
    };

    f.rax = result;
    core::ptr::null_mut()
}

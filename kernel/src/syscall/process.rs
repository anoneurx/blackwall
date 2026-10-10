//! Process-related system calls.

use crate::arch::x86_64::trap::{self, IrqFrame};
use crate::scheduler::SCHEDULER;
use blackwall_shared::syscall::*;

/// `fork` is not implemented yet — the kernel has no copy-on-write address
/// space support.  Returns `ENOSYS` instead of pretending to succeed.
pub fn sys_fork(_frame: *mut IrqFrame) -> u64 {
    ENOSYS as u64
}

/// `execve` will replace the current image once the ELF loader is exposed as a
/// syscall.  Until then it fails cleanly.
pub fn sys_execve(_path: u64, _argv: u64, _envp: u64) -> u64 {
    ENOSYS as u64
}

/// Parent PID of the current task (0 when there is no parent).
pub fn sys_getppid() -> u64 {
    match SCHEDULER.try_lock() {
        Some(guard) => guard
            .as_ref()
            .and_then(|s| s.manager().get(s.current_pid))
            .map(|p| p.parent as u64)
            .unwrap_or(0),
        None => 0,
    }
}

/// Terminate the calling task.
///
/// Returns the frame the kernel should switch to; never resumes the caller.
pub fn sys_exit(frame: *mut IrqFrame, status: u64) -> *mut IrqFrame {
    let pid = crate::scheduler::current_pid();
    crate::logging::print(format_args!("Task {} exited with status {}\n", pid, status));

    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            let pid = sched.current_pid;
            if let Some(pcb) = sched.manager_mut().get_mut(pid) {
                pcb.exit_status = status as i64;
            }
            sched.terminate_task(pid);
        }
    }

    unsafe { trap::kill_current_task(frame) }
}

/// Sleep for `ms` milliseconds.
///
/// Returns the frame to resume once the task is woken by the timer.
pub fn sys_sleep(frame: *mut IrqFrame, ms: u64) -> *mut IrqFrame {
    // 100 Hz PIT → 1 tick = 10 ms, round up so a non-zero request always
    // sleeps at least one tick.
    if ms == 0 {
        unsafe { (*frame).rax = 0 };
        return core::ptr::null_mut();
    }
    let duration_ticks = (ms + 9) / 10;
    let now = crate::arch::x86_64::timer::ticks();

    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            let pid = sched.current_pid;
            sched.sleep_task(pid, duration_ticks, now);
        }
    }

    // The return value is stored in the frame that will be restored when the
    // task wakes up.
    unsafe { (*frame).rax = 0 };
    unsafe { trap::block_current(frame) }
}

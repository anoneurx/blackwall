//! Process-related system calls.

use crate::arch::x86_64::trap::{self, IrqFrame};
use crate::process::state::ProcessState;
use crate::scheduler::SCHEDULER;
use blackwall_shared::syscall::*;

/// Duplicate the calling process.
///
/// The kernel has no copy-on-write support, so the address space is eagerly
/// deep-copied ([`crate::process::loader::clone_address_space`]): every user
/// page gets its own frame, while kernel/identity mappings stay shared.  The
/// child is enqueued ready to run and resumes from a copy of the caller's trap
/// frame with `rax = 0`; the parent returns the child PID.
pub fn sys_fork(frame: *mut IrqFrame) -> u64 {
    let parent_pid = crate::scheduler::current_pid();
    let parent_cr3 = trap::current_cr3();

    // Clone the parent's address space.
    let child_cr3 = {
        let mut guard = crate::memory::MEMORY.lock();
        let mem = match guard.as_mut() {
            Some(m) => m,
            None => return EAGAIN as u64,
        };
        unsafe {
            // SAFETY: MEMORY is published after boot and only used here /
            // in execve under this lock.
            crate::process::loader::clone_address_space(parent_cr3, &mut mem.physical)
        }
    };

    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return EAGAIN as u64,
    };
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return EAGAIN as u64,
    };

    // Copy the parent's name into a local buffer before mutating the manager.
    let mut name_buf = [0u8; 32];
    if let Some(parent) = sched.manager().get(parent_pid) {
        let len = parent.name.iter().position(|b| *b == 0).unwrap_or(32);
        let n = len.min(31);
        name_buf[..n].copy_from_slice(&parent.name[..n]);
    }
    let name_end = name_buf.iter().position(|b| *b == 0).unwrap_or(32);
    let name = core::str::from_utf8(&name_buf[..name_end]).unwrap_or("fork");

    let (child_pid, kstack_top) = sched.manager_mut().spawn_user(name, child_cr3, 0, 32 * 1024);

    // Build the child's initial frame at the top of its private kernel stack.
    let frame_size = core::mem::size_of::<IrqFrame>() as u64;
    let child_frame = kstack_top - frame_size;
    unsafe {
        // SAFETY: the manager just allocated and zeroed this kernel stack, so
        // `child_frame` is a live, writable IrqFrame slot.
        core::ptr::copy_nonoverlapping(
            frame as *const u8,
            child_frame as *mut u8,
            frame_size as usize,
        );
        let cf = &mut *(child_frame as *mut IrqFrame);
        cf.rax = 0; // fork returns 0 in the child
    }

    if let Some(pcb) = sched.manager_mut().get_mut(child_pid) {
        pcb.stack_ptr = child_frame;
        pcb.parent = parent_pid;
        pcb.state = ProcessState::Ready;
    }
    sched.enqueue_task(child_pid);

    child_pid as u64
}

/// Replace the calling process image with the ELF at `path`.
///
/// Loads the program into a brand-new address space, switches `CR3` and
/// rewrites the live trap frame so the `iretq` epilogue enters the new image
/// in ring 3.  The previous address space (and the old `CR3`) is abandoned.
pub fn sys_execve(frame: *mut IrqFrame, path_ptr: u64, _argv: u64, _envp: u64) -> u64 {
    // The address space this syscall started in; freed once the new image is
    // live.  Captured before any work: `read_path`/VFS reads walk it.
    let old_cr3 = trap::current_cr3() & !0xFFF_u64;

    let path = match crate::syscall::fs::read_path(path_ptr) {
        Some(p) => p,
        None => return ENOENT as u64,
    };

    let bytes = {
        let vfs = crate::fs::vfs::VFS.lock();
        let vfs_mgr = match vfs.as_ref() {
            Some(m) => m,
            None => return ENOENT as u64,
        };
        match vfs_mgr.read_all(&path) {
            Ok(b) => b,
            Err(_) => return ENOENT as u64,
        }
    };

    let image = {
        let mut guard = crate::memory::MEMORY.lock();
        let mem = match guard.as_mut() {
            Some(m) => m,
            None => return EAGAIN as u64,
        };
        unsafe {
            // SAFETY: the physical allocator is exclusively held here.
            let p4 = crate::process::loader::create_address_space(&mut mem.physical);
            crate::process::loader::build_user_image_into(&bytes, p4, &mut mem.physical)
        }
    };

    unsafe {
        // SAFETY: `image.cr3` is a fresh page-table root; the caller resumes
        // through the trap epilogue, so the new user mappings must be live now.
        trap::switch_cr3(image.cr3);
        let f = &mut *frame;
        f.rip = image.entry;
        f.rsp = image.user_rsp;
        f.rax = 0;
    }

    // Keep the PCB's CR3 in sync so later scheduler switches restore it.
    if let Some(mut g) = SCHEDULER.try_lock() {
        if let Some(s) = g.as_mut() {
            let pid = s.current_pid;
            if let Some(pcb) = s.manager_mut().get_mut(pid) {
                pcb.cr3 = image.cr3;
            }
        }
    }

    // The old address space is no longer in use: free its private tables and
    // user pages.  `destroy_address_space` only touches OWNED frames and
    // refuses the boot root, so shared kernel mappings survive.
    {
        let mut guard = crate::memory::MEMORY.lock();
        if let Some(mem) = guard.as_mut() {
            unsafe {
                // SAFETY: CR3 was switched above, so `old_cr3` is inactive;
                // the physical allocator is exclusively held here.
                crate::process::loader::destroy_address_space(old_cr3, &mut mem.physical);
            }
        }
    }

    0
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

/// Wait for a child process to terminate.
///
/// `pid` is the PID to wait for (0 means any child).  `status_ptr`, when
/// non-zero, receives the child's exit status.  Returns the PID of the
/// terminated child.  Blocks the caller when children exist but none has
/// exited yet, and returns `ECHILD` when the caller has no matching child.
pub fn sys_waitpid(frame: *mut IrqFrame, pid: u64, status_ptr: u64) -> *mut IrqFrame {
    let me = crate::scheduler::current_pid();
    let want = pid as u16;

    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => {
            unsafe { (*frame).rax = EAGAIN as u64 };
            return core::ptr::null_mut();
        }
    };
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => {
            unsafe { (*frame).rax = EAGAIN as u64 };
            return core::ptr::null_mut();
        }
    };

    // 1. Is there an already-exited matching child?  `reap_zombies` frees the
    //    kernel stack and flips Zombie → Terminated, so accept either state.
    let found = sched
        .manager()
        .iter()
        .find(|c| {
            c.parent == me
                && (want == 0 || c.pid == want)
                && matches!(c.state, ProcessState::Zombie | ProcessState::Terminated)
        })
        .map(|c| (c.pid, c.exit_status, c.state));

    if let Some((child, status, child_state)) = found {
        if status_ptr != 0 && crate::memory::user_access::access_ok(status_ptr, 8, true) {
            unsafe { core::ptr::write_unaligned(status_ptr as *mut u64, status as u64) };
        }
        if child_state == ProcessState::Zombie {
            unsafe { sched.manager_mut().free_stack(child) };
        }
        sched.manager_mut().remove(child);
        unsafe { (*frame).rax = child as u64 };
        return core::ptr::null_mut();
    }

    // 2. No matching child at all → ECHILD.
    let has_children =
        sched.manager().iter().any(|c| c.parent == me && (want == 0 || c.pid == want));
    if !has_children {
        unsafe { (*frame).rax = ECHILD as u64 };
        return core::ptr::null_mut();
    }

    // 3. Children exist but none exited: block until one does.
    if let Some(pcb) = sched.manager_mut().get_mut(me) {
        pcb.state = ProcessState::Blocked;
        pcb.wait_child = want;
        pcb.wait_status_ptr = status_ptr;
    }
    let _ = sched;
    // Release the scheduler lock before rescheduling: `block_current` takes it.
    drop(guard);
    unsafe {
        // SAFETY: the caller marked itself Blocked and removed itself from the
        // run queue; the scheduler will wake it from the timer tick.
        (*frame).rax = 0;
        trap::block_current(frame)
    }
}

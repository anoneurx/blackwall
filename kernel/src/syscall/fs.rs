extern crate alloc;

use crate::arch::x86_64::serial;
use crate::fs::vfs::{OpenFile, VFS};
use crate::scheduler::SCHEDULER;
use alloc::sync::Arc;
use blackwall_shared::syscall::*;
use core::slice;

// ── sys_read ────────────────────────────────────────────────────────────────
pub fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    // Get the open file from the current process's fd table.
    let open_file = {
        let sched = SCHEDULER.lock();
        let s = match sched.as_ref() {
            Some(s) => s,
            None => return ENOSYS as u64,
        };
        let pcb = match s.manager().get(s.current_pid) {
            Some(p) => p,
            None => return ENOSYS as u64,
        };
        pcb.get_fd(fd).map(|f| f.clone())
    };

    // fd 0 (stdin) reads from the serial port.
    if fd == 0 {
        // SAFETY: The userspace syscall ABI guarantees that `buf_ptr` is a
        // valid, non-null, writable region of at least `len` bytes within the
        // calling process's address space. The kernel does not dereference
        // this pointer after returning to userspace, preventing use-after-free.
        let buf = unsafe { slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
        let mut read_len = 0;
        while (read_len as u64) < len {
            if let Some(mut b) = serial::read_byte() {
                if b == b'\r' {
                    b = b'\n';
                }
                buf[read_len] = b;
                read_len += 1;

                // Echo back
                let echo_buf = [b];
                if let Ok(s) = core::str::from_utf8(&echo_buf) {
                    serial::write_str(s);
                }

                if b == b'\n' {
                    break;
                }
            } else {
                // No byte yet: let the timer tick (and other tasks) run while
                // we wait instead of spinning with interrupts disabled.
                unsafe {
                    crate::arch::x86_64::interrupts::enable();
                    core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
                    crate::arch::x86_64::interrupts::disable();
                }
            }
        }
        return read_len as u64;
    }

    let open_file = match open_file {
        Some(f) => f,
        None => return u64::MAX, // EBADF
    };

    // SAFETY: Same ABI guarantee as the stdin branch above — `buf_ptr` is a
    // caller-validated writable buffer of `len` bytes. The VFS read function
    // only writes within `[0, len)` and does not retain the slice reference.
    let buf = unsafe { slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    let offset = *open_file.offset.lock();
    match open_file.vnode.fs.read(open_file.vnode.inode, buf, offset) {
        Ok(n) => {
            *open_file.offset.lock() = offset + n as u64;
            n as u64
        }
        Err(_) => ENOSYS as u64,
    }
}

// ── sys_write ───────────────────────────────────────────────────────────────
pub fn sys_write(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    // fd 1 (stdout) and fd 2 (stderr) go straight to the serial port.
    if fd == 1 || fd == 2 {
        // SAFETY: `buf_ptr` points to a readable userspace buffer of `len`
        // bytes. The syscall ABI contract requires that the caller ensures
        // alignment (u8, so alignment = 1) and lifetime validity for the
        // duration of the syscall. The slice is only read, not written.
        let buf = unsafe { slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
        if let Ok(s) = core::str::from_utf8(buf) {
            serial::write_str(s);
            return len;
        }
        return ENOSYS as u64;
    }

    // For other fds, delegate to the VFS write path.
    let open_file = {
        let sched = SCHEDULER.lock();
        let s = match sched.as_ref() {
            Some(s) => s,
            None => return ENOSYS as u64,
        };
        let pcb = match s.manager().get(s.current_pid) {
            Some(p) => p,
            None => return ENOSYS as u64,
        };
        pcb.get_fd(fd).map(|f| f.clone())
    };

    let open_file = match open_file {
        Some(f) => f,
        None => return u64::MAX, // EBADF
    };

    // SAFETY: `buf_ptr` is a readable userspace buffer of `len` bytes per
    // the syscall ABI. The VFS write function only reads within `[0, len)`
    // and does not retain the slice after returning.
    let buf = unsafe { slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    let offset = *open_file.offset.lock();
    match open_file.vnode.fs.write(open_file.vnode.inode, buf, offset) {
        Ok(n) => {
            *open_file.offset.lock() = offset + n as u64;
            n as u64
        }
        Err(_) => ENOSYS as u64,
    }
}

// ── sys_open ────────────────────────────────────────────────────────────────
pub fn sys_open(path_ptr: u64, _flags: u64) -> u64 {
    // Read the null-terminated path from userspace.
    // SAFETY: `path_ptr` is a userspace pointer to a null-terminated UTF-8
    // string. We scan at most 512 bytes forward to find the null terminator,
    // preventing unbounded reads. The resulting slice lives only within
    // this function's scope and is never stored past the VFS resolve call.
    let path = unsafe {
        let mut len = 0;
        let ptr = path_ptr as *const u8;
        while *ptr.add(len) != 0 && len < 512 {
            len += 1;
        }
        core::str::from_utf8(slice::from_raw_parts(ptr, len)).unwrap_or("")
    };

    if path.is_empty() {
        return ENOSYS as u64;
    }

    // Resolve the path in the VFS.
    let vnode = {
        let vfs = VFS.lock();
        match vfs.as_ref() {
            Some(mgr) => match mgr.resolve_path(path) {
                Ok(vn) => vn,
                Err(_) => return u64::MAX, // ENOENT
            },
            None => return ENOSYS as u64,
        }
    };

    let open_file = OpenFile {
        vnode: Arc::new(vnode),
        offset: Arc::new(crate::sync::spin::SpinLock::new(0u64)),
    };

    // Install the open file into the current process's fd table.
    let mut sched = SCHEDULER.lock();
    let s = match sched.as_mut() {
        Some(s) => s,
        None => return ENOSYS as u64,
    };
    let pid = s.current_pid;
    let pcb = match s.manager_mut().get_mut(pid) {
        Some(p) => p,
        None => return ENOSYS as u64,
    };
    match pcb.alloc_fd(open_file) {
        Ok(fd) => fd,
        Err(_) => ENOSYS as u64,
    }
}

// ── sys_close ───────────────────────────────────────────────────────────────
pub fn sys_close(fd: u64) -> u64 {
    let mut sched = SCHEDULER.lock();
    let s = match sched.as_mut() {
        Some(s) => s,
        None => return ENOSYS as u64,
    };
    let pid = s.current_pid;
    let pcb = match s.manager_mut().get_mut(pid) {
        Some(p) => p,
        None => return ENOSYS as u64,
    };
    match pcb.close_fd(fd) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;

use crate::arch::x86_64::serial;
use crate::fs::vfs::{OpenFile, VFS, VfsError, VnodeType};
use crate::scheduler::SCHEDULER;
use blackwall_shared::syscall::*;
use core::slice;

/// Map a [`VfsError`] to a negative errno (as an unsigned return value).
fn err_of(e: VfsError) -> u64 {
    let code: i64 = match e {
        VfsError::FileNotFound | VfsError::InvalidPath => ENOENT,
        VfsError::AlreadyExists => EEXIST,
        VfsError::NotADirectory => ENOTDIR,
        VfsError::NotSupported | VfsError::IsADirectory => ENOTSUP,
        _ => ENOSYS,
    };
    code as u64
}

/// Read a NUL-terminated path from userspace (bounded at 511 bytes).
///
/// # Safety
/// `ptr` must point at a readable userspace buffer; we only scan forward until
/// the NUL terminator or the 511-byte cap, never past the caller's guarantee.
fn read_path(ptr: u64) -> Option<String> {
    if ptr == 0 {
        return None;
    }
    let mut buf = [0u8; 512];
    let mut len = 0usize;
    // SAFETY: same userspace ABI guarantee as the rest of the syscall layer —
    // the caller promises a valid NUL-terminated string; we bound the scan.
    unsafe {
        let p = ptr as *const u8;
        while len < 511 {
            let b = *p.add(len);
            if b == 0 {
                break;
            }
            buf[len] = b;
            len += 1;
        }
    }
    core::str::from_utf8(&buf[..len]).ok().map(String::from)
}

fn vtype_to_dt(vtype: VnodeType) -> u8 {
    match vtype {
        VnodeType::Directory => DT_DIR,
        VnodeType::Symlink => DT_SYMLINK,
        _ => DT_REG,
    }
}

/// Get the open file for `fd` of the current process.
fn current_open_file(fd: u64) -> Option<OpenFile> {
    let sched = SCHEDULER.lock();
    let s = sched.as_ref()?;
    let pcb = s.manager().get(s.current_pid)?;
    pcb.get_fd(fd).cloned()
}

// ── sys_read ────────────────────────────────────────────────────────────────
pub fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
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

    let open_file = match current_open_file(fd) {
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

    let open_file = match current_open_file(fd) {
        Some(f) => f,
        None => return u64::MAX, // EBADF
    };

    // SAFETY: `buf_ptr` is a readable userspace buffer of `len` bytes per
    // the syscall ABI. The VFS write function only reads within `[0, len)`
    // and does not retain the slice after returning.
    let buf = unsafe { slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
    let mut offset = *open_file.offset.lock();
    if open_file.flags & O_APPEND != 0 {
        offset = open_file.vnode.size;
    }
    match open_file.vnode.fs.write(open_file.vnode.inode, buf, offset) {
        Ok(n) => {
            *open_file.offset.lock() = offset + n as u64;
            n as u64
        }
        Err(_) => ENOSYS as u64,
    }
}

// ── sys_open ────────────────────────────────────────────────────────────────
pub fn sys_open(path_ptr: u64, flags: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }

    let vnode = {
        let vfs = VFS.lock();
        let mgr = match vfs.as_ref() {
            Some(m) => m,
            None => return ENOSYS as u64,
        };
        let trunc = flags & O_TRUNC != 0;

        if flags & O_CREAT != 0 {
            match mgr.resolve_path(&path) {
                Ok(_) if trunc => {
                    let _ = mgr.unlink(&path);
                    if let Err(e) = mgr.create_file(&path) {
                        return err_of(e);
                    }
                }
                Err(_) => {
                    if let Err(e) = mgr.create_file(&path) {
                        return err_of(e);
                    }
                }
                _ => {}
            }
        } else {
            if let Err(e) = mgr.resolve_path(&path) {
                return err_of(e);
            }
            if trunc {
                let _ = mgr.unlink(&path);
                let _ = mgr.create_file(&path);
            }
        }

        match mgr.resolve_path(&path) {
            Ok(vn) => vn,
            Err(e) => return err_of(e),
        }
    };

    let initial_offset = if flags & O_APPEND != 0 { vnode.size } else { 0 };
    let open_file = OpenFile {
        vnode: Arc::new(vnode),
        offset: Arc::new(crate::sync::spin::SpinLock::new(initial_offset)),
        flags,
    };

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

// ── sys_lseek ───────────────────────────────────────────────────────────────
pub fn sys_lseek(fd: u64, offset: u64, whence: u64) -> u64 {
    let open_file = match current_open_file(fd) {
        Some(f) => f,
        None => return u64::MAX, // EBADF
    };
    let mut off = open_file.offset.lock();
    let size = open_file.vnode.size;
    let new = match whence {
        SEEK_SET => offset,
        SEEK_CUR => offset.wrapping_add(*off),
        SEEK_END => size.wrapping_add(offset),
        _ => return u64::MAX, // EINVAL
    };
    *off = new;
    new
}

// ── sys_mkdir ───────────────────────────────────────────────────────────────
pub fn sys_mkdir(path_ptr: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }
    let vfs = VFS.lock();
    match vfs.as_ref() {
        Some(mgr) => match mgr.mkdir(&path) {
            Ok(_) => 0,
            Err(e) => err_of(e),
        },
        None => ENOSYS as u64,
    }
}

// ── sys_unlink ──────────────────────────────────────────────────────────────
pub fn sys_unlink(path_ptr: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }
    let vfs = VFS.lock();
    match vfs.as_ref() {
        Some(mgr) => match mgr.unlink(&path) {
            Ok(()) => 0,
            Err(e) => err_of(e),
        },
        None => ENOSYS as u64,
    }
}

// ── sys_rmdir ───────────────────────────────────────────────────────────────
pub fn sys_rmdir(path_ptr: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }
    let vfs = VFS.lock();
    match vfs.as_ref() {
        Some(mgr) => match mgr.rmdir(&path) {
            Ok(()) => 0,
            Err(e) => err_of(e),
        },
        None => ENOSYS as u64,
    }
}

// ── sys_readdir ─────────────────────────────────────────────────────────────
pub fn sys_readdir(path_ptr: u64, buf_ptr: u64, len: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }

    let entries = {
        let vfs = VFS.lock();
        match vfs.as_ref() {
            Some(mgr) => match mgr.readdir(&path) {
                Ok(e) => e,
                Err(e) => return err_of(e),
            },
            None => return ENOSYS as u64,
        }
    };

    // SAFETY: `buf_ptr` is a caller-validated writable buffer of `len` bytes.
    let buf = unsafe { slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
    let mut written = 0usize;
    for entry in entries {
        let name = entry.name.as_bytes();
        if name.is_empty() || name.len() > 255 {
            continue;
        }
        if written + 2 + name.len() > buf.len() {
            break;
        }
        buf[written] = vtype_to_dt(entry.vtype);
        buf[written + 1] = name.len() as u8;
        buf[written + 2..written + 2 + name.len()].copy_from_slice(name);
        written += 2 + name.len();
    }
    written as u64
}

// ── sys_stat ────────────────────────────────────────────────────────────────
pub fn sys_stat(path_ptr: u64, size_ptr: u64, type_ptr: u64) -> u64 {
    let Some(path) = read_path(path_ptr) else { return ENOSYS as u64; };
    if path.is_empty() {
        return ENOSYS as u64;
    }

    let (size, vtype) = {
        let vfs = VFS.lock();
        match vfs.as_ref() {
            Some(mgr) => match mgr.stat(&path) {
                Ok(x) => x,
                Err(e) => return err_of(e),
            },
            None => return ENOSYS as u64,
        }
    };

    // SAFETY: `size_ptr` / `type_ptr` are caller-validated writable slots.
    unsafe {
        if size_ptr != 0 {
            core::ptr::write_unaligned(size_ptr as *mut u64, size);
        }
        if type_ptr != 0 {
            core::ptr::write_unaligned(type_ptr as *mut u8, vtype_to_dt(vtype));
        }
    }
    0
}
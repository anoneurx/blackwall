extern crate alloc;

use super::state::ProcessState;
use crate::fs::vfs::{OpenFile, VfsError};
use alloc::vec::Vec;

/// Default kernel thread stack size: 64 KiB.
pub const DEFAULT_STACK_SIZE: usize = 64 * 1024;
/// Maximum number of open files per process.
pub const MAX_FDS: usize = 64;
/// FDs 0 (stdin), 1 (stdout) and 2 (stderr) are handled directly by the
/// syscall layer and are never handed out by [`ProcessControlBlock::alloc_fd`].
pub const FD_RESERVED: usize = 3;

/// Represents a file descriptor entry.  `None` = slot is free.
pub type FdTable = Vec<Option<OpenFile>>;

/// Process Control Block — the complete description of a kernel task.
///
/// The heap-allocated stack and file descriptor table are tracked by raw
/// pointer / Vec; the `ProcessManager` is responsible for freeing them.
pub struct ProcessControlBlock {
    /// Unique process identifier.  PID 0 is always the idle task.
    pub pid: u16,
    /// Human-readable name stored as a fixed-length ASCII byte array.
    pub name: [u8; 32],
    /// Current lifecycle state.
    pub state: ProcessState,
    /// Scheduling priority (0 = lowest / idle, 255 = highest).
    pub priority: u8,
    /// Saved stack pointer (`rsp`), updated on every context switch.
    pub stack_ptr: u64,
    /// Bottom of the heap-allocated stack region.
    pub stack_base: *mut u8,
    /// Size in bytes of the heap-allocated stack region.
    pub stack_size: usize,
    /// Total PIT ticks this task has been Running.
    pub ticks_run: u64,
    /// `ticks()` value at which a Sleeping task should be woken.
    pub sleep_until: u64,
    /// Per-process open file descriptor table.
    pub fd_table: FdTable,
    /// Physical address of this task's page table (0 = boot page table).
    pub cr3: u64,
    /// Parent PID (0 for kernel tasks and init).
    pub parent: u16,
    /// Exit status recorded by `exit`/`wait`.
    pub exit_status: i64,
}

// SAFETY: PCBs are only mutated while holding the global scheduler lock.
unsafe impl Send for ProcessControlBlock {}
unsafe impl Sync for ProcessControlBlock {}

impl ProcessControlBlock {
    /// Create a PCB with the given identity fields; the caller must
    /// separately allocate a stack and set `stack_base`, `stack_size`,
    /// and `stack_ptr` before enqueuing the task.
    pub fn new(pid: u16, name: &[u8], priority: u8) -> Self {
        let mut name_buf = [0u8; 32];
        let copy_len = name.len().min(31);
        name_buf[..copy_len].copy_from_slice(&name[..copy_len]);

        // Pre-allocate MAX_FDS slots; all start as None (closed).
        let mut fd_table = Vec::with_capacity(MAX_FDS);
        for _ in 0..MAX_FDS {
            fd_table.push(None);
        }

        Self {
            pid,
            name: name_buf,
            state: ProcessState::Ready,
            priority,
            stack_ptr: 0,
            stack_base: core::ptr::null_mut(),
            stack_size: 0,
            ticks_run: 0,
            sleep_until: 0,
            fd_table,
            cr3: 0,
            parent: 0,
            exit_status: 0,
        }
    }

    /// Top of this task's private kernel stack (0 when the task has none —
    /// the boot context, which never returns to ring 3 and never syscalls).
    pub fn kernel_stack_top(&self) -> u64 {
        if self.stack_base.is_null() || self.stack_size == 0 {
            0
        } else {
            self.stack_base as u64 + self.stack_size as u64
        }
    }

    /// Allocate the next free file descriptor and install `file` there.
    /// Returns the fd number or `Err` if the table is full.
    pub fn alloc_fd(&mut self, file: OpenFile) -> Result<u64, VfsError> {
        // Skip the reserved stdin/stdout/stderr slots so the first real file
        // gets fd 3 — handing out fd 0 would make `open` collide with stdin.
        for (i, slot) in self.fd_table.iter_mut().enumerate().skip(FD_RESERVED) {
            if slot.is_none() {
                *slot = Some(file);
                return Ok(i as u64);
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Close a file descriptor.  Returns `Err` if the fd was not open.
    pub fn close_fd(&mut self, fd: u64) -> Result<(), VfsError> {
        let slot = self.fd_table.get_mut(fd as usize).ok_or(VfsError::FileNotFound)?;
        if slot.is_none() {
            return Err(VfsError::FileNotFound);
        }
        *slot = None;
        Ok(())
    }

    /// Get a reference to an open file by fd.
    pub fn get_fd(&self, fd: u64) -> Option<&OpenFile> {
        self.fd_table.get(fd as usize)?.as_ref()
    }

    /// Returns the task name as a `&str`, trimming the null terminator.
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|b| *b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("?")
    }

    /// Print a one-line summary of this PCB to serial.
    pub fn log(&self) {
        crate::logging::print(format_args!(
            "PID {:5}  {:32}  {:11}  pri={}\n",
            self.pid,
            self.name_str(),
            self.state.as_str(),
            self.priority,
        ));
    }
}

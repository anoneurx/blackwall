extern crate alloc;

use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use alloc::vec::Vec;

use super::pcb::ProcessControlBlock;
use super::state::ProcessState;
use crate::scheduler::context_switch;

/// Maximum supported PID value.  Wraps back to 1 after reaching this limit.
/// PID 0 is permanently reserved for the idle task.
pub const MAX_PID: u16 = u16::MAX;

/// Manages the full set of process control blocks.
///
/// The `ProcessManager` is the single owner of every PCB and of the
/// heap-allocated stacks for each thread.  It does **not** manage the
/// run queue — that is the scheduler's responsibility.
pub struct ProcessManager {
    processes: Vec<ProcessControlBlock>,
    next_pid: u16,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self { processes: Vec::new(), next_pid: 0 }
    }

    /// Spawn a new kernel thread.
    ///
    /// Allocates a `stack_size`-byte stack from the kernel heap, initialises
    /// it so the first context switch will begin executing `entry`, and
    /// inserts a PCB in the `Ready` state.
    ///
    /// Returns the new PID.
    pub fn spawn(&mut self, name: &str, entry: fn(), stack_size: usize, priority: u8) -> u16 {
        let pid = self.allocate_pid();
        let mut pcb = ProcessControlBlock::new(pid, name.as_bytes(), priority);

        // Allocate and zero-initialise the thread stack.
        let layout = Layout::from_size_align(stack_size, 16).expect("invalid stack layout");
        let stack_base = unsafe {
            // SAFETY: layout is non-zero and 16-byte aligned.
            alloc_zeroed(layout)
        };
        assert!(!stack_base.is_null(), "kernel stack allocation failed");

        // Calculate the top of the stack (stacks grow downward on x86_64).
        let stack_top = unsafe { stack_base.add(stack_size) };

        // Initialise the stack so a context switch will jump into `entry`.
        let initial_rsp = unsafe {
            // SAFETY: stack_top is valid and the stack was just allocated.
            context_switch::init_stack(stack_top, entry)
        };

        pcb.stack_base = stack_base;
        pcb.stack_size = stack_size;
        pcb.stack_ptr = initial_rsp;
        pcb.state = ProcessState::Ready;

        self.processes.push(pcb);
        pid
    }

    /// Return an immutable reference to a PCB by PID.
    pub fn get(&self, pid: u16) -> Option<&ProcessControlBlock> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    /// Return a mutable reference to a PCB by PID.
    pub fn get_mut(&mut self, pid: u16) -> Option<&mut ProcessControlBlock> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    /// Release the stack memory for a terminated task.
    ///
    /// # Safety
    ///
    /// The caller must guarantee the task is no longer executing and
    /// that its stack pointer will never be loaded again.
    pub unsafe fn free_stack(&mut self, pid: u16) {
        if let Some(pcb) = self.get_mut(pid) {
            if !pcb.stack_base.is_null() && pcb.stack_size > 0 {
                let layout = Layout::from_size_align(pcb.stack_size, 16)
                    .expect("invalid stack layout on free");
                // SAFETY: Upheld by the caller contract.
                dealloc(pcb.stack_base, layout);
                pcb.stack_base = core::ptr::null_mut();
                pcb.stack_size = 0;
            }
        }
    }

    /// Allocate the next available PID.
    fn allocate_pid(&mut self) -> u16 {
        let pid = self.next_pid;
        // Advance, wrapping at MAX_PID and skipping 0 for non-idle spawns.
        self.next_pid = match self.next_pid {
            MAX_PID => 1,
            n => n + 1,
        };
        pid
    }

    /// Iterate over all PCBs (read-only).
    pub fn iter(&self) -> impl Iterator<Item = &ProcessControlBlock> {
        self.processes.iter()
    }

    /// Total number of tracked processes.
    pub fn count(&self) -> usize {
        self.processes.len()
    }
}

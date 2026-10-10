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

        // Initialise the stack so the first restore lands in the thread
        // trampoline with `entry` stashed in the frame.
        let initial_rsp = unsafe {
            // SAFETY: stack_top is valid and the stack was just allocated.
            context_switch::init_kernel_frame(stack_top as u64, entry)
        };

        pcb.stack_base = stack_base;
        pcb.stack_size = stack_size;
        pcb.stack_ptr = initial_rsp;
        pcb.cr3 = crate::arch::x86_64::trap::current_cr3();
        pcb.state = ProcessState::Ready;

        self.processes.push(pcb);
        pid
    }

    /// Register the context that is *already* executing (the boot flow) so the
    /// scheduler can save and restore it like any other task.
    ///
    /// The boot stack belongs to the firmware and is never freed, hence the
    /// null `stack_base`.
    pub fn register_running(&mut self, name: &str, cr3: u64) -> u16 {
        let pid = self.allocate_pid();
        let mut pcb = ProcessControlBlock::new(pid, name.as_bytes(), 255);
        pcb.cr3 = cr3;
        pcb.state = ProcessState::Running;
        self.processes.push(pcb);
        pid
    }

    /// Create the PCB for a user process that will be started from a
    /// pre-built trap frame.
    ///
    /// Returns `(pid, kernel_stack_top)` — the frame must be built at
    /// `kernel_stack_top - size_of::<IrqFrame>()`.
    pub fn spawn_user(
        &mut self,
        name: &str,
        cr3: u64,
        priority: u8,
        kernel_stack_size: usize,
    ) -> (u16, u64) {
        let pid = self.allocate_pid();
        let mut pcb = ProcessControlBlock::new(pid, name.as_bytes(), priority);

        let layout =
            Layout::from_size_align(kernel_stack_size, 16).expect("invalid kernel stack layout");
        let stack_base = unsafe {
            // SAFETY: layout is non-zero and 16-byte aligned.
            alloc_zeroed(layout)
        };
        assert!(!stack_base.is_null(), "kernel stack allocation failed");
        let stack_top = unsafe { stack_base.add(kernel_stack_size) as u64 };

        pcb.stack_base = stack_base;
        pcb.stack_size = kernel_stack_size;
        pcb.cr3 = cr3;
        pcb.state = ProcessState::Ready;

        self.processes.push(pcb);
        (pid, stack_top)
    }

    /// Free the stacks of terminated tasks.  Called from the timer tick.
    pub unsafe fn reap_zombies(&mut self) {
        let mut i = 0;
        while i < self.processes.len() {
            if self.processes[i].state == ProcessState::Zombie {
                self.free_stack(self.processes[i].pid);
                self.processes[i].state = ProcessState::Terminated;
            }
            i += 1;
        }
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

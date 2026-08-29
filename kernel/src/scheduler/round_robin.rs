use crate::arch::x86_64::serial;
use crate::process::manager::ProcessManager;
use crate::process::pcb::DEFAULT_STACK_SIZE;
use crate::process::state::ProcessState;
use crate::scheduler::{context_switch, queue::RunQueue};

/// Number of PIT ticks per scheduling quantum.
///
/// At 100 Hz (10 ms/tick) this gives ~100 ms time slices.
/// Reduce for finer-grained preemption; increase for lower overhead.
pub const TIME_SLICE_TICKS: u64 = 10;

/// The round-robin scheduler.
///
/// Owns the process table and the run queue.  The timer ISR calls `tick()`
/// on every PIT interrupt; the scheduler calls `schedule()` once per
/// quantum boundary.
///
/// # Single-CPU note
///
/// This implementation assumes a single CPU.  `current_pid` is the only
/// notion of "what is running now".  SMP would require per-CPU run queues
/// and IPI-based migrations.
pub struct RoundRobinScheduler {
    manager: ProcessManager,
    queue: RunQueue,
    /// PID of the task currently marked Running.
    pub current_pid: u16,
    /// Whether the scheduler has been fully started.
    started: bool,
}

impl RoundRobinScheduler {
    pub fn new() -> Self {
        Self {
            manager: ProcessManager::new(),
            queue: RunQueue::new(),
            current_pid: 0,
            started: false,
        }
    }

    // -----------------------------------------------------------------------
    // Public interface used by the timer ISR
    // -----------------------------------------------------------------------

    /// Called from the PIT ISR on every timer tick.
    ///
    /// Responsibilities:
    /// 1. Wake sleeping tasks whose deadline has passed.
    /// 2. Increment the running task's tick count.
    /// 3. Trigger a `schedule()` call at each quantum boundary.
    pub fn tick(&mut self, current_ticks: u64) {
        if !self.started {
            return;
        }

        // Wake any sleeping tasks whose deadline has been reached.
        self.wake_sleeping(current_ticks);

        // Charge one tick to the current task.
        if let Some(pcb) = self.manager.get_mut(self.current_pid) {
            if pcb.state == ProcessState::Running {
                pcb.ticks_run = pcb.ticks_run.saturating_add(1);
            }
        }

        // Preempt at quantum boundaries.
        if current_ticks % TIME_SLICE_TICKS == 0 {
            self.schedule();
        }
    }

    // -----------------------------------------------------------------------
    // Public interface used by thread helpers
    // -----------------------------------------------------------------------

    /// Spawn a new kernel thread and enqueue it.
    ///
    /// Returns the assigned PID.
    pub fn spawn_kernel_thread(&mut self, name: &str, entry: fn(), priority: u8) -> u16 {
        let pid = self.manager.spawn(name, entry, DEFAULT_STACK_SIZE, priority);
        self.queue.enqueue(pid);
        pid
    }

    /// Mark a task as Sleeping until `current_ticks + duration`.
    /// Removes it from the run queue so it won't be selected.
    pub fn sleep_task(&mut self, pid: u16, duration_ticks: u64, current_ticks: u64) {
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Sleeping;
            pcb.sleep_until = current_ticks.saturating_add(duration_ticks);
        }
        self.queue.remove(pid);
    }

    /// Forcefully terminate a task: mark as Zombie, remove from run queue.
    /// The caller is responsible for eventually freeing the stack via
    /// `free_zombie_stack`.
    pub fn terminate_task(&mut self, pid: u16) {
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Zombie;
        }
        self.queue.remove(pid);
    }

    /// Free the heap-allocated stack of a Zombie task.
    ///
    /// # Safety
    ///
    /// The task must not be executing and must be in the Zombie state.
    pub unsafe fn free_zombie_stack(&mut self, pid: u16) {
        self.manager.free_stack(pid);
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Terminated;
        }
    }

    /// Immutable access to the process manager (for inspection / logging).
    pub fn manager(&self) -> &ProcessManager {
        &self.manager
    }

    /// Mutable access to the process manager (for fd table operations).
    pub fn manager_mut(&mut self) -> &mut ProcessManager {
        &mut self.manager
    }

    // -----------------------------------------------------------------------
    // Entry point — called once after spawning idle + init tasks
    // -----------------------------------------------------------------------

    /// Start the scheduler by running the idle task.
    ///
    /// This function never returns.  It marks the idle task (PID 0) as
    /// Running, sets the `started` flag so timer ticks are honoured, and
    /// then drops into a fake "previous context" that jumps straight to
    /// the idle task's stack.
    pub fn run(&mut self) -> ! {
        serial::line("Scheduler Started");

        // Mark PID 0 (idle) as the initial running task.
        if let Some(pcb) = self.manager.get_mut(0) {
            pcb.state = ProcessState::Running;
        }
        self.queue.remove(0); // idle is Running, not in the ready queue.

        self.started = true;

        // We jump to the idle task's stack by loading its rsp and returning
        // into it.  We use a dummy `old_rsp` location on the current
        // (boot) stack since we will never return to this stack frame again.
        let idle_rsp = self.manager.get(0).map(|p| p.stack_ptr).unwrap_or(0);
        let mut dummy_rsp: u64 = 0;

        // SAFETY: idle_rsp was initialised by `init_stack` and is valid.
        unsafe {
            context_switch::switch_to(&mut dummy_rsp as *mut u64, idle_rsp);
        }

        // Unreachable, but the type system needs a `!` return.
        loop {
            context_switch::cpu_relax();
        }
    }

    // -----------------------------------------------------------------------
    // Internal scheduling logic
    // -----------------------------------------------------------------------

    /// Select the next ready task and switch to it.
    ///
    /// If the run queue is empty, keeps running the current task (or idle).
    pub fn schedule(&mut self) {
        // Try to pick the next task from the run queue.
        let Some(next_pid) = self.queue.dequeue() else {
            // Nothing else to run — keep the current task going.
            // If the current task somehow disappeared, fall back to idle.
            return;
        };

        let old_pid = self.current_pid;

        // Re-enqueue the old task if it is still runnable.
        if old_pid != next_pid {
            if let Some(old_pcb) = self.manager.get_mut(old_pid) {
                if old_pcb.state == ProcessState::Running {
                    old_pcb.state = ProcessState::Ready;
                    self.queue.enqueue(old_pid);
                }
            }
        }

        // Activate the next task.
        if let Some(next_pcb) = self.manager.get_mut(next_pid) {
            next_pcb.state = ProcessState::Running;
        }
        self.current_pid = next_pid;

        // Collect the stack pointers we need before the switch.
        let old_rsp_ptr = self
            .manager
            .get_mut(old_pid)
            .map(|p| &mut p.stack_ptr as *mut u64)
            .unwrap_or(core::ptr::null_mut());

        let next_rsp = self.manager.get(next_pid).map(|p| p.stack_ptr).unwrap_or(0);

        if old_rsp_ptr.is_null() || next_rsp == 0 {
            return;
        }

        // Set the TSS RSP0 for the incoming thread.
        let next_stack_base = self
            .manager
            .get(next_pid)
            .map(|p| p.stack_base as u64 + p.stack_size as u64)
            .unwrap_or(0);
        if next_stack_base != 0 {
            unsafe {
                crate::arch::x86_64::gdt::set_tss_stack(next_stack_base);
                crate::arch::x86_64::syscall::SYSCALL_KERNEL_STACK = next_stack_base;
            }
        }

        // Perform the actual register save/restore.
        // SAFETY: Both pointers are valid PCB fields from our owned table.
        unsafe {
            context_switch::switch_to(old_rsp_ptr, next_rsp);
        }
    }

    /// Move any sleeping task past its deadline back to the run queue.
    fn wake_sleeping(&mut self, current_ticks: u64) {
        // Collect PIDs to wake (avoid borrow-checker conflicts).
        let mut to_wake: [u16; 64] = [0; 64];
        let mut count = 0usize;

        for pcb in self.manager.iter() {
            if pcb.state == ProcessState::Sleeping && current_ticks >= pcb.sleep_until {
                if count < to_wake.len() {
                    to_wake[count] = pcb.pid;
                    count += 1;
                }
            }
        }

        for &pid in &to_wake[..count] {
            if let Some(pcb) = self.manager.get_mut(pid) {
                pcb.state = ProcessState::Ready;
                pcb.sleep_until = 0;
            }
            if !self.queue.contains(pid) {
                self.queue.enqueue(pid);
            }
        }
    }
}

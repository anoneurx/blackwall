use crate::arch::x86_64::trap::{switch_cr3, IrqFrame};
use crate::process::manager::ProcessManager;
use crate::process::state::ProcessState;
use crate::scheduler::queue::RunQueue;
use core::ptr;

/// Number of PIT ticks per scheduling quantum.
///
/// At 100 Hz (10 ms/tick) this gives ~100 ms time slices.
pub const TIME_SLICE_TICKS: u64 = 10;

/// The round-robin scheduler.
///
/// Owns the process table and the run queue.  There is deliberately **no**
/// stack-switching code in here: a switch is expressed purely as "return a
/// different trap frame to the common `iretq` epilogue" ([`Self::reschedule`]).
/// That makes the scheduler usable from an IRQ, from a syscall and from a
/// voluntary yield with one implementation.
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

    /// Enable scheduling.  Must be called once every task that should be
    /// runnable has been created.
    pub fn start(&mut self, current_pid: u16) {
        self.current_pid = current_pid;
        self.started = true;
    }

    pub fn started(&self) -> bool {
        self.started
    }

    // -----------------------------------------------------------------------
    // Entry points used by the trap dispatchers
    // -----------------------------------------------------------------------

    /// Called from the IRQ0 trap on every tick.
    ///
    /// Wakes sleepers, accounts CPU time and — at a quantum boundary or when
    /// the current task stopped being runnable — picks the next frame.
    pub fn on_timer_tick(&mut self, ticks: u64, frame: *mut IrqFrame) -> *mut IrqFrame {
        if !self.started {
            return ptr::null_mut();
        }

        self.wake_sleeping(ticks);

        // SAFETY: the frame pointer comes from the live trap entry.
        unsafe {
            self.manager.reap_zombies();
        }

        if let Some(pcb) = self.manager.get_mut(self.current_pid) {
            if pcb.state == ProcessState::Running {
                pcb.ticks_run = pcb.ticks_run.saturating_add(1);
            }
        }

        let current_state = self.manager.get(self.current_pid).map(|p| p.state);
        let runnable =
            matches!(current_state, Some(ProcessState::Running) | Some(ProcessState::Ready));
        let quantum_expired = ticks % TIME_SLICE_TICKS == 0;

        if !runnable {
            // The task blocked or exited — hand over immediately.
            self.reschedule(frame, true)
        } else if quantum_expired {
            self.reschedule(frame, false)
        } else {
            ptr::null_mut()
        }
    }

    /// Decide whether to switch tasks and, if so, return the frame to resume.
    ///
    /// * `frame`  — the live frame of the current task (saved if we switch).
    /// * `force`  — switch even if the current task is still runnable (used by
    ///              blocking syscalls, `exit` and voluntary yields).
    ///
    /// A `null` return means "keep running the caller's frame".
    pub fn reschedule(&mut self, frame: *mut IrqFrame, force: bool) -> *mut IrqFrame {
        if !self.started {
            return ptr::null_mut();
        }

        let old_pid = self.current_pid;
        let old_state = self.manager.get(old_pid).map(|p| p.state);
        let old_runnable =
            matches!(old_state, Some(ProcessState::Running) | Some(ProcessState::Ready));

        if !force && old_runnable && self.queue.is_empty() {
            // Nobody else wants the CPU.
            self.save_frame(old_pid, frame);
            return ptr::null_mut();
        }

        let Some(next_pid) = self.pick_next(old_pid) else {
            if old_runnable {
                self.save_frame(old_pid, frame);
                return ptr::null_mut();
            }
            // Nothing runnable at all — keep the CPU on the current frame
            // rather than returning to a task that will just fault again.
            return ptr::null_mut();
        };

        if next_pid == old_pid {
            self.save_frame(old_pid, frame);
            return ptr::null_mut();
        }

        // Park the outgoing task.
        if let Some(old_pcb) = self.manager.get_mut(old_pid) {
            old_pcb.stack_ptr = frame as u64;
            if old_pcb.state == ProcessState::Running {
                old_pcb.state = ProcessState::Ready;
                if !self.queue.contains(old_pid) {
                    self.queue.enqueue(old_pid);
                }
            }
        }

        // Activate the incoming task.
        let (next_frame, kstack, cr3) = match self.manager.get_mut(next_pid) {
            Some(pcb) => {
                pcb.state = ProcessState::Running;
                (pcb.stack_ptr, pcb.kernel_stack_top(), pcb.cr3)
            }
            None => return ptr::null_mut(),
        };

        self.current_pid = next_pid;

        if next_frame == 0 {
            return ptr::null_mut();
        }

        // Per-task ring-3 → ring-0 stacks and address space.
        unsafe {
            if kstack != 0 {
                crate::arch::x86_64::gdt::set_tss_stack(kstack);
                crate::arch::x86_64::syscall::SYSCALL_KERNEL_STACK = kstack;
            }
            if cr3 != 0 {
                switch_cr3(cr3);
            }
        }

        next_frame as *mut IrqFrame
    }

    // -----------------------------------------------------------------------
    // Public interface used by kernel threads and syscalls
    // -----------------------------------------------------------------------

    /// Spawn a new kernel thread and enqueue it.
    pub fn spawn_kernel_thread(&mut self, name: &str, entry: fn(), priority: u8) -> u16 {
        let pid = self.manager.spawn(name, entry, 32 * 1024, priority);
        self.queue.enqueue(pid);
        pid
    }

    /// Register a user process whose frame has already been built.
    ///
    /// Returns the PID.
    pub fn enqueue_task(&mut self, pid: u16) {
        self.queue.enqueue(pid);
    }

    /// Create a user task, build its initial ring-3 frame and enqueue it.
    ///
    /// `kernel_stack_size` is the size of the private kernel stack used for
    /// syscalls and ring-3 → ring-0 transitions.
    pub fn spawn_user_task(
        &mut self,
        name: &str,
        cr3: u64,
        entry: u64,
        user_rsp: u64,
        priority: u8,
        kernel_stack_size: usize,
    ) -> u16 {
        let (pid, kernel_stack_top) =
            self.manager.spawn_user(name, cr3, priority, kernel_stack_size);
        let frame = unsafe {
            // SAFETY: the manager just allocated and zeroed this stack.
            crate::scheduler::context_switch::init_user_frame(kernel_stack_top, entry, user_rsp, 0)
        };
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.stack_ptr = frame;
        }
        crate::logging::print(format_args!(
            "[SPAWN] pid={} entry={:#x} user_rsp={:#x} kstack_top={:#x} frame={:#x} cr3={:#x}\n",
            pid, entry, user_rsp, kernel_stack_top, frame, cr3
        ));
        self.queue.enqueue(pid);
        pid
    }

    /// Mark a task as Sleeping until `current_ticks + duration_ticks`.
    pub fn sleep_task(&mut self, pid: u16, duration_ticks: u64, current_ticks: u64) {
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Sleeping;
            pcb.sleep_until = current_ticks.saturating_add(duration_ticks);
        }
        self.queue.remove(pid);
    }

    /// Forcefully terminate a task: mark as Zombie, remove from run queue.
    pub fn terminate_task(&mut self, pid: u16) {
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Zombie;
        }
        self.queue.remove(pid);
    }

    /// Free the heap-allocated kernel stack of a terminated task.
    ///
    /// # Safety
    /// The task must not be executing and must be in the Zombie state.
    pub unsafe fn free_zombie_stack(&mut self, pid: u16) {
        self.manager.free_stack(pid);
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.state = ProcessState::Terminated;
        }
    }

    /// Immutable view of the process manager.
    pub fn manager(&self) -> &ProcessManager {
        &self.manager
    }

    /// Mutable view of the process manager.
    pub fn manager_mut(&mut self) -> &mut ProcessManager {
        &mut self.manager
    }

    // -----------------------------------------------------------------------
    // Internal scheduling logic
    // -----------------------------------------------------------------------

    fn save_frame(&mut self, pid: u16, frame: *mut IrqFrame) {
        if let Some(pcb) = self.manager.get_mut(pid) {
            pcb.stack_ptr = frame as u64;
        }
    }

    /// Take the next runnable task off the run queue (excluding `old_pid`).
    fn pick_next(&mut self, old_pid: u16) -> Option<u16> {
        while let Some(pid) = self.queue.dequeue() {
            if pid == old_pid {
                // Put it back — we only want a *different* task here.
                if !self.queue.contains(pid) {
                    self.queue.enqueue(pid);
                }
                // If that was the only task, the loop ends after the queue
                // drained; break out to the scan below.
                break;
            }
            let state = self.manager.get(pid).map(|p| p.state);
            if matches!(state, Some(ProcessState::Ready) | Some(ProcessState::Running)) {
                return Some(pid);
            }
            // Stale queue entry (sleeping/terminated) — drop it and continue.
        }

        // Fallback: a runnable task that somehow missed the queue.
        self.manager
            .iter()
            .find(|p| p.pid != old_pid && p.state == ProcessState::Ready)
            .map(|p| p.pid)
    }

    /// Move any sleeping task past its deadline back to the run queue.
    fn wake_sleeping(&mut self, current_ticks: u64) {
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

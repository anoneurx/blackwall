use crate::scheduler::RoundRobinScheduler;

/// Terminate a kernel thread.
///
/// Marks the thread as Zombie and removes it from the run queue.  The
/// heap-allocated stack is freed immediately.
///
/// After this call, `pid` will never be scheduled again.
///
/// # Safety
///
/// The caller must ensure `pid` is **not** the currently executing thread,
/// and that no other path holds a reference to its stack memory.
pub unsafe fn thread_exit(scheduler: &mut RoundRobinScheduler, pid: u16) {
    scheduler.terminate_task(pid);
    // SAFETY: upheld by the caller contract above.
    scheduler.free_zombie_stack(pid);
    crate::logging::print(format_args!("Thread Destroyed: PID {}\n", pid));
}

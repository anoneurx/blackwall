use crate::scheduler::RoundRobinScheduler;

/// Spawn a new kernel thread.
///
/// This is a convenience wrapper around the scheduler's internal spawn
/// path.  The thread becomes `Ready` immediately and will be selected by
/// the next scheduler tick.
///
/// # Arguments
///
/// * `scheduler` — mutable reference to the active scheduler.
/// * `name`      — human-readable name (truncated to 31 bytes).
/// * `entry`     — function the thread begins executing after its first
///                 context switch.
/// * `priority`  — scheduling priority (0 = lowest / idle).
///
/// # Returns
///
/// The PID assigned to the new thread.
pub fn thread_spawn(
    scheduler: &mut RoundRobinScheduler,
    name: &str,
    entry: fn(),
    priority: u8,
) -> u16 {
    let pid = scheduler.spawn_kernel_thread(name, entry, priority);
    crate::logging::print(format_args!("Thread Created: PID {}\n", pid));
    pid
}

use crate::arch::x86_64::timer;
use crate::scheduler::RoundRobinScheduler;

/// Put a kernel thread to sleep for `duration_ticks` PIT ticks.
///
/// The thread is removed from the run queue and will not be selected
/// again until the timer ISR wakes it after the deadline passes.
///
/// # Arguments
///
/// * `scheduler`      — mutable reference to the active scheduler.
/// * `pid`            — the PID to sleep.
/// * `duration_ticks` — number of PIT ticks to sleep (100 Hz → 10 ticks/s).
pub fn thread_sleep(scheduler: &mut RoundRobinScheduler, pid: u16, duration_ticks: u64) {
    let now = timer::ticks();
    scheduler.sleep_task(pid, duration_ticks, now);
    crate::logging::print(format_args!(
        "PID {} Sleeping for {} ticks (until tick {})\n",
        pid,
        duration_ticks,
        now.saturating_add(duration_ticks),
    ));
}

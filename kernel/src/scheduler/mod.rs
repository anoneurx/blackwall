pub mod context_switch;
pub mod queue;
pub mod round_robin;

pub use round_robin::RoundRobinScheduler;

use crate::sync::SpinLock;
pub static SCHEDULER: SpinLock<Option<RoundRobinScheduler>> = SpinLock::new(None);

/// PID of the task the scheduler currently considers running (0 if the
/// scheduler has not started yet).
///
/// Safe to call from a trap dispatcher that already holds the scheduler lock
/// in the same task (it only reads the cached field through a `try_lock`).
pub fn current_pid() -> u16 {
    match SCHEDULER.try_lock() {
        Some(guard) => guard.as_ref().map(|s| s.current_pid).unwrap_or(0),
        None => 0,
    }
}

pub mod context_switch;
pub mod queue;
pub mod round_robin;

pub use round_robin::RoundRobinScheduler;

use crate::sync::SpinLock;
pub static SCHEDULER: SpinLock<Option<RoundRobinScheduler>> = SpinLock::new(None);

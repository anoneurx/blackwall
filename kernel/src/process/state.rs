/// All possible lifecycle states for a kernel process/thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ProcessState {
    /// Currently executing on the CPU.
    Running = 0,
    /// Ready to run; waiting to be selected by the scheduler.
    Ready = 1,
    /// Sleeping until a timer wakeup tick is reached.
    Sleeping = 2,
    /// Blocked waiting for a resource (future use).
    Blocked = 3,
    /// Has exited but its PCB has not yet been reaped.
    Zombie = 4,
    /// Fully terminated and removed from all queues.
    Terminated = 5,
}

impl ProcessState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessState::Running => "Running",
            ProcessState::Ready => "Ready",
            ProcessState::Sleeping => "Sleeping",
            ProcessState::Blocked => "Blocked",
            ProcessState::Zombie => "Zombie",
            ProcessState::Terminated => "Terminated",
        }
    }
}

extern crate alloc;

use alloc::collections::VecDeque;

/// A simple FIFO queue of PIDs representing tasks that are ready to run.
///
/// The scheduler pushes to the back and pops from the front, giving each
/// task a fair turn in insertion order (round-robin).
pub struct RunQueue {
    inner: VecDeque<u16>,
}

impl RunQueue {
    pub fn new() -> Self {
        Self { inner: VecDeque::new() }
    }

    /// Add a PID to the back of the queue.
    pub fn enqueue(&mut self, pid: u16) {
        self.inner.push_back(pid);
    }

    /// Remove and return the PID at the front of the queue.
    pub fn dequeue(&mut self) -> Option<u16> {
        self.inner.pop_front()
    }

    /// Remove a specific PID from any position in the queue.
    pub fn remove(&mut self, pid: u16) {
        self.inner.retain(|&p| p != pid);
    }

    /// Returns `true` if the queue contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Number of tasks currently in the queue.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the given PID is present in the queue.
    pub fn contains(&self, pid: u16) -> bool {
        self.inner.contains(&pid)
    }
}

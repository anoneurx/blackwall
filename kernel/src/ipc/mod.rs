/// IPC — Inter-Process Communication
/// Provides a kernel message-passing channel (async, bounded FIFO).
extern crate alloc;

use crate::sync::spin::SpinLock;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub const MAX_MESSAGE_SIZE: usize = 4096;
pub const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct Message {
    pub sender_pid: u64,
    pub data: Vec<u8>,
}

/// A bounded, lock-protected message channel.
pub struct Channel {
    pub queue: SpinLock<VecDeque<Message>>,
    pub capacity: usize,
}

impl Channel {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self { queue: SpinLock::new(VecDeque::with_capacity(capacity)), capacity })
    }

    /// Non-blocking send. Returns `false` if the queue is full.
    pub fn send(&self, msg: Message) -> bool {
        let mut q = self.queue.lock();
        if q.len() >= self.capacity {
            return false;
        }
        q.push_back(msg);
        true
    }

    /// Non-blocking receive. Returns `None` if the queue is empty.
    pub fn recv(&self) -> Option<Message> {
        self.queue.lock().pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }
}

/// Global IPC channel registry: maps service name → channel endpoint
pub static IPC_REGISTRY: SpinLock<Option<IpcRegistry>> = SpinLock::new(None);

pub struct IpcRegistry {
    services: Vec<(alloc::string::String, Arc<Channel>)>,
}

impl IpcRegistry {
    pub fn new() -> Self {
        Self { services: Vec::new() }
    }

    pub fn register(&mut self, name: &str, ch: Arc<Channel>) {
        self.services.push((alloc::string::String::from(name), ch));
    }

    pub fn lookup(&self, name: &str) -> Option<Arc<Channel>> {
        self.services.iter().find(|(n, _)| n.as_str() == name).map(|(_, ch)| ch.clone())
    }
}

pub fn init() {
    use crate::arch::x86_64::serial;
    serial::line("[IPC] Initializing IPC registry...");
    let mut reg = IpcRegistry::new();

    // Pre-register core system services
    reg.register("kernel.log", Channel::new(CHANNEL_CAPACITY));
    reg.register("kernel.events", Channel::new(CHANNEL_CAPACITY));
    reg.register("fs.requests", Channel::new(CHANNEL_CAPACITY));
    reg.register("net.packets", Channel::new(CHANNEL_CAPACITY));

    *IPC_REGISTRY.lock() = Some(reg);
    serial::line(
        "[IPC] IPC registry ready. Services: kernel.log, kernel.events, fs.requests, net.packets",
    );
}

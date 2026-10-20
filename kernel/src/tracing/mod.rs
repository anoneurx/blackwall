use core::sync::atomic::{AtomicU64, Ordering};

static COUNTER_SYSCALL: AtomicU64 = AtomicU64::new(0);
static COUNTER_CTXSW: AtomicU64 = AtomicU64::new(0);

pub fn inc_syscall() {
    COUNTER_SYSCALL.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_ctxsw() {
    COUNTER_CTXSW.fetch_add(1, Ordering::Relaxed);
}

pub fn get_syscall_count() -> u64 {
    COUNTER_SYSCALL.load(Ordering::Relaxed)
}

pub fn get_ctxsw_count() -> u64 {
    COUNTER_CTXSW.load(Ordering::Relaxed)
}

pub fn reset() {
    COUNTER_SYSCALL.store(0, Ordering::Relaxed);
    COUNTER_CTXSW.store(0, Ordering::Relaxed);
}

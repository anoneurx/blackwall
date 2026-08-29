use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A simple spin-lock for single-CPU kernel use.
///
/// Provides mutual exclusion between the main kernel path and interrupt
/// service routines.  On a uniprocessor kernel this is sufficient; SMP
/// would additionally require disabling interrupts or per-CPU structures.
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: The spin-lock guarantees exclusive access at all times.
unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        Self { locked: AtomicBool::new(false), data: UnsafeCell::new(data) }
    }

    /// Spin until the lock is acquired and return a guard.
    ///
    /// # Panics
    ///
    /// Will spin forever if the lock is already held by the same execution
    /// context (dead-lock).  Callers must ensure they do not re-enter.
    pub fn lock(&self) -> SpinGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Hint to the CPU that we are in a spin-wait loop.
            core::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }

    /// Attempt to acquire the lock without blocking.
    ///
    /// Returns `None` if the lock is currently held.
    pub fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            Some(SpinGuard { lock: self })
        } else {
            None
        }
    }

    /// Forcefully unlock the spin lock.
    ///
    /// # Safety
    ///
    /// This should only be used in low-level startup code (like thread
    /// trampolines) to release a lock held across a context switch.
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: The guard proves we hold the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The guard proves we hold the lock exclusively.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

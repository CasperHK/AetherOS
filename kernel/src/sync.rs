//! # Synchronisation Primitives
//!
//! Provides a `no_std`-compatible `SpinLock<T>` (a spin-based mutex) that
//! is safe to use from interrupt context on a single-hart (CPU core) system.
//!
//! ## Design notes
//! - The lock uses `compare_exchange_weak` with `Acquire`/`Release` ordering,
//!   which maps to the appropriate RISC-V LR/SC fence sequences.
//! - `core::hint::spin_loop()` lowers power consumption while spinning
//!   (equivalent to `pause` on x86, `yield` on AArch64, `nop` on RV).
//! - **Interrupt safety**: callers that acquire a SpinLock inside an ISR
//!   must ensure the same lock is never held across an interrupt boundary.
//!   Use `arch::disable_interrupts()` / `restore_interrupts()` around the
//!   lock acquisition in task context when the lock can be taken in an ISR.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

// ── SpinLock ─────────────────────────────────────────────────────────────────

/// A spin-based mutual-exclusion lock.
///
/// ```no_run
/// use crate::sync::SpinLock;
///
/// static TABLE: SpinLock<[u8; 4]> = SpinLock::new([0u8; 4]);
///
/// let mut guard = TABLE.lock();
/// guard[0] = 42;
/// // guard drops here → lock released
/// ```
pub struct SpinLock<T> {
    locked: AtomicBool,
    data:   UnsafeCell<T>,
}

// SAFETY: SpinLock ensures exclusive access via atomic operations.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Create a new, unlocked `SpinLock` wrapping `data`.
    ///
    /// This is a `const fn` so the lock can be placed in a `static`.
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data:   UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, spinning until it becomes available.
    ///
    /// Returns a [`SpinLockGuard`] that releases the lock on drop.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Spin with a CPU-level hint to reduce power and bus contention.
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard { lock: self }
    }

    /// Try to acquire the lock without spinning.
    ///
    /// Returns `Some(guard)` if the lock was free, `None` if it was held.
    /// Useful for non-blocking tries inside ISRs.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinLockGuard { lock: self })
    }

    /// Force-release the lock without acquiring a guard.
    ///
    /// # Safety
    /// The caller must guarantee it currently holds the lock and that no
    /// other code is accessing the inner data concurrently.
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

// ── SpinLockGuard ─────────────────────────────────────────────────────────────

/// RAII guard returned by [`SpinLock::lock`].
///
/// The lock is released automatically when this guard is dropped.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold the lock, so exclusive access is guaranteed.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: same reasoning as above.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

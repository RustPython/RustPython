//! One-byte mutex whose blocking acquire detaches the waiting thread.
//!
//! Only the two least significant bits of the state are used:
//! 0b00 unlocked, 0b01 locked, 0b10 unlocked with parked threads,
//! 0b11 locked with parked threads.

use core::sync::atomic::AtomicU8;
#[cfg(feature = "threading")]
use core::sync::atomic::Ordering;

#[cfg(feature = "threading")]
const UNLOCKED: u8 = 0;
#[cfg(feature = "threading")]
const LOCKED: u8 = 1;
#[cfg(feature = "threading")]
const HAS_PARKED: u8 = 2;

/// `PyMutex`. Zero is the unlocked state.
#[derive(Debug, Default)]
#[repr(transparent)]
pub(crate) struct PyMutex {
    bits: AtomicU8,
}

impl PyMutex {
    pub(crate) const fn new() -> Self {
        Self {
            bits: AtomicU8::new(0),
        }
    }
}

#[cfg(feature = "threading")]
mod threaded {
    use super::{HAS_PARKED, LOCKED, Ordering, PyMutex, UNLOCKED};
    use core::cell::Cell;
    use core::time::Duration;
    use parking_lot_core::{FilterOp, ParkResult, ParkToken, UnparkToken};
    use std::time::Instant;

    /// A waiter blocked longer than this is handed the lock directly by the
    /// unlocking thread, which avoids starvation.
    const TIME_TO_BE_FAIR: Duration = Duration::from_millis(1);

    /// Spin for a bit before parking the thread.
    const MAX_SPIN_COUNT: usize = 40;

    const TOKEN_NORMAL: UnparkToken = UnparkToken(0);
    const TOKEN_HANDED_OFF: UnparkToken = UnparkToken(1);

    /// `struct mutex_entry`; its address is the waiter's park token.
    struct MutexEntry {
        /// After this point the unlocking thread hands off ownership directly.
        time_to_be_fair: Instant,
    }

    impl PyMutex {
        /// `PyMutex_LockFast`
        #[inline]
        pub(crate) fn lock_fast(&self) -> bool {
            self.bits
                .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        }

        /// `PyMutex_Lock`. A thread attached to an interpreter detaches while
        /// it is parked.
        #[inline]
        pub(crate) fn lock(&self) {
            if !self.lock_fast() {
                self.lock_slow();
            }
        }

        /// `_PyMutex_LockTimed` with no timeout and `_PY_LOCK_DETACH`.
        #[cold]
        fn lock_slow(&self) {
            let mut v = self.bits.load(Ordering::Relaxed);
            if v & LOCKED == 0
                && self
                    .bits
                    .compare_exchange(v, v | LOCKED, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
            {
                return;
            }

            let entry = MutexEntry {
                time_to_be_fair: Instant::now() + TIME_TO_BE_FAIR,
            };

            let mut spin_count = 0;
            loop {
                if v & LOCKED == 0 {
                    // The lock is unlocked. Try to grab it.
                    match self.bits.compare_exchange(
                        v,
                        v | LOCKED,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => return,
                        Err(actual) => {
                            v = actual;
                            continue;
                        }
                    }
                }

                if v & HAS_PARKED == 0 && spin_count < MAX_SPIN_COUNT {
                    // Spin for a bit.
                    std::thread::yield_now();
                    spin_count += 1;
                    continue;
                }

                let mut newv = v;
                if v & HAS_PARKED == 0 {
                    // We are the first waiter. Set the HAS_PARKED flag.
                    newv = v | HAS_PARKED;
                    if let Err(actual) =
                        self.bits
                            .compare_exchange(v, newv, Ordering::Relaxed, Ordering::Relaxed)
                    {
                        v = actual;
                        continue;
                    }
                }

                if self.park(newv, &entry) == ParkResult::Unparked(TOKEN_HANDED_OFF) {
                    // We own the lock now.
                    debug_assert!(self.bits.load(Ordering::Relaxed) & LOCKED != 0);
                    return;
                }

                v = self.bits.load(Ordering::Relaxed);
            }
        }

        /// `_PyParkingLot_Park` on this mutex, detached when attached.
        fn park(&self, expected: u8, entry: &MutexEntry) -> ParkResult {
            let result = Cell::new(ParkResult::Invalid);
            crate::vm::thread::wait_detached_from_interpreter(&|| {
                // SAFETY: the key is this mutex's address, used only by this
                // mutex. The validate and timed-out callbacks only load an
                // atomic or do nothing, so they neither panic nor reenter the
                // parking lot. The entry outlives the park, and the unparking
                // thread reads it under the bucket lock while this thread is
                // still queued.
                result.set(unsafe {
                    parking_lot_core::park(
                        self.key(),
                        || self.bits.load(Ordering::Relaxed) == expected,
                        || {},
                        |_, _| {},
                        ParkToken(core::ptr::from_ref(entry) as usize),
                        None,
                    )
                });
            });
            result.get()
        }

        /// `PyMutex_Unlock`
        #[inline]
        pub(crate) fn unlock(&self) {
            if self
                .bits
                .compare_exchange(LOCKED, UNLOCKED, Ordering::Release, Ordering::Relaxed)
                .is_err()
            {
                self.unlock_slow();
            }
        }

        /// `_PyMutex_UnlockSlow`
        #[cold]
        fn unlock_slow(&self) {
            assert!(
                self.try_unlock().is_ok(),
                "unlocking mutex that is not locked"
            );
        }

        /// `_PyMutex_TryUnlock`
        fn try_unlock(&self) -> Result<(), ()> {
            let mut v = self.bits.load(Ordering::Acquire);
            loop {
                if v & LOCKED == 0 {
                    // error: the mutex is not locked
                    return Err(());
                } else if v & HAS_PARKED != 0 {
                    // wake up a single thread
                    self.unpark();
                    return Ok(());
                }
                match self
                    .bits
                    .compare_exchange(v, UNLOCKED, Ordering::Release, Ordering::Relaxed)
                {
                    // fast-path: no waiters
                    Ok(_) => return Ok(()),
                    Err(actual) => v = actual,
                }
            }
        }

        /// `_PyParkingLot_Unpark` with `mutex_unpark`.
        fn unpark(&self) {
            let waiter = Cell::new(None);
            // SAFETY: the key is this mutex's address. The filter and callback
            // run under the bucket lock, do not panic and do not reenter the
            // parking lot. A token is the address of a `MutexEntry` its waiter
            // keeps alive while it is queued.
            unsafe {
                parking_lot_core::unpark_filter(
                    self.key(),
                    |token| {
                        if waiter.get().is_some() {
                            return FilterOp::Stop;
                        }
                        waiter.set(Some(token.0 as *const MutexEntry));
                        FilterOp::Unpark
                    },
                    |result| {
                        let mut v = UNLOCKED;
                        let mut token = TOKEN_NORMAL;
                        if let Some(entry) = waiter.get() {
                            let should_be_fair = Instant::now() > (*entry).time_to_be_fair;
                            if should_be_fair {
                                v |= LOCKED;
                                token = TOKEN_HANDED_OFF;
                            }
                            if result.have_more_threads {
                                v |= HAS_PARKED;
                            }
                        }
                        self.bits.store(v, Ordering::Release);
                        token
                    },
                );
            }
        }

        /// `PyMutex_IsLocked`
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "Removed when the profiler consumer is wired")
        )]
        pub(crate) fn is_locked(&self) -> bool {
            self.bits.load(Ordering::Acquire) & LOCKED != 0
        }

        fn key(&self) -> usize {
            core::ptr::from_ref(&self.bits) as usize
        }
    }
}

#[cfg(test)]
#[cfg(feature = "threading")]
mod tests {
    use super::{HAS_PARKED, PyMutex};
    use alloc::sync::Arc;
    use core::{sync::atomic::Ordering, time::Duration};

    #[test]
    fn unlock_hands_off_to_starved_waiter() {
        let m = Arc::new(PyMutex::new());
        m.lock();
        let waiter = {
            let m = m.clone();
            std::thread::spawn(move || {
                m.lock();
                m.unlock();
            })
        };
        while m.bits.load(Ordering::Relaxed) & HAS_PARKED == 0 {
            std::thread::yield_now();
        }
        // Long past the waiter's fairness deadline.
        std::thread::sleep(Duration::from_millis(20));
        m.unlock();
        // The waiter owns the lock before it has even run again.
        assert!(!m.lock_fast());
        waiter.join().unwrap();
        assert!(!m.is_locked());
    }
}

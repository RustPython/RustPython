//! Suspendable native operation scopes, separate from Rust payload borrows.
//!
//! A scope does not grant access to data. Payload mutex guards and references
//! into mutable collections must end before callbacks, safepoints, nested
//! scopes or detachment. Keep owned Python references across those boundaries:
//! keeping the operation lock alive does not root its containing Python object.

use crate::VirtualMachine;

/// An operation lock confined to one interpreter, without owning that interpreter.
#[derive(Debug)]
pub(crate) struct CriticalSectionLock {
    interpreter_id: i64,
    #[cfg(feature = "threading")]
    lock: std::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl CriticalSectionLock {
    pub(crate) fn new(vm: &VirtualMachine) -> Self {
        Self {
            interpreter_id: vm.state.interpreter_id,
            #[cfg(feature = "threading")]
            lock: std::sync::Arc::new(core::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Run an operation while its section is the active top section.
    ///
    /// Calling Python can suspend this lock, so revalidate state afterward.
    /// This API deliberately exposes no guard or data reference. Traversal
    /// uses separate short payload locks; there is no stopped-world bypass.
    pub(crate) fn with<R>(&self, vm: &VirtualMachine, f: impl FnOnce() -> R) -> R {
        assert_eq!(self.interpreter_id, vm.state.interpreter_id);
        #[cfg(feature = "threading")]
        {
            threaded::with(self, &vm.state, f)
        }
        #[cfg(not(feature = "threading"))]
        {
            assert_eq!(
                super::try_with_current_vm(|current| current.state.interpreter_id),
                Some(self.interpreter_id),
                "operation section requires the current interpreter"
            );
            f()
        }
    }
}

#[cfg(feature = "threading")]
pub(super) use threaded::{assert_empty, resume, suspend};

#[cfg(feature = "threading")]
mod threaded {
    use super::CriticalSectionLock;
    use crate::vm::{PyGlobalState, thread};
    use core::{
        cell::RefCell,
        marker::PhantomData,
        sync::atomic::{AtomicBool, Ordering},
    };
    use std::{collections::HashMap, rc::Rc, sync::Arc};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Status {
        Pending,
        Active,
        Inactive,
    }

    struct Section {
        lock: Arc<AtomicBool>,
        status: Status,
    }

    std::thread_local! {
        // Interpreter IDs survive ThreadSlot replacement after fork. Entries
        // belong only to this OS thread and are removed when their scope ends.
        static STACKS: RefCell<HashMap<i64, Vec<Section>>> = RefCell::default();
    }

    fn assert_attached(state: &PyGlobalState) {
        let slot = thread::current_thread_slot().expect("operation section requires a thread slot");
        assert_eq!(slot.interpreter_id, state.interpreter_id);
        assert_eq!(
            slot.state.load(Ordering::Acquire),
            thread::ThreadState::Attached as i32,
            "operation section requires an attached interpreter"
        );
    }

    // The Arc supplies a stable, exclusively controlled parking key. No
    // ownership is transferred to a waking thread: it must attach, pass STW
    // admission, and win a fresh CAS before accessing operation state.
    fn notify(lock: &Arc<AtomicBool>) {
        lock.store(false, Ordering::Release);
        // SAFETY: this address is owned by this synchronization primitive and
        // remains alive through the call. No other primitive uses its key.
        unsafe {
            parking_lot_core::unpark_all(
                Arc::as_ptr(lock) as usize,
                parking_lot_core::DEFAULT_UNPARK_TOKEN,
            );
        }
    }

    fn acquire_top(state: &PyGlobalState, lock: &Arc<AtomicBool>) {
        loop {
            assert_attached(state);
            let acquired = STACKS.with(|stacks| {
                let mut stacks = stacks.borrow_mut();
                let top = stacks
                    .get_mut(&state.interpreter_id)
                    .unwrap()
                    .last_mut()
                    .unwrap();
                assert_eq!(top.status, Status::Pending);
                assert!(Arc::ptr_eq(&top.lock, lock));
                if lock
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    // No allocation, callback, safepoint or fallible operation
                    // between acquiring ownership and recording it for suspend.
                    top.status = Status::Active;
                    true
                } else {
                    false
                }
            });
            if acquired {
                return;
            }
            thread::allow_threads_for_state(state, || {
                // SAFETY: the strong lock reference keeps this controlled key
                // alive. The queue-locked callbacks only load an atomic or do
                // nothing; they cannot panic or reenter the parking lot. The
                // thread was detached before entering park, including before
                // its queue validation, and never acquires ownership here.
                unsafe {
                    parking_lot_core::park(
                        Arc::as_ptr(lock) as usize,
                        || lock.load(Ordering::Acquire),
                        || {},
                        |_, _| {},
                        parking_lot_core::DEFAULT_PARK_TOKEN,
                        None,
                    );
                }
            });
            // Queue validation and notify serialize through the parking bucket:
            // either we were queued for the release, or validate saw unlocked.
            // A new owner supplies a later notification. Retry after attachment;
            // this intentionally provides no fairness or handoff guarantee.
        }
    }

    struct Scope<'a> {
        state: &'a PyGlobalState,
        depth: usize,
        _not_send: PhantomData<Rc<()>>,
    }

    impl Drop for Scope<'_> {
        fn drop(&mut self) {
            let section = STACKS.with(|stacks| {
                let mut stacks = stacks.borrow_mut();
                let stack = stacks.get_mut(&self.state.interpreter_id).unwrap();
                assert_eq!(stack.len(), self.depth);
                let section = stack.pop().unwrap();
                if stack.is_empty() {
                    stacks.remove(&self.state.interpreter_id);
                }
                section
            });
            if section.status == Status::Active {
                notify(&section.lock);
            }
            resume(self.state);
        }
    }

    pub(super) fn with<R>(
        lock: &CriticalSectionLock,
        state: &PyGlobalState,
        f: impl FnOnce() -> R,
    ) -> R {
        assert_attached(state);
        let depth = STACKS.with(|stacks| {
            let mut stacks = stacks.borrow_mut();
            let stack = stacks.entry(state.interpreter_id).or_default();
            if stack.last().is_some_and(|top| {
                top.status == Status::Active && Arc::ptr_eq(&top.lock, &lock.lock)
            }) {
                return None;
            }
            // Allocate before acquiring; unwinding cannot strand ownership.
            stack.push(Section {
                lock: lock.lock.clone(),
                status: Status::Pending,
            });
            Some(stack.len())
        });
        let Some(depth) = depth else {
            return f();
        };
        let _scope = Scope {
            state,
            depth,
            _not_send: PhantomData,
        };
        acquire_top(state, &lock.lock);
        f()
    }

    /// Release operation ownership before publishing DETACHED, SUSPENDED or
    /// SHUTTING_DOWN. Payload guards must have ended before this point.
    pub(crate) fn suspend(interpreter_id: i64) {
        STACKS.with(|stacks| {
            let mut stacks = stacks.borrow_mut();
            if let Some(stack) = stacks.get_mut(&interpreter_id) {
                for section in stack.iter_mut().rev() {
                    if section.status == Status::Active {
                        section.status = Status::Inactive;
                        // notify does not invoke user code or detach. No TLS
                        // borrow is retained across an operation that can do so.
                        notify(&section.lock);
                    }
                }
            }
        });
    }

    /// Restore only the top section. A pending acquire owns no lock and is
    /// resumed by its own loop after the attachment path returns.
    pub(crate) fn resume(state: &PyGlobalState) {
        let lock = STACKS.with(|stacks| {
            let mut stacks = stacks.borrow_mut();
            let top = stacks.get_mut(&state.interpreter_id)?.last_mut()?;
            if top.status != Status::Inactive {
                return None;
            }
            top.status = Status::Pending;
            Some(top.lock.clone())
        });
        if let Some(lock) = lock {
            acquire_top(state, &lock);
        }
    }

    pub(crate) fn assert_empty(interpreter_id: i64) {
        STACKS.with(|stacks| {
            assert!(
                !stacks.borrow().contains_key(&interpreter_id),
                "thread cleanup cannot abandon an operation section"
            );
        });
    }
}

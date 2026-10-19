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
    lock: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl CriticalSectionLock {
    #[cfg_attr(
        not(all(test, feature = "threading")),
        expect(dead_code, reason = "Removed when the profiler consumer is wired")
    )]
    pub(crate) fn new(vm: &VirtualMachine) -> Self {
        Self {
            interpreter_id: vm.state.interpreter_id,
            #[cfg(feature = "threading")]
            lock: alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Run an operation while its section is the active top section.
    ///
    /// Calling Python can suspend this lock, so revalidate state afterward.
    /// This API deliberately exposes no guard or data reference. Traversal
    /// uses separate short payload locks; there is no stopped-world bypass.
    #[cfg_attr(
        not(all(test, feature = "threading")),
        expect(dead_code, reason = "Removed when the profiler consumer is wired")
    )]
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
    use alloc::{rc::Rc, sync::Arc};
    use core::{
        cell::RefCell,
        marker::PhantomData,
        sync::atomic::{AtomicBool, Ordering},
    };
    use std::collections::HashMap;

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

#[cfg(test)]
#[cfg(feature = "threading")]
mod tests {
    use super::*;
    #[cfg(all(unix, feature = "host_env"))]
    use crate::AsObject;
    use crate::Interpreter;
    use alloc::sync::Arc;
    use core::{
        panic::AssertUnwindSafe,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn nested_sections_restore_ownership() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let first = CriticalSectionLock::new(vm);
                let second = CriticalSectionLock::new(vm);
                first.with(vm, || {
                    first.with(vm, || assert!(first.lock.load(Ordering::Acquire)));
                    second.with(vm, || {
                        first.with(vm, || {
                            assert!(first.lock.load(Ordering::Acquire));
                            assert!(!second.lock.load(Ordering::Acquire));
                        });
                        assert!(second.lock.load(Ordering::Acquire));
                        assert!(!first.lock.load(Ordering::Acquire));
                    });
                    assert!(first.lock.load(Ordering::Acquire));
                });
                assert!(!first.lock.load(Ordering::Acquire));
                assert!(!second.lock.load(Ordering::Acquire));
                assert_empty(vm.state.interpreter_id);
            });
    }

    #[test]
    fn detachment_and_unwind_restore_sections() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let lock = CriticalSectionLock::new(vm);
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    lock.with(vm, || {
                        vm.allow_threads(|| assert!(!lock.lock.load(Ordering::Acquire)));
                        assert!(lock.lock.load(Ordering::Acquire));
                        panic!("unwind the operation scope");
                    });
                }));
                assert!(result.is_err());
                assert!(!lock.lock.load(Ordering::Acquire));
                assert_empty(vm.state.interpreter_id);
                lock.with(vm, || assert!(lock.lock.load(Ordering::Acquire)));
            });
    }

    #[test]
    fn interpreter_switch_restores_only_its_own_scopes() {
        let first = Interpreter::builder(Default::default()).build();
        let second = Interpreter::builder(Default::default()).build();
        first.enter(|first_vm| {
            let first_lock = CriticalSectionLock::new(first_vm);
            first_lock.with(first_vm, || {
                second.enter(|second_vm| {
                    assert!(!first_lock.lock.load(Ordering::Acquire));
                    let second_lock = CriticalSectionLock::new(second_vm);
                    second_lock.with(second_vm, || {
                        first.enter(|restored_vm| {
                            assert!(first_lock.lock.load(Ordering::Acquire));
                            assert!(!second_lock.lock.load(Ordering::Acquire));
                            first_lock.with(restored_vm, || {});
                        });
                        assert!(second_lock.lock.load(Ordering::Acquire));
                        let mismatch = std::panic::catch_unwind(AssertUnwindSafe(|| {
                            first_lock.with(second_vm, || {});
                        }));
                        assert!(mismatch.is_err());
                    });
                });
                assert!(first_lock.lock.load(Ordering::Acquire));
            });
        });
    }

    #[test]
    fn opposite_order_contention_completes() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let first = Arc::new(CriticalSectionLock::new(vm));
                let second = Arc::new(CriticalSectionLock::new(vm));
                let completed = Arc::new(AtomicUsize::new(0));
                let workers: Vec<_> = [false, true]
                    .into_iter()
                    .map(|reverse| {
                        let (outer, inner) = if reverse {
                            (second.clone(), first.clone())
                        } else {
                            (first.clone(), second.clone())
                        };
                        let completed = completed.clone();
                        vm.start_thread(move |vm| {
                            for _ in 0..1000 {
                                outer.with(vm, || {
                                    inner.with(vm, || {
                                        completed.fetch_add(1, Ordering::Relaxed);
                                    })
                                });
                            }
                        })
                    })
                    .collect();
                vm.allow_threads(|| {
                    for worker in workers {
                        worker.join().unwrap();
                    }
                });
                assert_eq!(completed.load(Ordering::Relaxed), 2000);
                assert!(!first.lock.load(Ordering::Acquire));
                assert!(!second.lock.load(Ordering::Acquire));
            });
    }

    #[test]
    fn stop_the_world_releases_operation_ownership() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let lock = Arc::new(CriticalSectionLock::new(vm));
                let finished = Arc::new(core::sync::atomic::AtomicBool::new(false));
                let (ready, started) = std::sync::mpsc::channel();
                let worker = {
                    let lock = lock.clone();
                    let finished = finished.clone();
                    vm.start_thread(move |vm| {
                        lock.with(vm, || {
                            ready.send(()).unwrap();
                            while !finished.load(Ordering::Acquire) {
                                crate::vm::thread::suspend_if_needed(&vm.state);
                                std::thread::yield_now();
                            }
                            assert!(lock.lock.load(Ordering::Acquire));
                        });
                    })
                };
                vm.allow_threads(|| started.recv().unwrap());
                let mut stopped = crate::vm::stop_the_world::StopTheWorldGroup::new();
                assert!(!lock.lock.load(Ordering::Acquire));
                finished.store(true, Ordering::Release);
                stopped.restart();
                vm.allow_threads(|| worker.join().unwrap());
                assert!(!lock.lock.load(Ordering::Acquire));
            });
    }

    // Run this exact test alone in an owned process under an external timeout.
    // It forks while an operation is active in another interpreter, and keeps
    // lexical scopes in both interpreters alive across the child's slot repair.
    #[cfg(all(unix, feature = "host_env"))]
    #[test]
    #[ignore = "requires an isolated process and external process-tree timeout"]
    fn fork_preserves_nested_interpreter_scopes() {
        let first = Interpreter::builder(Default::default()).build();
        let second = Interpreter::builder(Default::default()).build();
        first.enter(|first_vm| {
            let outer = CriticalSectionLock::new(first_vm);
            let worker_lock = Arc::new(CriticalSectionLock::new(first_vm));
            let finished = Arc::new(core::sync::atomic::AtomicBool::new(false));
            let (ready, started) = std::sync::mpsc::channel();
            let worker = {
                let lock = worker_lock.clone();
                let finished = finished.clone();
                first_vm.start_thread(move |vm| {
                    lock.with(vm, || {
                        ready.send(()).unwrap();
                        while !finished.load(Ordering::Acquire) {
                            crate::vm::thread::suspend_if_needed(&vm.state);
                            std::thread::yield_now();
                        }
                    });
                })
            };
            first_vm.allow_threads(|| started.recv().unwrap());
            outer.with(first_vm, || {
                second.enter(|vm| {
                    let inner = CriticalSectionLock::new(vm);
                    let posix = vm.import("posix", 0).unwrap();
                    let lookups = Arc::new(AtomicUsize::new(0));
                    let hook = {
                        let lookups = lookups.clone();
                        vm.new_function(
                            "__getattr__",
                            move |_name: crate::PyObjectRef,
                                  vm: &VirtualMachine|
                                  -> crate::PyResult {
                                // Stream repair can reenter Python. Its native
                                // callback must be able to acquire a fresh STW
                                // transaction after core child repair.
                                let mut stopped =
                                    crate::vm::stop_the_world::StopTheWorldGroup::new();
                                stopped.restart();
                                lookups.fetch_add(1, Ordering::Relaxed);
                                Err(vm.new_attribute_error("missing test stream"))
                            },
                        )
                    };
                    vm.sys_module.set_attr("__getattr__", hook, vm).unwrap();
                    let _ = vm.sys_module.dict().del_item("stdin", vm);
                    inner.with(vm, || {
                        lookups.store(0, Ordering::Relaxed);
                        let pid = vm
                            .call_method(posix.as_object(), "fork", ())
                            .unwrap()
                            .try_into_value::<i32>(vm)
                            .unwrap();
                        if pid == 0 {
                            assert!(lookups.load(Ordering::Relaxed) > 0);
                            assert!(inner.lock.load(Ordering::Acquire));
                            assert!(!worker_lock.lock.load(Ordering::Acquire));
                            first.enter(|vm| {
                                assert!(outer.lock.load(Ordering::Acquire));
                                assert!(!inner.lock.load(Ordering::Acquire));
                                worker_lock.with(vm, || {});
                            });
                            assert!(inner.lock.load(Ordering::Acquire));
                            // The inherited shared transaction was released.
                            let mut stopped = crate::vm::stop_the_world::StopTheWorldGroup::new();
                            stopped.restart();
                            // SAFETY: this owned child must not return into the
                            // parent test runner or join the vanished worker.
                            unsafe { libc::_exit(0) };
                        }
                        finished.store(true, Ordering::Release);
                        let mut status = 0;
                        vm.allow_threads(|| {
                            rustpython_host_env::posix::waitpid(pid, &mut status, 0).unwrap();
                        });
                        assert!(libc::WIFEXITED(status));
                        assert_eq!(libc::WEXITSTATUS(status), 0);
                    });
                });
                assert!(outer.lock.load(Ordering::Acquire));
            });
            first_vm.allow_threads(|| worker.join().unwrap());
        });
    }

    // The worker deliberately remains parked until the owned test process exits.
    #[test]
    #[ignore = "requires an isolated process and external process-tree timeout"]
    fn shutting_down_releases_operation_ownership() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let lock = Arc::new(CriticalSectionLock::new(vm));
                let (ready, started) = std::sync::mpsc::channel();
                let worker = {
                    let lock = lock.clone();
                    vm.start_thread(move |vm| {
                        lock.with(vm, || {
                            ready
                                .send(crate::vm::thread::current_thread_slot().unwrap())
                                .unwrap();
                            crate::vm::thread::hang_current_thread(&vm.state);
                        });
                    })
                };
                let slot = vm.allow_threads(|| started.recv().unwrap());
                vm.allow_threads(|| {
                    while slot.state.load(Ordering::Acquire)
                        != crate::vm::thread::ThreadState::ShuttingDown as i32
                    {
                        std::thread::yield_now();
                    }
                });
                assert!(!lock.lock.load(Ordering::Acquire));
                lock.with(vm, || assert!(lock.lock.load(Ordering::Acquire)));
                drop(worker);
            });
    }
}

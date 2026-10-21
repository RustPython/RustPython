//! Critical sections over [`PyMutex`]es.
//!
//! Each thread state keeps a stack of the critical sections it has entered.
//! Detaching releases the mutexes of every section on the stack, and attaching
//! again reacquires only the top-most one; the others are reacquired as the
//! sections above them end. Code inside a section must therefore revalidate
//! its state after anything that can detach, such as calling Python.
//!
//! Without the `threading` feature critical sections are no-ops.

use super::lock::PyMutex;
use crate::VirtualMachine;

/// `Py_BEGIN_CRITICAL_SECTION_MUTEX(m)` ... `Py_END_CRITICAL_SECTION()`
#[cfg_attr(
    not(all(test, feature = "threading")),
    expect(dead_code, reason = "Removed when the profiler consumer is wired")
)]
pub(crate) fn with<R>(vm: &VirtualMachine, m: &PyMutex, f: impl FnOnce() -> R) -> R {
    #[cfg(feature = "threading")]
    {
        threaded::with(vm, m, f)
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = (vm, m);
        f()
    }
}

/// `Py_BEGIN_CRITICAL_SECTION2_MUTEX(m1, m2)` ... `Py_END_CRITICAL_SECTION2()`
#[cfg_attr(
    not(all(test, feature = "threading")),
    expect(dead_code, reason = "Removed when the profiler consumer is wired")
)]
pub(crate) fn with2<R>(
    vm: &VirtualMachine,
    m1: &PyMutex,
    m2: &PyMutex,
    f: impl FnOnce() -> R,
) -> R {
    #[cfg(feature = "threading")]
    {
        threaded::with2(vm, m1, m2, f)
    }
    #[cfg(not(feature = "threading"))]
    {
        let _ = (vm, m1, m2);
        f()
    }
}

#[cfg(feature = "threading")]
pub(super) use threaded::{resume, suspend_all};

#[cfg(feature = "threading")]
mod threaded {
    use super::PyMutex;
    use crate::VirtualMachine;
    use crate::vm::thread::{self, ThreadSlot};
    use core::ptr;
    use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

    /// The section's mutexes are released; it is resumed when it becomes the
    /// top-most section of an attached thread.
    const INACTIVE: usize = 0x1;
    /// The tagged pointer refers to a [`PyCriticalSection2`].
    const TWO_MUTEXES: usize = 0x2;
    const MASK: usize = 0x3;

    /// `PyCriticalSection`
    ///
    /// Only the owning thread touches a section; the fields are atomics
    /// because the thread state that links them is shared.
    #[repr(C, align(4))]
    struct PyCriticalSection {
        /// Tagged pointer to the previous critical section.
        prev: AtomicUsize,
        /// Mutex held by this section. Null while it is being acquired, and
        /// for a section that skipped locking.
        mutex: AtomicPtr<PyMutex>,
    }

    /// `PyCriticalSection2`
    #[repr(C)]
    struct PyCriticalSection2 {
        base: PyCriticalSection,
        mutex2: AtomicPtr<PyMutex>,
    }

    impl PyCriticalSection {
        const fn new() -> Self {
            Self {
                prev: AtomicUsize::new(0),
                mutex: AtomicPtr::new(ptr::null_mut()),
            }
        }
    }

    fn mutex_ptr(m: &PyMutex) -> *mut PyMutex {
        ptr::from_ref(m).cast_mut()
    }

    /// # Safety
    /// A non-null `m` must be a mutex of a live critical section.
    unsafe fn lock(m: *mut PyMutex) {
        unsafe { &*m }.lock();
    }

    /// # Safety
    /// A non-null `m` must be a mutex of a live critical section.
    unsafe fn unlock(m: *mut PyMutex) {
        unsafe { &*m }.unlock();
    }

    fn untag(tag: usize) -> *const PyCriticalSection {
        (tag & !MASK) as *const PyCriticalSection
    }

    /// `_PyCriticalSection_IsActive`
    fn is_active(tag: usize) -> bool {
        tag != 0 && tag & INACTIVE == 0
    }

    fn tstate() -> thread::CurrentFrameSlot {
        thread::current_thread_slot().expect("critical section requires a thread state")
    }

    fn world_stopped(vm: &VirtualMachine) -> bool {
        vm.state
            .stop_the_world
            .world_stopped
            .load(Ordering::Acquire)
    }

    pub(super) fn with<R>(vm: &VirtualMachine, m: &PyMutex, f: impl FnOnce() -> R) -> R {
        let c = PyCriticalSection::new();
        begin_mutex(vm, &tstate(), &c, m);
        // The thread state is looked up again at the end, because fork
        // replaces the slot of the surviving thread.
        let _end = scopeguard::guard(&c, |c| end(&tstate(), c));
        f()
    }

    pub(super) fn with2<R>(
        vm: &VirtualMachine,
        m1: &PyMutex,
        m2: &PyMutex,
        f: impl FnOnce() -> R,
    ) -> R {
        let c = PyCriticalSection2 {
            base: PyCriticalSection::new(),
            mutex2: AtomicPtr::new(ptr::null_mut()),
        };
        begin2_mutex(vm, &tstate(), &c, m1, m2);
        let _end = scopeguard::guard(&c, |c| end2(&tstate(), c));
        f()
    }

    /// `_PyCriticalSection_BeginMutex`
    fn begin_mutex(vm: &VirtualMachine, tstate: &ThreadSlot, c: &PyCriticalSection, m: &PyMutex) {
        if m.lock_fast() {
            c.mutex.store(mutex_ptr(m), Ordering::Relaxed);
            c.prev.store(
                tstate.critical_section.load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            tstate
                .critical_section
                .store(ptr::from_ref(c) as usize, Ordering::Relaxed);
        } else {
            begin_slow(vm, tstate, c, m);
        }
    }

    /// `_PyCriticalSection_BeginSlow`
    #[cold]
    fn begin_slow(vm: &VirtualMachine, tstate: &ThreadSlot, c: &PyCriticalSection, m: &PyMutex) {
        // As an optimisation for locking the same mutex recursively, skip
        // locking if the mutex is currently locked by the top-most critical
        // section. If the top-most critical section is a two-mutex critical
        // section, then locking is skipped if either mutex is m.
        let top = tstate.critical_section.load(Ordering::Relaxed);
        if top != 0 {
            // SAFETY: the stack links live sections of this thread.
            let prev = unsafe { &*untag(top) };
            if prev.mutex.load(Ordering::Relaxed) == mutex_ptr(m) {
                c.mutex.store(ptr::null_mut(), Ordering::Relaxed);
                c.prev.store(0, Ordering::Relaxed);
                return;
            }
            if top & TWO_MUTEXES != 0 {
                // SAFETY: the tag marks a two-mutex section.
                let prev2 = unsafe { &*untag(top).cast::<PyCriticalSection2>() };
                if prev2.mutex2.load(Ordering::Relaxed) == mutex_ptr(m) {
                    c.mutex.store(ptr::null_mut(), Ordering::Relaxed);
                    c.prev.store(0, Ordering::Relaxed);
                    return;
                }
            }
        }
        // If the world is stopped, we don't need to acquire the lock because
        // there are no other threads that could be accessing the object.
        // Without this check, acquiring a critical section while the world is
        // stopped could lead to a deadlock.
        if world_stopped(vm) {
            c.mutex.store(ptr::null_mut(), Ordering::Relaxed);
            c.prev.store(0, Ordering::Relaxed);
            return;
        }
        c.mutex.store(ptr::null_mut(), Ordering::Relaxed);
        c.prev.store(top, Ordering::Relaxed);
        tstate
            .critical_section
            .store(ptr::from_ref(c) as usize, Ordering::Relaxed);

        m.lock();
        c.mutex.store(mutex_ptr(m), Ordering::Relaxed);
    }

    /// `_PyCriticalSection2_BeginMutex`
    fn begin2_mutex(
        vm: &VirtualMachine,
        tstate: &ThreadSlot,
        c: &PyCriticalSection2,
        m1: &PyMutex,
        m2: &PyMutex,
    ) {
        if ptr::eq(m1, m2) {
            // If the two mutex arguments are the same, treat this as a
            // critical section with a single mutex.
            c.mutex2.store(ptr::null_mut(), Ordering::Relaxed);
            begin_mutex(vm, tstate, &c.base, m1);
            return;
        }

        // Sort the mutexes so that the lower address is locked first. The
        // exact order does not matter, but we need to acquire the mutexes in
        // a consistent order to avoid lock ordering deadlocks.
        let (m1, m2) = if mutex_ptr(m2) < mutex_ptr(m1) {
            (m2, m1)
        } else {
            (m1, m2)
        };

        if m1.lock_fast() {
            if m2.lock_fast() {
                c.base.mutex.store(mutex_ptr(m1), Ordering::Relaxed);
                c.mutex2.store(mutex_ptr(m2), Ordering::Relaxed);
                c.base.prev.store(
                    tstate.critical_section.load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
                tstate
                    .critical_section
                    .store(ptr::from_ref(c) as usize | TWO_MUTEXES, Ordering::Relaxed);
            } else {
                begin2_slow(vm, tstate, c, m1, m2, true);
            }
        } else {
            begin2_slow(vm, tstate, c, m1, m2, false);
        }
    }

    /// `_PyCriticalSection2_BeginSlow`
    #[cold]
    fn begin2_slow(
        vm: &VirtualMachine,
        tstate: &ThreadSlot,
        c: &PyCriticalSection2,
        m1: &PyMutex,
        m2: &PyMutex,
        is_m1_locked: bool,
    ) {
        if world_stopped(vm) {
            c.base.mutex.store(ptr::null_mut(), Ordering::Relaxed);
            c.mutex2.store(ptr::null_mut(), Ordering::Relaxed);
            c.base.prev.store(0, Ordering::Relaxed);
            return;
        }
        // Same optimization as in begin_slow: skip locking when recursively
        // acquiring the same locks.
        let top = tstate.critical_section.load(Ordering::Relaxed);
        if top != 0 && top & TWO_MUTEXES != 0 {
            // SAFETY: the tag marks a live two-mutex section of this thread.
            let prev2 = unsafe { &*untag(top).cast::<PyCriticalSection2>() };
            debug_assert!(mutex_ptr(m1) < mutex_ptr(m2));
            debug_assert!(
                prev2.base.mutex.load(Ordering::Relaxed) < prev2.mutex2.load(Ordering::Relaxed)
            );
            if prev2.base.mutex.load(Ordering::Relaxed) == mutex_ptr(m1)
                && prev2.mutex2.load(Ordering::Relaxed) == mutex_ptr(m2)
            {
                c.base.mutex.store(ptr::null_mut(), Ordering::Relaxed);
                c.mutex2.store(ptr::null_mut(), Ordering::Relaxed);
                c.base.prev.store(0, Ordering::Relaxed);
                return;
            }
        }
        c.base.mutex.store(ptr::null_mut(), Ordering::Relaxed);
        c.mutex2.store(ptr::null_mut(), Ordering::Relaxed);
        c.base.prev.store(top, Ordering::Relaxed);
        tstate
            .critical_section
            .store(ptr::from_ref(c) as usize | TWO_MUTEXES, Ordering::Relaxed);

        if !is_m1_locked {
            m1.lock();
        }
        m2.lock();
        c.base.mutex.store(mutex_ptr(m1), Ordering::Relaxed);
        c.mutex2.store(mutex_ptr(m2), Ordering::Relaxed);
    }

    /// `_PyCriticalSection_Pop`: removes the top-most critical section from
    /// the thread's stack. If the new top-most section is inactive, then it
    /// is resumed.
    fn pop(tstate: &ThreadSlot, c: &PyCriticalSection) {
        let prev = c.prev.load(Ordering::Relaxed);
        tstate.critical_section.store(prev, Ordering::Relaxed);
        if prev & INACTIVE != 0 {
            resume_top(tstate);
        }
    }

    /// `_PyCriticalSection_End`
    fn end(tstate: &ThreadSlot, c: &PyCriticalSection) {
        // If the mutex is null, we used the fast path in begin_slow for locks
        // already held in the top-most critical section, and we shouldn't
        // unlock or pop this critical section.
        let m = c.mutex.load(Ordering::Relaxed);
        if m.is_null() {
            return;
        }
        // SAFETY: an active section holds its mutex.
        unsafe { unlock(m) };
        pop(tstate, c);
    }

    /// `_PyCriticalSection2_End`
    fn end2(tstate: &ThreadSlot, c: &PyCriticalSection2) {
        // If mutex1 is null, we used the fast path in either begin_slow or
        // begin2_slow for mutexes that are already held, and mutex2 should
        // also be null.
        let m1 = c.base.mutex.load(Ordering::Relaxed);
        if m1.is_null() {
            debug_assert!(c.mutex2.load(Ordering::Relaxed).is_null());
            return;
        }
        let m2 = c.mutex2.load(Ordering::Relaxed);
        // SAFETY: an active section holds its mutexes.
        unsafe {
            if !m2.is_null() {
                unlock(m2);
            }
            unlock(m1);
        }
        pop(tstate, &c.base);
    }

    /// `_PyCriticalSection_SuspendAll`: release all locks held by critical
    /// sections. Called before the thread stops being ATTACHED.
    pub(crate) fn suspend_all(tstate: &ThreadSlot) {
        let mut tagptr = &tstate.critical_section;
        loop {
            let tag = tagptr.load(Ordering::Relaxed);
            if !is_active(tag) {
                break;
            }
            // SAFETY: the stack links live sections of this thread.
            let c = unsafe { &*untag(tag) };
            let m = c.mutex.load(Ordering::Relaxed);
            if !m.is_null() {
                // SAFETY: an active section holds its mutexes.
                unsafe { unlock(m) };
                if tag & TWO_MUTEXES != 0 {
                    // SAFETY: the tag marks a two-mutex section.
                    let c2 = unsafe { &*untag(tag).cast::<PyCriticalSection2>() };
                    let m2 = c2.mutex2.load(Ordering::Relaxed);
                    if !m2.is_null() {
                        // SAFETY: as above.
                        unsafe { unlock(m2) };
                    }
                }
            }

            tagptr.store(tag | INACTIVE, Ordering::Relaxed);
            tagptr = &c.prev;
        }
    }

    /// Resume the top-most critical section after the thread attached.
    pub(crate) fn resume(tstate: &ThreadSlot) {
        if tstate.critical_section.load(Ordering::Relaxed) != 0 {
            resume_top(tstate);
        }
    }

    /// `_PyCriticalSection_Resume`
    fn resume_top(tstate: &ThreadSlot) {
        let p = tstate.critical_section.load(Ordering::Relaxed);
        // SAFETY: the stack links live sections of this thread.
        let c = unsafe { &*untag(p) };
        debug_assert!(!is_active(p));

        let m1 = c.mutex.swap(ptr::null_mut(), Ordering::Relaxed);
        let (c2, m2) = if p & TWO_MUTEXES != 0 {
            // SAFETY: the tag marks a two-mutex section.
            let c2 = unsafe { &*untag(p).cast::<PyCriticalSection2>() };
            (Some(c2), c2.mutex2.swap(ptr::null_mut(), Ordering::Relaxed))
        } else {
            (None, ptr::null_mut())
        };

        // SAFETY: the mutexes belong to a live section that released them.
        unsafe {
            if !m1.is_null() {
                lock(m1);
            }
            if !m2.is_null() {
                lock(m2);
            }
        }

        c.mutex.store(m1, Ordering::Relaxed);
        if let Some(c2) = c2
            && !m2.is_null()
        {
            c2.mutex2.store(m2, Ordering::Relaxed);
        }

        let p = tstate.critical_section.load(Ordering::Relaxed);
        tstate
            .critical_section
            .store(p & !INACTIVE, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[cfg(feature = "threading")]
mod tests {
    use super::*;
    #[cfg(all(unix, feature = "host_env"))]
    use crate::AsObject;
    use crate::Interpreter;
    use crate::vm::stop_the_world::StopTheWorldGroup;
    use alloc::sync::Arc;
    use core::{
        panic::AssertUnwindSafe,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    fn assert_empty() {
        let slot = crate::vm::thread::current_thread_slot().unwrap();
        assert_eq!(slot.critical_section.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn nested_sections_restore_ownership() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let first = PyMutex::new();
                let second = PyMutex::new();
                with(vm, &first, || {
                    with(vm, &first, || assert!(first.is_locked()));
                    with(vm, &second, || {
                        assert!(first.is_locked());
                        // Not the top-most section: locking it again detaches,
                        // which suspends every section of this thread.
                        with(vm, &first, || {
                            assert!(first.is_locked());
                            assert!(!second.is_locked());
                        });
                        assert!(second.is_locked());
                        assert!(!first.is_locked());
                    });
                    assert!(first.is_locked());
                });
                assert!(!first.is_locked());
                assert!(!second.is_locked());
                assert_empty();
            });
    }

    #[test]
    fn two_mutex_sections() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let first = PyMutex::new();
                let second = PyMutex::new();
                with2(vm, &second, &first, || {
                    assert!(first.is_locked() && second.is_locked());
                    with2(vm, &first, &second, || {});
                    with(vm, &second, || {});
                    vm.allow_threads(|| assert!(!first.is_locked() && !second.is_locked()));
                    assert!(first.is_locked() && second.is_locked());
                });
                with2(vm, &first, &first, || assert!(first.is_locked()));
                assert!(!first.is_locked() && !second.is_locked());
                assert_empty();
            });
    }

    #[test]
    fn detachment_and_unwind_restore_sections() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let m = PyMutex::new();
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    with(vm, &m, || {
                        vm.allow_threads(|| assert!(!m.is_locked()));
                        assert!(m.is_locked());
                        panic!("unwind the critical section");
                    });
                }));
                assert!(result.is_err());
                assert!(!m.is_locked());
                assert_empty();
                with(vm, &m, || assert!(m.is_locked()));
            });
    }

    #[test]
    fn interpreter_switch_restores_only_its_own_sections() {
        let first = Interpreter::builder(Default::default()).build();
        let second = Interpreter::builder(Default::default()).build();
        first.enter(|first_vm| {
            let first_lock = PyMutex::new();
            with(first_vm, &first_lock, || {
                second.enter(|second_vm| {
                    assert!(!first_lock.is_locked());
                    let second_lock = PyMutex::new();
                    with(second_vm, &second_lock, || {
                        first.enter(|restored_vm| {
                            assert!(first_lock.is_locked());
                            assert!(!second_lock.is_locked());
                            with(restored_vm, &first_lock, || {});
                        });
                        assert!(second_lock.is_locked());
                    });
                });
                assert!(first_lock.is_locked());
            });
        });
    }

    #[test]
    fn opposite_order_contention_completes() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let first = Arc::new(PyMutex::new());
                let second = Arc::new(PyMutex::new());
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
                                with(vm, &outer, || {
                                    with(vm, &inner, || {
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
                assert!(!first.is_locked());
                assert!(!second.is_locked());
            });
    }

    #[test]
    fn stop_the_world_releases_critical_sections() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let m = Arc::new(PyMutex::new());
                let finished = Arc::new(AtomicBool::new(false));
                let (ready, started) = std::sync::mpsc::channel();
                let worker = {
                    let m = m.clone();
                    let finished = finished.clone();
                    vm.start_thread(move |vm| {
                        with(vm, &m, || {
                            ready.send(()).unwrap();
                            while !finished.load(Ordering::Acquire) {
                                crate::vm::thread::suspend_if_needed(&vm.state);
                                std::thread::yield_now();
                            }
                            assert!(m.is_locked());
                        });
                    })
                };
                vm.allow_threads(|| started.recv().unwrap());
                let mut stopped = StopTheWorldGroup::new();
                assert!(!m.is_locked());
                finished.store(true, Ordering::Release);
                stopped.restart();
                vm.allow_threads(|| worker.join().unwrap());
                assert!(!m.is_locked());
            });
    }

    #[test]
    fn stopped_world_skips_locking() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let m = PyMutex::new();
                m.lock();
                let mut stopped = StopTheWorldGroup::new();
                with(vm, &m, assert_empty);
                stopped.restart();
                m.unlock();
            });
    }

    // Run this exact test alone in an owned process under an external timeout.
    // It forks while a critical section is active in another interpreter, and
    // keeps lexical sections in both interpreters alive across the child's
    // slot repair.
    #[cfg(all(unix, feature = "host_env"))]
    #[test]
    #[ignore = "requires an isolated process and external process-tree timeout"]
    fn fork_preserves_nested_interpreter_sections() {
        let first = Interpreter::builder(Default::default()).build();
        let second = Interpreter::builder(Default::default()).build();
        first.enter(|first_vm| {
            let outer = PyMutex::new();
            let worker_lock = Arc::new(PyMutex::new());
            let finished = Arc::new(AtomicBool::new(false));
            let (ready, started) = std::sync::mpsc::channel();
            let worker = {
                let m = worker_lock.clone();
                let finished = finished.clone();
                first_vm.start_thread(move |vm| {
                    with(vm, &m, || {
                        ready.send(()).unwrap();
                        while !finished.load(Ordering::Acquire) {
                            crate::vm::thread::suspend_if_needed(&vm.state);
                            std::thread::yield_now();
                        }
                    });
                })
            };
            first_vm.allow_threads(|| started.recv().unwrap());
            with(first_vm, &outer, || {
                second.enter(|vm| {
                    let inner = PyMutex::new();
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
                                let mut stopped = StopTheWorldGroup::new();
                                stopped.restart();
                                lookups.fetch_add(1, Ordering::Relaxed);
                                Err(vm.new_attribute_error("missing test stream"))
                            },
                        )
                    };
                    vm.sys_module.set_attr("__getattr__", hook, vm).unwrap();
                    let _ = vm.sys_module.dict().del_item("stdin", vm);
                    with(vm, &inner, || {
                        lookups.store(0, Ordering::Relaxed);
                        let pid = vm
                            .call_method(posix.as_object(), "fork", ())
                            .unwrap()
                            .try_into_value::<i32>(vm)
                            .unwrap();
                        if pid == 0 {
                            assert!(lookups.load(Ordering::Relaxed) > 0);
                            assert!(inner.is_locked());
                            assert!(!worker_lock.is_locked());
                            first.enter(|vm| {
                                assert!(outer.is_locked());
                                assert!(!inner.is_locked());
                                with(vm, &worker_lock, || {});
                            });
                            assert!(inner.is_locked());
                            // The inherited shared transaction was released.
                            let mut stopped = StopTheWorldGroup::new();
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
                assert!(outer.is_locked());
            });
            first_vm.allow_threads(|| worker.join().unwrap());
        });
    }

    // The worker deliberately remains parked until the owned test process exits.
    #[test]
    #[ignore = "requires an isolated process and external process-tree timeout"]
    fn shutting_down_releases_critical_sections() {
        Interpreter::builder(Default::default())
            .build()
            .enter(|vm| {
                let m = Arc::new(PyMutex::new());
                let (ready, started) = std::sync::mpsc::channel();
                let worker = {
                    let m = m.clone();
                    vm.start_thread(move |vm| {
                        with(vm, &m, || {
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
                assert!(!m.is_locked());
                with(vm, &m, || assert!(m.is_locked()));
                drop(worker);
            });
    }
}

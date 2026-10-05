//! Process-wide quiescence used only around the native fork syscall.

use super::{PyGlobalState, VirtualMachine, runtime};
use crate::common::{
    lock::{PyDetachingRwLock, PyDetachingRwLockReadGuard},
    rc::PyRc,
};
use core::{cell::RefCell, sync::atomic::Ordering};

static LIFECYCLE: PyDetachingRwLock<()> = PyDetachingRwLock::new(());
static GENERATION: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

thread_local! {
    static NATIVE_PHASE: RefCell<(usize, Option<PyDetachingRwLockReadGuard<'static, ()>>)> =
        const { RefCell::new((0, None)) };
}

/// Bootstrap and terminal heap retirement cannot be interrupted halfway through
/// publication. Nested native destruction shares one read permit.
pub(crate) struct NativePhase(core::marker::PhantomData<alloc::rc::Rc<()>>);

pub(crate) fn native_phase() -> NativePhase {
    NATIVE_PHASE.with(|phase| {
        if phase.borrow().0 == 0 {
            let guard = LIFECYCLE.read();
            phase.borrow_mut().1 = Some(guard);
        }
        phase.borrow_mut().0 += 1;
    });
    NativePhase(core::marker::PhantomData)
}

impl Drop for NativePhase {
    fn drop(&mut self) {
        NATIVE_PHASE.with(|phase| {
            let mut phase = phase.borrow_mut();
            phase.0 -= 1;
            if phase.0 == 0 {
                phase.1.take();
            }
        });
    }
}

struct Stopped(Vec<PyRc<PyGlobalState>>);

/// Read a native namespace without hashing or invoking user attribute hooks.
/// Fork repair must finish before any Python callback may create a new thread.
pub(crate) fn namespace_item(
    dict: &crate::Py<crate::builtins::PyDict>,
    name: &str,
) -> Option<crate::PyObjectRef> {
    dict.into_iter().find_map(|(key, value)| {
        key.downcast_ref::<crate::builtins::PyUtf8Str>()
            .is_some_and(|key| key.as_str() == name)
            .then_some(value)
    })
}

impl Drop for Stopped {
    fn drop(&mut self) {
        for state in self.0.iter().rev() {
            state.stop_the_world.start_the_world(state);
        }
    }
}

/// The closure must perform only the syscall and return whether this is its
/// child branch. Python at-fork hooks run outside all these guards.
pub(crate) fn with_fork<R>(vm: &VirtualMachine, syscall: impl FnOnce() -> (bool, R)) -> R {
    assert_eq!(NATIVE_PHASE.with(|phase| phase.borrow().0), 0);
    // Declare the snapshot before the gate: its last Arc can retire a heap,
    // which takes a read permit and therefore must run after releasing write.
    let states;
    let gc = crate::gc_state::gc_state();
    // A running collector can enter bootstrap/teardown from a callback.
    // Finish it before excluding those native lifecycle phases.
    let retired = vm.allow_threads(|| gc.lock_for_fork());
    let gate = LIFECYCLE.write();
    states = runtime::live_interpreter_states();
    let mut stopped = Stopped(Vec::with_capacity(states.len()));
    for state in &states {
        // Waiting for a foreign collector must detach the *current* owner.
        // suspend_if_needed(foreign_state) cannot park its slot correctly.
        vm.allow_threads(|| state.stop_the_world.stop_the_world(state));
        stopped.0.push(state.clone());
    }
    let readers = super::thread::surviving_qsbr_slots();
    // Native lookup/anchor cloning may run without a Python attachment. Hold
    // both maps across fork rather than resetting a possibly interrupted map.
    let registries = runtime::prepare_fork();
    let mut owners: Vec<_> = states
        .iter()
        .map(|state| state.owner_leases.prepare_fork())
        .collect();
    let result = crate::stdlib::_interpchannels::with_fork(|| {
        crate::stdlib::_interpqueues::with_fork(|| {
            let roots: Vec<_> = states
                .iter()
                .map(|state| state.roots.prepare_fork())
                .collect();
            let mut domain = crate::object::qsbr::QSBR.prepare_fork();
            let (child, result) = syscall();
            if child {
                GENERATION.fetch_add(1, Ordering::Relaxed);
                for (state, owners) in states.iter().zip(&mut owners) {
                    // All registries were stabilized before fork. Only this thread
                    // remains, including any nested/saved native entry scopes.
                    let (count, teardowns) = unsafe { owners.repair_child() };
                    state.owners.store(count, Ordering::Relaxed);
                    state.teardowns.store(teardowns, Ordering::Relaxed);
                }
                unsafe { domain.repair_child(&readers) };
                for state in &stopped.0 {
                    state.stop_the_world.reset_after_fork();
                }
                stopped.0.clear();
            }
            drop(domain);
            drop(roots);
            result
        })
    });
    drop(owners);
    drop(registries);
    drop(stopped);
    drop(gate);
    drop(retired);
    result
}

/// Retire runtimes that have no surviving native entry, after every internal
/// lock and thread handle has been repaired and the current owner is attached.
pub(crate) fn finish_child(current: &VirtualMachine) {
    #[cfg(feature = "host_env")]
    let active: Vec<_> = runtime::live_interpreter_states()
        .iter()
        .filter(|state| state.interpreter_id != current.state.interpreter_id)
        // SAFETY: these scopes belong to this native thread and surround the
        // fork call, including SavedThreadState scopes outside VM_STACK.
        .filter_map(|state| unsafe { state.owner_leases.active_vm() })
        .collect();
    #[cfg(feature = "host_env")]
    for ptr in &active {
        let vm = unsafe { ptr.as_ref() };
        let _entry = super::thread::VmBootstrapGuard::new(vm);
        unsafe { crate::stdlib::_io::reinit_std_streams_after_fork(vm) };
    }
    let mut cleanup = super::owned::Cleanup::default();
    for state in runtime::live_interpreter_states() {
        let orphaned = state.owners.load(Ordering::Relaxed) == 0
            && state.teardowns.load(Ordering::Relaxed) == 0;
        let pending = state.owner_leases.take_abandoned_cleanups();
        cleanup.run(|| {
            super::thread::native_sweep(|| {
                let _owner = state.gc.allocation_scope();
                let mut steps = super::owned::Cleanup::default();
                if orphaned {
                    state.closed.store(true, Ordering::Release);
                    state.admission_closed.store(true, Ordering::Release);
                    steps.run(|| {
                        crate::stdlib::_interpchannels::clear_interpreter(state.interpreter_id)
                    });
                    steps.run(|| {
                        crate::stdlib::_interpqueues::clear_interpreter(state.interpreter_id)
                    });
                    steps.run(|| state.close_python_roots());
                    steps.run(|| state.gc.unfreeze());
                    steps.run(|| {
                        state.gc.collect_force(2);
                    });
                }
                // Pending owners must be routed through destroy_vm even if a
                // resource destructor fails; plain Box Drop bypasses teardown.
                for vm in pending {
                    steps.run(|| super::thread::destroy_vm(vm));
                }
                steps.finish();
            })
        });
    }
    cleanup.run(runtime::discard_invalid_owned_after_fork);
    #[cfg(feature = "host_env")]
    for ptr in active {
        let vm = unsafe { ptr.as_ref() };
        let _entry = super::thread::VmBootstrapGuard::new(vm);
        // User at-fork callbacks remain scoped to the VM that called fork.
        // Repair the inherited standard-library thread bookkeeping of saved
        // owners before any user child hook can start another native thread.
        let callback = namespace_item(&vm.sys_module.dict(), "modules")
            .and_then(|modules| {
                namespace_item(
                    modules.downcast_ref::<crate::builtins::PyDict>()?,
                    "threading",
                )
            })
            .and_then(|module| {
                namespace_item(
                    &module.downcast_ref::<crate::builtins::PyModule>()?.dict(),
                    "_after_fork",
                )
            });
        if let Some(callback) = callback
            && let Err(error) = callback.call((), vm)
        {
            vm.run_unraisable(error, Some("threading after fork".to_owned()), callback);
        }
    }
    #[cfg(not(feature = "host_env"))]
    let _ = current;
    cleanup.finish();
}

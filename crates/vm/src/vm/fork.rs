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
    let gate = LIFECYCLE.write();
    let gc = crate::gc_state::gc_state();
    let retired = vm.allow_threads(|| gc.lock_retired_for_fork());
    states = runtime::live_interpreter_states();
    let mut stopped = Stopped(Vec::with_capacity(states.len()));
    for state in &states {
        // Waiting for a foreign collector must detach the *current* owner.
        // suspend_if_needed(foreign_state) cannot park its slot correctly.
        vm.allow_threads(|| state.stop_the_world.stop_the_world(state));
        stopped.0.push(state.clone());
    }
    let heaps = gc.heaps_for_fork();
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
            let mut domains: Vec<_> = heaps.iter().map(|heap| heap.qsbr.prepare_fork()).collect();
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
                for domain in &mut domains {
                    unsafe { domain.repair_child(&readers) };
                }
                for state in &stopped.0 {
                    state.stop_the_world.reset_after_fork();
                }
                stopped.0.clear();
            }
            drop(domains);
            drop(roots);
            result
        })
    });
    drop(owners);
    drop(registries);
    drop(stopped);
    drop(retired);
    drop(gate);
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

#[cfg(test)]
#[cfg(all(feature = "host_env", feature = "rustpython-compiler"))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    // Fork in a dedicated test process: the ordinary test harness may be
    // concurrently constructing unrelated interpreters on other native threads.
    fn in_subprocess(test: impl FnOnce()) {
        let thread = std::thread::current();
        let name = thread.name().unwrap();
        if rustpython_host_env::os::var("RUSTPYTHON_FORK_TEST").as_deref() == Ok(name) {
            test();
            return;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "native test harness, not interpreter process access"
        )]
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--no-capture"])
            .env("RUSTPYTHON_FORK_TEST", name)
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + core::time::Duration::from_secs(40);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "fork regression: {status}");
                return;
            }
            if std::time::Instant::now() >= deadline {
                // Include grandchildren if post-fork repair deadlocked.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                panic!("fork regression timed out");
            }
            std::thread::sleep(core::time::Duration::from_millis(20));
        }
    }

    fn fork(vm: &VirtualMachine) -> bool {
        let module = vm.import("posix", 0).unwrap();
        let pid: i32 = module
            .get_attr("fork", vm)
            .unwrap()
            .call((), vm)
            .unwrap()
            .try_to_value(vm)
            .unwrap();
        if pid == 0 {
            return true;
        }
        vm.allow_threads(|| {
            let mut status = 0;
            loop {
                let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
                if waited == -1
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                assert_eq!(waited, pid);
                break;
            }
            assert_eq!(status, 0);
        });
        false
    }

    #[test]
    fn fork_preserves_saved_entries_and_retires_idle_owners() {
        in_subprocess(|| {
            let outer = crate::Interpreter::without_stdlib(Default::default());
            let forking = crate::Interpreter::without_stdlib(Default::default());
            let idle = crate::Interpreter::without_stdlib(Default::default());
            let idle_id = idle.id();
            let owned_id = runtime::store_owned_interpreter(outer.create_subinterpreter());
            let mut child = false;
            outer.enter_raw(|outer_vm| {
                let slot = super::super::thread::current_thread_slot().unwrap();
                let saved = super::super::thread::save_current_thread();
                forking.enter_raw(|vm| {
                    child = fork(vm);
                    if child {
                        assert!(outer_vm.owner_is_valid());
                        assert_eq!(outer_vm.state.owners.load(Ordering::Acquire), 1);
                        assert!(runtime::lookup_open_interpreter(idle_id).is_none());
                        assert!(!runtime::is_owned_interpreter(owned_id));
                        assert!(matches!(
                            idle.enter(|vm| vm.new_int(1).map(|v| v.unbind())),
                            Err(crate::embedding::Error::ForkedOwner)
                        ));
                        vm.state.gc.collect_force(2);
                    }
                });
                super::super::thread::restore_current_thread(saved);
                assert!(super::super::thread::is_current_attached(outer_vm));
                assert!(PyRc::ptr_eq(
                    &slot,
                    &super::super::thread::current_thread_slot().unwrap()
                ));
                outer_vm.state.gc.collect_force(2);
            });
            drop(idle);
            drop(forking);
            drop(outer);
            if child {
                unsafe { libc::_exit(0) };
            }
            drop(runtime::take_owned_interpreter(owned_id));
        });
    }
}

//! Native VM owners that survive independently of registry snapshots.

use crate::common::{lock::PyMutex, rc::PyRc};
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering},
};

#[cfg(not(feature = "threading"))]
use alloc::rc::Weak;
#[cfg(feature = "threading")]
use alloc::sync::Weak;

const OWNED: u8 = 0;
const RELEASING: u8 = 1;
const RELEASED: u8 = 2;
const FINISHED: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnerKind {
    Host,
    #[cfg(feature = "threading")]
    PythonThread,
}

#[derive(Default)]
pub(super) struct OwnerLeases {
    registry: PyMutex<Registry>,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Weak<OwnerLease>>,
    finalizer: Option<Finalizer>,
    // A foreign stale owner must not block in Drop: the finalizer may join
    // that native thread. The finalization permit drains these outside locks.
    #[expect(
        clippy::vec_box,
        reason = "queued VM storage retains its stable address"
    )]
    pending: Vec<Box<super::VirtualMachine>>,
}

#[derive(Clone, Copy)]
struct Finalizer {
    thread: u64,
    reserve_host_admission: bool,
}

impl OwnerLeases {
    #[cfg(test)]
    pub(super) fn register(&self) -> PyRc<OwnerLease> {
        self.register_kind(OwnerKind::Host)
    }

    pub(super) fn register_kind(&self, kind: OwnerKind) -> PyRc<OwnerLease> {
        self.registry.lock().register(OWNED, kind)
    }

    /// # Safety
    /// Use only on the current native thread during fork preparation/repair.
    /// Its active entry scopes must remain alive until the pointer is last used.
    #[cfg(all(unix, feature = "threading"))]
    pub(super) unsafe fn active_vm(&self) -> Option<core::ptr::NonNull<super::VirtualMachine>> {
        self.registry.lock().entries.iter().find_map(|entry| {
            let lease = entry.upgrade()?;
            if lease.is_valid()
                && lease.active.load(Ordering::Acquire) != 0
                && lease.thread.load(Ordering::Acquire) == native_thread_id()
            {
                core::ptr::NonNull::new(lease.vm.load(Ordering::Acquire))
            } else {
                None
            }
        })
    }

    pub(super) fn begin_teardown(
        &self,
        vm: Box<super::VirtualMachine>,
    ) -> Option<(Box<super::VirtualMachine>, PyRc<OwnerLease>, bool)> {
        let mut registry = self.registry.lock();
        let owns_vm = vm.owner_is_valid();
        if !owns_vm
            && registry
                .finalizer
                .is_some_and(|finalizer| finalizer.thread != native_thread_id())
        {
            registry.pending.push(vm);
            return None;
        }
        let lease = if owns_vm {
            let lease = vm.owner_lease().clone();
            lease.begin_teardown();
            lease
        } else {
            registry.register(RELEASED, OwnerKind::Host)
        };
        vm.state.teardowns.fetch_add(1, Ordering::Relaxed);
        Some((vm, lease, owns_vm))
    }

    pub(super) fn try_finalize(&self, state: &PyRc<super::PyGlobalState>) -> Option<Finalization> {
        let mut registry = self.registry.lock();
        if registry.finalizer.is_some() {
            return None;
        }
        let mut owners = 0;
        for lease in registry.entries.iter().filter_map(Weak::upgrade) {
            if !lease.is_valid() || lease.kind != OwnerKind::Host {
                continue;
            }
            match lease.phase.load(Ordering::Acquire) {
                OWNED => owners += 1,
                RELEASING | RELEASED => return None,
                FINISHED => (),
                _ => unreachable!(),
            }
        }
        if owners != 1 {
            return None;
        }
        registry.finalizer = Some(Finalizer {
            thread: native_thread_id(),
            reserve_host_admission: true,
        });
        // Reserve host admission under the registry lock. Python workers
        // must still be able to start children while _shutdown joins them.
        Some(Finalization(state.clone()))
    }

    /// Raw/CLI finalization may still have Python workers to join. It shares
    /// cleanup admission with scoped finalization, without its Busy check.
    pub(super) fn enter_finalization(
        &self,
        state: &PyRc<super::PyGlobalState>,
    ) -> Option<Finalization> {
        let mut registry = self.registry.lock();
        if let Some(finalizer) = registry.finalizer {
            assert_eq!(
                finalizer.thread,
                native_thread_id(),
                "concurrent native finalization"
            );
            return None;
        }
        registry.finalizer = Some(Finalizer {
            thread: native_thread_id(),
            reserve_host_admission: false,
        });
        Some(Finalization(state.clone()))
    }

    #[cfg(all(unix, feature = "threading"))]
    #[expect(clippy::vec_box, reason = "VM storage retains its published address")]
    pub(super) fn take_abandoned_cleanups(&self) -> Vec<Box<super::VirtualMachine>> {
        let mut registry = self.registry.lock();
        if registry.finalizer.is_none() {
            core::mem::take(&mut registry.pending)
        } else {
            Vec::new()
        }
    }

    /// The fork coordinator holds this guard across the syscall, after stopping
    /// mutation of all registered runtimes. Resetting an inherited mutex alone
    /// would not repair a Vec interrupted during a concurrent registration.
    #[cfg(any(all(unix, feature = "threading"), test))]
    pub(super) fn prepare_fork(&self) -> ForkOwners<'_> {
        ForkOwners {
            registry: self.registry.lock(),
        }
    }
}

impl Registry {
    fn register(&mut self, phase: u8, kind: OwnerKind) -> PyRc<OwnerLease> {
        assert!(
            phase != OWNED
                || kind != OwnerKind::Host
                || self
                    .finalizer
                    .is_none_or(|finalizer| !finalizer.reserve_host_admission),
            "cannot create a native owner during interpreter shutdown"
        );
        let lease = PyRc::new(OwnerLease {
            kind,
            valid: AtomicBool::new(true),
            active: AtomicUsize::new(0),
            thread: AtomicU64::new(0),
            phase: AtomicU8::new(phase),
            vm: AtomicPtr::new(core::ptr::null_mut()),
        });
        self.entries.retain(|entry| entry.strong_count() != 0);
        self.entries.push(PyRc::downgrade(&lease));
        lease
    }
}

pub(super) struct Finalization(PyRc<super::PyGlobalState>);

impl Drop for Finalization {
    fn drop(&mut self) {
        let mut cleanup = super::owned::Cleanup::default();
        loop {
            let pending = {
                let mut registry = self.0.owner_leases.registry.lock();
                if registry.pending.is_empty() {
                    registry.finalizer = None;
                    break;
                }
                core::mem::take(&mut registry.pending)
            };
            for vm in pending {
                cleanup.run(|| super::thread::destroy_vm(vm));
            }
        }
        if std::thread::panicking() {
            // Preserve the original panic, including when a secondary panic
            // payload itself has a panicking destructor.
            core::mem::forget(cleanup);
        } else {
            cleanup.finish();
        }
    }
}

pub(super) struct OwnerLease {
    kind: OwnerKind,
    valid: AtomicBool,
    active: AtomicUsize,
    thread: AtomicU64,
    phase: AtomicU8,
    vm: AtomicPtr<super::VirtualMachine>,
}

impl OwnerLease {
    pub(super) fn publish_vm(&self, vm: &super::VirtualMachine) {
        self.vm
            .store(core::ptr::from_ref(vm).cast_mut(), Ordering::Release);
    }

    pub(super) fn retire_vm(&self) {
        self.vm.store(core::ptr::null_mut(), Ordering::Release);
    }

    pub(super) fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }

    pub(super) fn enter(self: &PyRc<Self>) -> OwnerEntry {
        assert!(self.is_valid(), "VM owner did not survive fork");
        let ident = native_thread_id();
        if self.active.fetch_add(1, Ordering::Relaxed) == 0 {
            self.thread.store(ident, Ordering::Release);
        } else {
            assert_eq!(self.thread.load(Ordering::Acquire), ident);
        }
        OwnerEntry {
            lease: self.clone(),
            _thread: PhantomData,
        }
    }

    pub(super) fn begin_teardown(&self) {
        self.phase.store(RELEASING, Ordering::Release);
    }

    pub(super) fn release_owner(&self) {
        self.phase.store(RELEASED, Ordering::Release);
    }

    pub(super) fn finish_teardown(&self) {
        self.phase.store(FINISHED, Ordering::Release);
    }
}

/// Retained until attachment restoration, even after the VM Box was destroyed.
/// Saving/detaching a thread does not end its native entry scope.
pub(super) struct OwnerEntry {
    lease: PyRc<OwnerLease>,
    _thread: PhantomData<alloc::rc::Rc<()>>,
}

impl Drop for OwnerEntry {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.lease.thread.load(Ordering::Acquire),
            native_thread_id()
        );
        let previous = self.lease.active.fetch_sub(1, Ordering::Release);
        debug_assert!(previous != 0);
    }
}

fn native_thread_id() -> u64 {
    #[cfg(feature = "threading")]
    {
        crate::stdlib::_thread::get_ident()
    }
    #[cfg(not(feature = "threading"))]
    {
        // Non-threaded runtimes and their registries are native-thread-local.
        1
    }
}

#[cfg(any(all(unix, feature = "threading"), test))]
pub(super) struct ForkOwners<'a> {
    registry: crate::common::lock::PyMutexGuard<'a, Registry>,
}

#[cfg(any(all(unix, feature = "threading"), test))]
impl ForkOwners<'_> {
    /// # Safety
    /// Only the surviving fork thread may be executing. The guard was acquired
    /// before fork, with owner publication and runtime mutations quiescent.
    pub(super) unsafe fn repair_child(&mut self) -> (usize, usize) {
        let ident = native_thread_id();
        let mut owners = 0;
        let mut teardowns = 0;
        if self
            .registry
            .finalizer
            .is_some_and(|finalizer| finalizer.thread != ident)
        {
            self.registry.finalizer = None;
        }
        self.registry.entries.retain(|entry| {
            let Some(lease) = entry.upgrade() else {
                return false;
            };
            if !lease.is_valid() {
                return false;
            }
            if lease.active.load(Ordering::Acquire) == 0
                || lease.thread.load(Ordering::Acquire) != ident
            {
                lease.valid.store(false, Ordering::Release);
                return false;
            }
            match lease.phase.load(Ordering::Acquire) {
                OWNED => owners += 1,
                RELEASING => {
                    owners += 1;
                    teardowns += 1;
                }
                RELEASED => teardowns += 1,
                FINISHED => return false,
                _ => unreachable!(),
            }
            true
        });
        (owners, teardowns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "threading")]
    #[test]
    fn scoped_admission_preserves_raw_finalization_workers() {
        let interpreter = crate::Interpreter::without_stdlib(Default::default());
        let state = interpreter.enter_raw(|vm| vm.state.clone());
        {
            let _raw = state.owner_leases.enter_finalization(&state);
            let worker = interpreter.new_thread();
            drop(worker);
        }
        let _scoped = state.owner_leases.try_finalize(&state).unwrap();
        let nested = state.owner_leases.enter_finalization(&state);
        assert!(nested.is_none());
        assert!(
            std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| interpreter.new_thread()))
                .is_err()
        );
        let worker = interpreter.enter_raw(super::super::VirtualMachine::new_python_thread);
        drop(worker);
    }

    #[test]
    fn repair_keeps_nested_entries_and_rejects_idle_owners() {
        let registry = OwnerLeases::default();
        let active = registry.register();
        let idle = registry.register();
        let outer = active.enter();
        let inner = active.enter();
        // This native-only registry belongs exclusively to this test.
        assert_eq!(unsafe { registry.prepare_fork().repair_child() }, (1, 0));
        assert!(active.is_valid());
        assert!(!idle.is_valid());
        drop(inner);
        assert_eq!(unsafe { registry.prepare_fork().repair_child() }, (1, 0));
        drop(outer);
        assert!(std::panic::catch_unwind(|| idle.enter()).is_err());
    }

    #[test]
    fn repair_preserves_a_native_destructor_after_owner_release() {
        let registry = OwnerLeases::default();
        let lease = registry.register();
        let entered = lease.enter();
        lease.begin_teardown();
        assert_eq!(unsafe { registry.prepare_fork().repair_child() }, (1, 1));
        lease.release_owner();
        assert_eq!(unsafe { registry.prepare_fork().repair_child() }, (0, 1));
        lease.finish_teardown();
        drop(entered);
    }

    #[test]
    fn repair_invalidates_a_vanished_threads_owner() {
        let registry = OwnerLeases::default();
        let foreign = registry.register();
        // Model the completed atomic publication of a foreign native entry;
        // there is no actual concurrent thread during simulated child repair.
        foreign
            .thread
            .store(native_thread_id() ^ 1, Ordering::Relaxed);
        foreign.active.store(1, Ordering::Relaxed);
        assert_eq!(unsafe { registry.prepare_fork().repair_child() }, (0, 0));
        assert!(!foreign.is_valid());
    }
}

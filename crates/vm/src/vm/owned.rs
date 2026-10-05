//! Explicit owners of VM thread state, independent of registry Arc snapshots.

use super::{VirtualMachine, thread};
use core::{
    cell::{OnceCell, UnsafeCell},
    ops::{Deref, DerefMut},
    sync::atomic::Ordering,
};

/// Finish releasing independent root groups even if a native destructor panics.
/// Each group must remove its roots before destroying their values.
#[derive(Default)]
pub(crate) struct Cleanup {
    panic: Option<Box<dyn core::any::Any + Send>>,
}

impl Cleanup {
    pub(crate) fn run(&mut self, step: impl FnOnce()) {
        if let Err(error) = std::panic::catch_unwind(core::panic::AssertUnwindSafe(step)) {
            if self.panic.is_none() {
                self.panic = Some(error);
            } else {
                // Even a panic payload can have a panicking destructor. Keep
                // the first error and finish retiring runtime roots regardless.
                core::mem::forget(error);
            }
        }
    }

    pub(crate) fn finish(self) {
        if let Some(error) = self.panic {
            std::panic::resume_unwind(error);
        }
    }
}

/// Initialized once while a VM is live; emptied by its exclusive owner before
/// terminal destruction. Reads need no RefCell guard on the call fast paths.
pub(crate) struct ExecutionRoot<T>(UnsafeCell<OnceCell<T>>);

impl<T> Default for ExecutionRoot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ExecutionRoot<T> {
    pub(crate) const fn new() -> Self {
        Self(UnsafeCell::new(OnceCell::new()))
    }

    pub(crate) fn get(&self) -> Option<&T> {
        // SAFETY: initialized roots are immutable throughout ordinary entry.
        unsafe { &*self.0.get() }.get()
    }

    pub(crate) fn set(&self, value: T) -> Result<(), T> {
        // SAFETY: OnceCell initialization only needs shared access.
        unsafe { &*self.0.get() }.set(value)
    }

    /// # Safety
    /// No reference returned by get() may remain. Only VM owner destruction or
    /// consuming finalization, after all execution scopes end, may retire roots.
    pub(crate) unsafe fn retire(&self) -> Option<T> {
        // The VM can remain shared in TLS: only this UnsafeCell is mutated.
        unsafe { &mut *self.0.get() }.take()
    }
}

impl<T: Clone> Clone for ExecutionRoot<T> {
    fn clone(&self) -> Self {
        Self(UnsafeCell::new(unsafe { &*self.0.get() }.clone()))
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for ExecutionRoot<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

pub(super) struct OwnedVm(Option<Box<VirtualMachine>>);

impl OwnedVm {
    pub(super) fn new(vm: VirtualMachine) -> Self {
        assert!(
            !vm.state.closed.load(Ordering::Acquire),
            "interpreter is closed"
        );
        let vm = Box::new(vm);
        assert!(vm.owner_lease.set(vm.state.owner_leases.register()).is_ok());
        vm.owner_lease().publish_vm(&vm);
        vm.state.owners.fetch_add(1, Ordering::Relaxed);
        Self(Some(vm))
    }
}

impl Deref for OwnedVm {
    type Target = VirtualMachine;

    fn deref(&self) -> &Self::Target {
        self.0.as_deref().expect("VM owner already released")
    }
}

impl DerefMut for OwnedVm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_deref_mut().expect("VM owner already released")
    }
}

impl Drop for OwnedVm {
    fn drop(&mut self) {
        if let Some(vm) = self.0.take() {
            thread::destroy_vm(vm);
        }
    }
}

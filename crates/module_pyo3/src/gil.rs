//! Holding both interpreters at once.
//!
//! RustPython and CPython each have a lock that a thread must hold to run
//! Python code: RustPython's attached thread state (which a stop-the-world
//! request waits on) and the GIL. A thread that blocks on one while
//! holding the other can deadlock against a thread doing the opposite, so:
//!
//! - both are held only to move objects across ([`with_gil`],
//!   [`with_rustpython`]), and the GIL is always waited for with the
//!   RustPython thread state detached;
//! - CPython code runs detached from RustPython ([`run_cpython`]);
//! - RustPython code called from CPython runs with the GIL released
//!   ([`run_rustpython`]).
//!
//! Each side attaches again whenever the code running reaches the other.

use core::mem::ManuallyDrop;
use core::ops::Deref;

use pyo3::Python;
use rustpython_vm::{PyResult, VirtualMachine};

use crate::cpython_init::ensure_cpython_initialized;
use crate::errors::Pyo3ResultExt;

/// Run `f` holding the CPython GIL, with the RustPython thread attached.
///
/// The thread detaches from RustPython while it waits for the GIL and attaches
/// again once it has it, so `f` may freely touch objects of both runtimes.
pub(crate) fn with_gil<R>(vm: &VirtualMachine, f: impl for<'py> FnOnce(Python<'py>) -> R) -> R {
    vm.allow_threads(|| {
        ensure_cpython_initialized();
        Python::attach(|py| {
            // Copies queued by a result's destructor outlive that destructor.
            let _flush = FlushDeferredDrops;
            vm.attach_for_callback(|| f(py))
        })
    })
}

/// Drops call copies after the closure that released their result returns.
struct FlushDeferredDrops;

impl Drop for FlushDeferredDrops {
    fn drop(&mut self) {
        crate::convert::flush_deferred_drops();
    }
}

/// Run `f` on behalf of CPython code calling back into RustPython.
///
/// Fails with `RuntimeError` when the calling thread has no RustPython
/// interpreter, e.g. a thread started by a CPython extension.
pub(crate) fn with_rustpython<R>(
    py: Python<'_>,
    f: impl FnOnce(&VirtualMachine) -> PyResult<R>,
) -> pyo3::PyResult<R> {
    rustpython_vm::vm::thread::try_with_current_vm(|vm| {
        vm.attach_for_callback(|| {
            f(vm).map_err(|exc| crate::errors::rustpython_exc_to_cpython(py, exc, vm))
        })
    })
    .unwrap_or_else(|| {
        Err(pyo3::exceptions::PyRuntimeError::new_err(
            "no RustPython interpreter is running on this thread",
        ))
    })
}

/// Run CPython code, which may run for long or wait on RustPython threads,
/// with the thread detached from RustPython.
pub(crate) fn run_cpython<T>(
    vm: &VirtualMachine,
    f: impl FnOnce() -> pyo3::PyResult<T>,
) -> PyResult<T> {
    vm.allow_threads(f).to_pyresult(vm)
}

/// Run RustPython code called from CPython with the GIL released, so CPython
/// threads keep running meanwhile.
///
/// `f` must reach CPython objects only through [`with_gil`], which takes the
/// GIL again.
pub(crate) fn run_rustpython<R>(py: Python<'_>, f: impl FnOnce() -> R) -> R {
    // `Python::detach` requires `Send` to keep CPython objects out of `f`,
    // which RustPython objects are not without threading.
    struct AssertUngil<T>(T);
    // SAFETY: `f` runs, and its result is used, on this thread; it does not
    // touch CPython objects without the GIL, as documented above.
    unsafe impl<T> Send for AssertUngil<T> {}

    let f = AssertUngil(f);
    py.detach(move || {
        let f = f;
        AssertUngil((f.0)())
    })
    .0
}

/// A RustPython value owned by a CPython object, released with the thread
/// attached to RustPython: CPython frees objects wherever it likes, and
/// releasing a RustPython object may run its finalizer.
pub(crate) struct DropAttached<T>(ManuallyDrop<T>);

impl<T> DropAttached<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(ManuallyDrop::new(value))
    }
}

impl<T> Deref for DropAttached<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Drop for DropAttached<T> {
    fn drop(&mut self) {
        // SAFETY: the value is taken once, here, and not used afterwards.
        let value = unsafe { ManuallyDrop::take(&mut self.0) };
        // With no interpreter on this thread, `value` drops with the closure.
        rustpython_vm::vm::thread::try_with_current_vm(move |vm| {
            vm.attach_for_callback(|| drop(value));
        });
    }
}

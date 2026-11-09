#![cfg(test)]
#![cfg(all(feature = "compiler", feature = "gc"))]

extern crate alloc;

// Keep collection isolated from other interpreter fixtures in this process.
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use rustpython_vm::{
    AsObject, Interpreter, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyBaseException, PyType, PyValueError},
    class::{PyClassImpl, PySubclass},
    function::FuncArgs,
    object::{Traverse, TraverseFn},
    pyclass,
    types::Constructor,
};

#[pyclass(module = "exception_payload_test", name = "DropError", base = PyValueError, traverse = "manual")]
#[repr(C)]
#[derive(Debug)]
struct DropError {
    base: PyValueError,
    dropped: Arc<AtomicBool>,
}

// SAFETY: repr(C) keeps the valid base first for the payload's entire lifetime;
// pyclass checks the base offset and compatible payload alignment.
unsafe impl PySubclass for DropError {
    type Base = PyValueError;

    /// Exposes the embedded base without changing the native payload's lifetime.
    fn as_base(&self) -> &Self::Base {
        &self.base
    }
}

// SAFETY: The base owns every Python reference; the flag owns none.
unsafe impl Traverse for DropError {
    /// Visits the base's Python references; the destruction flag holds no Python objects.
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.base.traverse(tracer_fn);
    }

    /// Delegates collection to the production wrapper's clearing implementation.
    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        self.base.clear(out);
    }
}

#[pyclass(with(Constructor))]
impl DropError {}

impl Constructor for DropError {
    type Args = FuncArgs;

    /// Constructs the owned base and initializes its native destruction witness.
    fn py_new(cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self {
            base: PyValueError::py_new(cls, args, vm)?,
            dropped: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Drop for DropError {
    /// Records actual native destruction independently of Python weakref invalidation.
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// Proves a context-only cycle releases the native exception payload itself.
#[test]
fn context_cycle_destroys_owned_exception_payload() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = DropError::make_static_type();
        let payload = DropError::py_new(&class, FuncArgs::default(), vm).unwrap();
        let dropped = payload.dropped.clone();
        let exception = payload.into_ref_with_type(vm, class).unwrap();
        let base = exception.as_base().as_base().as_base();
        base.set_context(Some(exception.clone().upcast::<PyBaseException>()));
        assert!(base.__context__().unwrap().is(exception.as_object()));
        assert!(!dropped.load(Ordering::SeqCst));
        drop(exception);
        assert!(!dropped.load(Ordering::SeqCst));
        vm.state.gc.collect_force(2);
        assert!(dropped.load(Ordering::SeqCst));
    });
}

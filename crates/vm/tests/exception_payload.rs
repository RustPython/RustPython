#![cfg(test)]
#![cfg(all(feature = "compiler", feature = "gc"))]

extern crate alloc;

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

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[pyclass(module = "exception_payload_test", name = "DropError", base = PyValueError, traverse = "manual")]
#[repr(C)]
#[derive(Debug)]
struct DropError {
    base: PyValueError,
    dropped: Arc<AtomicBool>,
}

// SAFETY: repr(C) keeps the valid base first; pyclass checks offset and alignment.
unsafe impl PySubclass for DropError {
    type Base = PyValueError;

    fn as_base(&self) -> &Self::Base {
        &self.base
    }
}

// SAFETY: The base owns every Python reference; the flag owns none.
unsafe impl Traverse for DropError {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.base.traverse(tracer_fn);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        self.base.clear(out);
    }
}

#[pyclass(with(Constructor))]
impl DropError {}

impl Constructor for DropError {
    type Args = FuncArgs;

    fn py_new(cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self {
            base: PyValueError::py_new(cls, args, vm)?,
            dropped: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Drop for DropError {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn owned_native_exception_round_trips_through_python() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = DropError::make_static_type();
        let argument = vm.ctx.new_str("payload");
        let payload = DropError::py_new(&class, vec![argument.clone().into()].into(), vm).unwrap();
        let dropped = payload.dropped.clone();
        let exception = payload.into_ref_with_type(vm, class).unwrap();
        let base = exception.as_base().as_base().as_base();
        assert!(base.__traceback__().is_none());
        assert!(base.__cause__().is_none());
        assert!(base.__context__().is_none());
        assert!(
            !exception
                .as_object()
                .get_attr("__suppress_context__", vm)
                .unwrap()
                .try_to_bool(vm)
                .unwrap()
        );
        assert!(!dropped.load(Ordering::SeqCst));

        let scope = vm.new_scope_with_builtins();
        scope.globals.set_item("exc", exception.into(), vm).unwrap();
        scope.globals.set_item("arg", argument.into(), vm).unwrap();
        let source = r#"
try:
    raise exc
except ValueError as e:
    assert e.args == (arg,)
    assert isinstance(e, ValueError)
    caught = e
"#;
        let code = vm
            .compile(
                source,
                rustpython_vm::compiler::Mode::Exec,
                "<native-exception>",
            )
            .map_err(|error| error.into_pyexception(vm, Some(source)))
            .unwrap();
        if let Err(error) = vm.run_code_obj(code, scope.clone()) {
            vm.print_exception(&error);
            panic!("owned native exception round-trip failed");
        }
        let caught = scope.globals.get_item("caught", vm).unwrap();
        let native = caught.downcast::<DropError>().unwrap();
        assert!(Arc::ptr_eq(&native.dropped, &dropped));
        assert!(!native.dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn context_cycle_destroys_owned_exception_payload() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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

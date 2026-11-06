#![cfg(test)]

use rustpython_vm::{
    AsObject, Interpreter, PyPayload,
    builtins::{PyBaseException, PyException, PyValueError},
    class::PySubclass,
    function::FuncArgs,
    object::Traverse,
    types::Constructor,
};

#[test]
fn owned_base_exception_preserves_arguments_and_starts_without_exception_state() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = vm.ctx.exceptions.base_exception_type;
        let empty = PyBaseException::py_new(class, FuncArgs::default(), vm).unwrap();
        assert!(empty.args().is_empty());

        let first = vm.ctx.new_str("first");
        let second = vm.ctx.new_int(17);
        let arguments = vec![first.clone().into(), second.clone().into()];
        let payload = PyBaseException::py_new(class, arguments.into(), vm).unwrap();
        let arguments = payload.args();
        assert_eq!(arguments.len(), 2);
        assert!(arguments.as_slice()[0].is(first.as_object()));
        assert!(arguments.as_slice()[1].is(second.as_object()));
        assert!(payload.__traceback__().is_none());
        assert!(payload.__cause__().is_none());
        assert!(payload.__context__().is_none());
    });
}

#[test]
fn owned_simple_exception_payloads_delegate_through_their_base() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let argument = vm.ctx.new_str("message");
        let exception = PyException::py_new(
            vm.ctx.exceptions.exception_type,
            vec![argument.clone().into()].into(),
            vm,
        )
        .unwrap();
        assert!(exception.as_base().args().as_slice()[0].is(argument.as_object()));

        let value_error = PyValueError::py_new(
            vm.ctx.exceptions.value_error,
            vec![argument.clone().into()].into(),
            vm,
        )
        .unwrap();
        assert!(value_error.as_base().as_base().args().as_slice()[0].is(argument.as_object()));
    });
}

#[test]
fn allocated_owned_wrapper_traverses_its_argument_tuple() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let argument = vm.ctx.new_str("argument retained by the base");
        let exception = PyValueError::py_new(
            vm.ctx.exceptions.value_error,
            vec![argument.into()].into(),
            vm,
        )
        .unwrap()
        .into_ref(&vm.ctx);
        let arguments = exception.as_base().as_base().args();
        let mut visited_arguments = false;
        exception.as_object().traverse(&mut |object| {
            visited_arguments |= object.is(arguments.as_object());
        });
        assert!(visited_arguments);
    });
}

#[cfg(feature = "compiler")]
mod native_subclass {
    use super::*;
    use rustpython_vm::{
        Py, PyResult, VirtualMachine, builtins::PyType, class::PyClassImpl, object::TraverseFn,
        pyclass,
    };

    #[pyclass(module = "exception_payload_test", name = "NativeError", base = PyException, traverse = "manual")]
    #[repr(C)]
    #[derive(Debug)]
    struct NativeError {
        base: PyException,
        marker: usize,
    }

    // SAFETY: repr(C) keeps the valid base as the first field for Self's entire
    // lifetime. pyclass checks base/payload offsets and compatible alignment.
    unsafe impl PySubclass for NativeError {
        type Base = PyException;

        fn as_base(&self) -> &Self::Base {
            &self.base
        }
    }

    // SAFETY: The base owns all Python references; marker contains none.
    unsafe impl Traverse for NativeError {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.base.as_base().traverse(tracer_fn);
        }
    }

    #[pyclass(flags(BASETYPE, HAS_DICT), with(Constructor))]
    impl NativeError {
        #[pymethod]
        fn marker(zelf: &Py<Self>) -> usize {
            zelf.marker
        }
    }

    impl Constructor for NativeError {
        type Args = FuncArgs;

        fn py_new(cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                base: PyException::py_new(cls, args, vm)?,
                marker: 42,
            })
        }
    }

    #[cfg(feature = "gc")]
    #[test]
    fn owned_wrapper_cycles_release_argument_objects() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let source = "class Marker: pass\nmarker = Marker()\n";
            let code = vm
                .compile(source, rustpython_vm::compiler::Mode::Exec, "<cycle>")
                .map_err(|error| error.into_pyexception(vm, Some(source)))
                .unwrap();
            vm.run_code_obj(code, scope.clone()).unwrap();
            let argument = scope.globals.get_item("marker", vm).unwrap();
            let weak = argument.downgrade(None, vm).unwrap();
            let exception =
                PyValueError::py_new(vm.ctx.exceptions.value_error, vec![argument].into(), vm)
                    .unwrap()
                    .into_ref(&vm.ctx);
            exception
                .as_base()
                .as_base()
                .set_cause(Some(exception.clone().upcast::<PyBaseException>()));
            scope.globals.del_item("marker", vm).unwrap();
            drop(exception);
            vm.state.gc.collect_force(2);
            assert!(weak.upgrade().is_none());
        });
    }

    #[test]
    fn external_native_exception_retains_payload_and_python_constructor_dispatch() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let class = NativeError::make_static_type();
            let base = PyException::py_new(
                &class,
                vec![vm.ctx.new_str("host message").into()].into(),
                vm,
            )
            .unwrap();
            let native = NativeError { base, marker: 17 }
                .into_ref_with_type(vm, class.clone())
                .unwrap();
            let scope = vm.new_scope_with_builtins();
            scope
                .globals
                .set_item("NativeError", class.into(), vm)
                .unwrap();
            scope
                .globals
                .set_item("native", native.clone().into(), vm)
                .unwrap();
            let source = r#"
assert isinstance(native, Exception)
assert native.args == ("host message",)
assert native.__cause__ is None
assert native.__context__ is None
assert native.__traceback__ is None
try:
    raise native
except NativeError as caught:
    assert caught is native
try:
    try:
        raise ValueError("inner")
    except ValueError as inner:
        raise native from inner
except Exception as caught:
    assert caught is native
    assert isinstance(caught.__cause__, ValueError)
    assert caught.__traceback__ is not None
assert native.marker() == 17
native.marker = lambda: 99

events = []
class Child(NativeError):
    def __new__(cls, *args):
        events.append("new")
        return super().__new__(cls, *args)
    def __init__(self, *args):
        events.append("init")
        super().__init__(*args)

child = Child("child message")
assert events == ["new", "init"]
assert child.args == ("child message",)
assert isinstance(child, Exception)
assert child.marker() == 42
assert NativeError("plain").args == ("plain",)
"#;
            let code = vm
                .compile(
                    source,
                    rustpython_vm::compiler::Mode::Exec,
                    "<native-exception>",
                )
                .map_err(|error| error.into_pyexception(vm, Some(source)))
                .unwrap();
            if let Err(error) = vm.run_code_obj(code, scope) {
                vm.print_exception(&error);
                panic!("native exception assertions failed");
            }
            assert_eq!(native.marker, 17);
        });
    }
}

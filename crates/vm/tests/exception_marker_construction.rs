#![cfg(test)]
#![cfg(feature = "compiler")]

// Rust guideline compliant 2026-10-09
use core::marker::PhantomData;
use rustpython_vm::{
    AsObject, Interpreter, Py, PyPayload, PyResult, VirtualMachine,
    builtins::{PyException, PyType},
    class::{PyClassImpl, PySubclass},
    function::FuncArgs,
    pyclass, pyexception,
    types::Constructor,
};

#[pyexception(name = "PhantomError", base = PyException)]
#[derive(Debug)]
#[repr(transparent)]
struct PhantomError(PyException, PhantomData<()>);

#[pyexception]
impl PhantomError {}

// Deliberately inhabited but not Default: transparent marker fields must not
// acquire construction bounds merely from expanding #[pyexception].
#[derive(Debug)]
struct Marker;

#[pyexception(name = "MarkerError", base = PyException)]
#[derive(Debug)]
#[repr(transparent)]
struct MarkerError(PyException, Marker);

#[pyexception]
impl MarkerError {}

#[pyexception(name = "CustomError", base = PyException)]
#[derive(Debug)]
#[repr(transparent)]
struct CustomError(PyException, Marker);

#[pyexception(with(Constructor))]
impl CustomError {}

impl Constructor for CustomError {
    type Args = FuncArgs;

    fn py_new(cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self(PyException::py_new(cls, args, vm)?, Marker))
    }
}

#[pyexception(name = "SimpleError", base = PyException, impl)]
#[derive(Debug)]
#[repr(transparent)]
struct SimpleError(PyException);

fn check_python_dispatch(class: rustpython_vm::builtins::PyTypeRef, vm: &VirtualMachine) {
    let scope = vm.new_scope_with_builtins();
    scope.globals.set_item("Error", class.into(), vm).unwrap();
    let source = r#"
plain = Error("plain")
assert isinstance(plain, Error)
assert isinstance(plain, Exception)
assert plain.args == ("plain",)
events = []
class Child(Error):
    def __new__(cls, *args):
        events.append("new")
        return super().__new__(cls, *args)
    def __init__(self, *args):
        events.append("init")
        super().__init__(*args)
child = Child("child")
assert events == ["new", "init"]
assert isinstance(child, Child)
assert isinstance(child, Error)
assert child.args == ("child",)
try:
    raise child
except Error as caught:
    assert caught is child
"#;
    let code = vm
        .compile(
            source,
            rustpython_vm::compiler::Mode::Exec,
            "<marker-wrapper>",
        )
        .map_err(|error| error.into_pyexception(vm, Some(source)))
        .unwrap();
    if let Err(error) = vm.run_code_obj(code, scope) {
        vm.print_exception(&error);
        panic!("marker wrapper Python dispatch failed");
    }
}

#[test]
fn phantom_wrapper_preserves_python_allocation_and_dispatch() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        check_python_dispatch(PhantomError::make_static_type(), vm);
    });
}

#[test]
fn non_default_marker_preserves_python_allocation_and_dispatch() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        check_python_dispatch(MarkerError::make_static_type(), vm);
    });
}

#[test]
fn custom_constructor_initializes_owned_marker_payload() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = CustomError::make_static_type();
        let argument = vm.ctx.new_str("custom");
        let payload =
            CustomError::py_new(&class, vec![argument.clone().into()].into(), vm).unwrap();
        let Marker = payload.1;
        assert!(payload.as_base().as_base().args().as_slice()[0].is(argument.as_object()));
        let owned = payload.into_ref_with_type(vm, class.clone()).unwrap();
        assert!(owned.class().is(&class));
        check_python_dispatch(class, vm);
    });
}

#[test]
fn simple_wrapper_retains_automatic_owned_construction() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = SimpleError::make_static_type();
        let argument = vm.ctx.new_str("simple");
        let owned = SimpleError::py_new(&class, vec![argument.clone().into()].into(), vm)
            .unwrap()
            .into_ref_with_type(vm, class)
            .unwrap();
        assert!(owned.as_base().as_base().args().as_slice()[0].is(argument.as_object()));
    });
}

pub mod split_payload {
    use rustpython_vm::{builtins::PyException, pyclass, pyexception};

    #[pyexception(name = "SplitError", base = PyException)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct SplitError(pub PyException);
}

use split_payload::SplitError;

#[pyexception]
impl SplitError {}

#[test]
fn split_module_wrapper_preserves_owned_construction_and_python_dispatch() {
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let class = SplitError::make_static_type();
        let argument = vm.ctx.new_str("split");
        let owned = SplitError::py_new(&class, vec![argument.clone().into()].into(), vm)
            .unwrap()
            .into_ref_with_type(vm, class.clone())
            .unwrap();
        assert!(owned.class().is(&class));
        assert!(owned.as_base().as_base().args().as_slice()[0].is(argument.as_object()));
        check_python_dispatch(class, vm);
    });
}

use crate::pyobject::{
    IntoPyObject, PyContext, PyFuncArgs, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyNone;
pub type PyNoneRef = PyRef<PyNone>;

impl PyValue for PyNone {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.none().typ()
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl IntoPyObject for () {
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult {
        Ok(vm.ctx.none())
    }
}

impl<T: IntoPyObject> IntoPyObject for Option<T> {
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult {
        match self {
            Some(x) => x.into_pyobject(vm),
            None => Ok(vm.ctx.none()),
        }
    }
}

impl PyNoneRef {
    fn repr(self, _vm: &mut VirtualMachine) -> PyResult<String> {
        Ok("None".to_string())
    }

    fn bool(self, _vm: &mut VirtualMachine) -> PyResult<bool> {
        Ok(false)
    }
}

fn none_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.type_type.clone()))]
    );
    Ok(vm.get_none())
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.none.typ(), {
        "__new__" => context.new_rustfunc(none_new),
        crate::VM_REPR => context.new_rustfunc(PyNoneRef::repr),
        "__bool__" => context.new_rustfunc(PyNoneRef::bool),
    });
}

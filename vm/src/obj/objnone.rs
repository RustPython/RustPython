use super::objtype::PyClassRef;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[pyclass(name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;
pub type PyNoneRef = PyRef<PyNone>;

impl PyValue for PyNone {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.none().class()
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl IntoPyObject for () {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

impl<T: IntoPyObject> IntoPyObject for Option<T> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Some(x) => x.into_pyobject(vm),
            None => vm.ctx.none(),
        }
    }
}

#[pyimpl]
impl PyNone {
    #[pyslot]
    fn tp_new(_: PyClassRef, vm: &VirtualMachine) -> PyNoneRef {
        vm.ctx.none.clone()
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> PyResult<String> {
        Ok("None".to_owned())
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> PyResult<bool> {
        Ok(false)
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if vm.is_none(&rhs) {
            vm.ctx.new_bool(true)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if vm.is_none(&rhs) {
            vm.ctx.new_bool(false)
        } else {
            vm.ctx.not_implemented()
        }
    }
}

pub fn init(context: &PyContext) {
    PyNone::extend_class(context, &context.none.class());
}

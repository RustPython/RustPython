use crate::function::PyFuncArgs;
use crate::pyobject::{PyResult, PyValue};
use crate::VirtualMachine;

#[pyimpl]
pub trait PyBuiltinCallable: PyValue {
    #[pymethod(magic)]
    #[pyslot]
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult;
}

use std::fmt;

use crate::function::PyNativeFunc;
use crate::pyobject::{PyObjectRef, PyValue};
use crate::vm::VirtualMachine;

pub struct PyBuiltinFunction {
    // TODO: shouldn't be public
    pub value: PyNativeFunc,
}

impl PyValue for PyBuiltinFunction {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.builtin_function_or_method_type()]
    }
}

impl fmt::Debug for PyBuiltinFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "builtin function")
    }
}

impl PyBuiltinFunction {
    pub fn new(value: PyNativeFunc) -> Self {
        Self { value }
    }
}

use std::fmt;

use crate::function::PyNativeFunc;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::PyValue;
use crate::vm::VirtualMachine;

pub struct PyBuiltinFunction {
    value: PyNativeFunc,
}

impl PyValue for PyBuiltinFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.builtin_function_or_method_type()
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

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.value
    }
}

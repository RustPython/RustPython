use std::fmt;

use crate::function::{PyFuncArgs, PyNativeFunc};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[pyclass]
pub struct PyBuiltinFunction {
    value: PyNativeFunc,
    bindable: bool,
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
    pub fn new(value: PyNativeFunc, bindable: bool) -> Self {
        Self { value, bindable }
    }

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.value
    }
}

#[pyimpl]
impl PyBuiltinFunction {
    #[pymethod(name = "__call__")]
    pub fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        (self.value)(vm, args)
    }

    #[pymethod(name = "__get__")]
    fn bind_method(
        function: PyRef<Self>,
        obj: PyObjectRef,
        cls: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        if !function.bindable || obj.is(&vm.get_none()) && !cls.is(&obj.class()) {
            Ok(function.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(function.into_object(), obj))
        }
    }
}

pub fn init(context: &PyContext) {
    PyBuiltinFunction::extend_class(context, &context.types.builtin_function_or_method_type);
}

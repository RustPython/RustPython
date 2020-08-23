use std::fmt;

use crate::function::{OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::{SlotCall, SlotDescriptor};
use crate::vm::VirtualMachine;

#[pyclass(name = "builtin_function_or_method", module = false)]
pub struct PyBuiltinFunction {
    value: PyNativeFunc,
    module: Option<PyStringRef>,
    name: Option<PyStringRef>,
}

impl PyValue for PyBuiltinFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.builtin_function_or_method_type.clone()
    }
}

impl fmt::Debug for PyBuiltinFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "builtin function")
    }
}

impl From<PyNativeFunc> for PyBuiltinFunction {
    fn from(value: PyNativeFunc) -> Self {
        Self {
            value,
            module: None,
            name: None,
        }
    }
}

impl PyBuiltinFunction {
    pub fn new_with_name(value: PyNativeFunc, module: PyStringRef, name: PyStringRef) -> Self {
        Self {
            value,
            module: Some(module),
            name: Some(name),
        }
    }

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.value
    }
}

impl SlotCall for PyBuiltinFunction {
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        (self.value)(vm, args)
    }
}

#[pyimpl(with(SlotCall), flags(HAS_DICT))]
impl PyBuiltinFunction {
    #[pyproperty(magic)]
    fn module(&self) -> Option<PyStringRef> {
        self.module.clone()
    }
    #[pyproperty(magic)]
    fn name(&self) -> Option<PyStringRef> {
        self.name.clone()
    }
}

#[pyclass(module = false, name = "method_descriptor")]
pub struct PyBuiltinMethod {
    function: PyBuiltinFunction,
}

impl PyValue for PyBuiltinMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.method_descriptor_type.clone()
    }
}

impl fmt::Debug for PyBuiltinMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "method descriptor")
    }
}

impl From<PyNativeFunc> for PyBuiltinMethod {
    fn from(value: PyNativeFunc) -> Self {
        Self {
            function: value.into(),
        }
    }
}

impl PyBuiltinMethod {
    pub fn new_with_name(value: PyNativeFunc, module: PyStringRef, name: PyStringRef) -> Self {
        Self {
            function: PyBuiltinFunction::new_with_name(value, module, name),
        }
    }

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.function.value
    }
}

impl SlotDescriptor for PyBuiltinMethod {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: OptionalArg<PyObjectRef>,
    ) -> PyResult {
        let (zelf, obj) = match Self::_check(zelf, obj, vm) {
            Ok(obj) => obj,
            Err(result) => return result,
        };
        if obj.is(&vm.get_none()) && !Self::_cls_is(&cls, &obj.class()) {
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(zelf.into_object(), obj))
        }
    }
}

impl SlotCall for PyBuiltinMethod {
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        (self.function.value)(vm, args)
    }
}

#[pyimpl(with(SlotDescriptor, SlotCall))]
impl PyBuiltinMethod {
    #[pyproperty(magic)]
    fn module(&self) -> Option<PyStringRef> {
        self.function.module.clone()
    }
    #[pyproperty(magic)]
    fn name(&self) -> Option<PyStringRef> {
        self.function.name.clone()
    }
}

pub fn init(context: &PyContext) {
    PyBuiltinFunction::extend_class(context, &context.types.builtin_function_or_method_type);
    PyBuiltinMethod::extend_class(context, &context.types.method_descriptor_type);
}

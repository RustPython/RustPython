use std::fmt;

use super::objclassmethod::PyClassMethod;
use crate::function::{OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    PyClassImpl, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::{SlotCall, SlotDescriptor};
use crate::vm::VirtualMachine;

pub struct PyFuncDef {
    pub func: PyNativeFunc,
    pub name: Option<PyStringRef>,
    pub doc: Option<PyStringRef>,
}

impl From<PyNativeFunc> for PyFuncDef {
    fn from(func: PyNativeFunc) -> Self {
        Self {
            func,
            name: None,
            doc: None,
        }
    }
}

impl PyFuncDef {
    pub fn with_doc(mut self, doc: String, ctx: &PyContext) -> Self {
        self.doc = Some(ctx.new_stringref(doc));
        self
    }

    pub fn into_function(self) -> PyBuiltinFunction {
        self.into()
    }
    pub fn build_function(self, ctx: &PyContext) -> PyObjectRef {
        self.into_function().build(ctx)
    }
    pub fn build_method(self, ctx: &PyContext) -> PyObjectRef {
        PyObject::new(
            PyBuiltinMethod::from(self),
            ctx.types.method_descriptor_type.clone(),
            None,
        )
    }
    pub fn build_classmethod(self, ctx: &PyContext) -> PyObjectRef {
        // TODO: classmethod_descriptor
        PyObject::new(
            PyClassMethod::from(self.build_method(ctx)),
            ctx.types.classmethod_type.clone(),
            None,
        )
    }
}

#[pyclass(name = "builtin_function_or_method", module = false)]
pub struct PyBuiltinFunction {
    value: PyFuncDef,
    module: Option<PyObjectRef>,
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
        PyFuncDef::from(value).into()
    }
}
impl From<PyFuncDef> for PyBuiltinFunction {
    fn from(value: PyFuncDef) -> Self {
        Self {
            value,
            module: None,
        }
    }
}

impl PyBuiltinFunction {
    pub fn with_module(mut self, module: PyObjectRef) -> Self {
        self.module = Some(module);
        self
    }

    pub fn build(self, ctx: &PyContext) -> PyObjectRef {
        PyObject::new(
            self,
            ctx.types.builtin_function_or_method_type.clone(),
            None,
        )
    }

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.value.func
    }
}

impl SlotCall for PyBuiltinFunction {
    fn call(zelf: PyRef<Self>, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.value.func)(vm, args)
    }
}

#[pyimpl(with(SlotCall), flags(HAS_DICT))]
impl PyBuiltinFunction {
    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.unwrap_or_none(self.module.clone())
    }
    #[pyproperty(magic)]
    fn name(&self) -> Option<PyStringRef> {
        self.value.name.clone()
    }
    #[pyproperty(magic)]
    fn doc(&self) -> Option<PyStringRef> {
        self.value.doc.clone()
    }
}

#[pyclass(module = false, name = "method_descriptor")]
pub struct PyBuiltinMethod {
    value: PyFuncDef,
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

impl From<PyFuncDef> for PyBuiltinMethod {
    fn from(value: PyFuncDef) -> Self {
        Self { value }
    }
}

impl PyBuiltinMethod {
    pub fn new_with_name(func: PyNativeFunc, name: PyStringRef) -> Self {
        Self {
            value: PyFuncDef {
                func,
                name: Some(name),
                doc: None,
            },
        }
    }

    pub fn as_func(&self) -> &PyNativeFunc {
        &self.value.func
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
        if vm.is_none(&obj) && !Self::_cls_is(&cls, &obj.class()) {
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(zelf.into_object(), obj))
        }
    }
}

impl SlotCall for PyBuiltinMethod {
    fn call(zelf: PyRef<Self>, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.value.func)(vm, args)
    }
}

#[pyimpl(with(SlotDescriptor, SlotCall))]
impl PyBuiltinMethod {
    #[pyproperty(magic)]
    fn name(&self) -> Option<PyStringRef> {
        self.value.name.clone()
    }
    #[pyproperty(magic)]
    fn doc(&self) -> Option<PyStringRef> {
        self.value.doc.clone()
    }
}

pub fn init(context: &PyContext) {
    PyBuiltinFunction::extend_class(context, &context.types.builtin_function_or_method_type);
    PyBuiltinMethod::extend_class(context, &context.types.method_descriptor_type);
}

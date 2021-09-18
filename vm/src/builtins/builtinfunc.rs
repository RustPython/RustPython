use std::fmt;

use super::classmethod::PyClassMethod;
use super::pytype;
use crate::builtins::pystr::PyStrRef;
use crate::builtins::pytype::PyTypeRef;
use crate::function::{FuncArgs, PyNativeFunc};
use crate::slots::{Callable, SlotDescriptor};
use crate::vm::VirtualMachine;
use crate::{
    PyClassImpl, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};

pub struct PyNativeFuncDef {
    pub func: PyNativeFunc,
    pub name: PyStrRef,
    pub doc: Option<PyStrRef>,
}

impl PyNativeFuncDef {
    pub fn new(func: PyNativeFunc, name: PyStrRef) -> Self {
        Self {
            func,
            name,
            doc: None,
        }
    }
}

impl PyNativeFuncDef {
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
    pub fn build_method(self, ctx: &PyContext, class: PyTypeRef) -> PyObjectRef {
        PyObject::new(
            PyBuiltinMethod { value: self, class },
            ctx.types.method_descriptor_type.clone(),
            None,
        )
    }
    pub fn build_classmethod(self, ctx: &PyContext, class: PyTypeRef) -> PyObjectRef {
        // TODO: classmethod_descriptor
        PyObject::new(
            PyClassMethod::from(self.build_method(ctx, class)),
            ctx.types.classmethod_type.clone(),
            None,
        )
    }
}

#[pyclass(name = "builtin_function_or_method", module = false)]
pub struct PyBuiltinFunction {
    value: PyNativeFuncDef,
    module: Option<PyObjectRef>,
}

impl PyValue for PyBuiltinFunction {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.builtin_function_or_method_type
    }
}

impl fmt::Debug for PyBuiltinFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "builtin function {}", self.value.name.as_str())
    }
}

impl From<PyNativeFuncDef> for PyBuiltinFunction {
    fn from(value: PyNativeFuncDef) -> Self {
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

impl Callable for PyBuiltinFunction {
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.value.func)(vm, args)
    }
}

#[pyimpl(with(Callable), flags(HAS_DICT))]
impl PyBuiltinFunction {
    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.unwrap_or_none(self.module.clone())
    }
    #[pyproperty(magic)]
    fn name(&self) -> PyStrRef {
        self.value.name.clone()
    }
    #[pyproperty(magic)]
    fn qualname(&self) -> PyStrRef {
        self.name()
    }
    #[pyproperty(magic)]
    fn doc(&self) -> Option<PyStrRef> {
        self.value.doc.clone()
    }
    #[pyproperty(name = "__self__")]
    fn get_self(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
    #[pymethod(magic)]
    fn reduce(&self) -> PyStrRef {
        // TODO: return (getattr, (self.object, self.name)) if this is a method
        self.name()
    }
    #[pymethod(magic)]
    fn reduce_ex(&self, _ver: PyObjectRef) -> PyStrRef {
        self.name()
    }
    #[pymethod(magic)]
    fn repr(&self) -> String {
        format!("<built-in function {}>", self.value.name)
    }
    #[pyproperty(magic)]
    fn text_signature(&self) -> Option<String> {
        self.value.doc.as_ref().and_then(|doc| {
            pytype::get_text_signature_from_internal_doc(self.value.name.as_str(), doc.as_str())
                .map(|signature| signature.to_string())
        })
    }
}

// `PyBuiltinMethod` is similar to both `PyMethodDescrObject` in
// https://github.com/python/cpython/blob/main/Include/descrobject.h
// https://github.com/python/cpython/blob/main/Objects/descrobject.c
// and `PyCMethodObject` in
// https://github.com/python/cpython/blob/main/Include/cpython/methodobject.h
// https://github.com/python/cpython/blob/main/Objects/methodobject.c
#[pyclass(module = false, name = "method_descriptor")]
pub struct PyBuiltinMethod {
    value: PyNativeFuncDef,
    class: PyTypeRef,
}

impl PyValue for PyBuiltinMethod {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.method_descriptor_type
    }
}

impl fmt::Debug for PyBuiltinMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "method descriptor for '{}'", self.value.name)
    }
}

impl SlotDescriptor for PyBuiltinMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
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

impl Callable for PyBuiltinMethod {
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.value.func)(vm, args)
    }
}

#[pyimpl(with(SlotDescriptor, Callable), flags(METHOD_DESCR))]
impl PyBuiltinMethod {
    #[pyproperty(magic)]
    fn name(&self) -> PyStrRef {
        self.value.name.clone()
    }
    #[pyproperty(magic)]
    fn qualname(&self) -> String {
        format!("{}.{}", self.class.name(), &self.value.name)
    }
    #[pyproperty(magic)]
    fn doc(&self) -> Option<PyStrRef> {
        self.value.doc.clone()
    }
    #[pyproperty(magic)]
    fn text_signature(&self) -> Option<String> {
        self.value.doc.as_ref().and_then(|doc| {
            pytype::get_text_signature_from_internal_doc(self.value.name.as_str(), doc.as_str())
                .map(|signature| signature.to_string())
        })
    }
    #[pymethod(magic)]
    fn repr(&self) -> String {
        format!(
            "<method '{}' of '{}' objects>",
            &self.value.name,
            self.class.name()
        )
    }
}

pub fn init(context: &PyContext) {
    PyBuiltinFunction::extend_class(context, &context.types.builtin_function_or_method_type);
    PyBuiltinMethod::extend_class(context, &context.types.method_descriptor_type);
}

use super::{PyStrInterned, PyStrRef, PyType, type_};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::wtf8::Wtf8,
    convert::TryFromObject,
    function::{FuncArgs, PyComparisonValue, PyMethodDef, PyMethodFlags, PyNativeFn},
    types::{Callable, Comparable, PyComparisonOp, Representable, Unconstructible},
};
use std::fmt;

// PyCFunctionObject in CPython
#[pyclass(name = "builtin_function_or_method", module = false)]
pub struct PyNativeFunction {
    pub(crate) value: &'static PyMethodDef,
    pub(crate) zelf: Option<PyObjectRef>,
    pub(crate) module: Option<&'static PyStrInterned>, // None for bound method
}

impl PyPayload for PyNativeFunction {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.builtin_function_or_method_type
    }
}

impl fmt::Debug for PyNativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "builtin function {}.{} ({:?}) self as instance of {:?}",
            self.module.map_or(Wtf8::new("<unknown>"), |m| m.as_wtf8()),
            self.value.name,
            self.value.flags,
            self.zelf.as_ref().map(|z| z.class().name().to_owned())
        )
    }
}

impl PyNativeFunction {
    pub const fn with_module(mut self, module: &'static PyStrInterned) -> Self {
        self.module = Some(module);
        self
    }

    pub fn into_ref(self, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(
            self,
            ctx.types.builtin_function_or_method_type.to_owned(),
            None,
        )
    }

    // PyCFunction_GET_SELF
    pub const fn get_self(&self) -> Option<&PyObjectRef> {
        if self.value.flags.contains(PyMethodFlags::STATIC) {
            return None;
        }
        self.zelf.as_ref()
    }

    pub const fn as_func(&self) -> &'static dyn PyNativeFn {
        self.value.func
    }
}

impl Callable for PyNativeFunction {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if let Some(z) = &zelf.zelf {
            args.prepend_arg(z.clone());
        }
        (zelf.value.func)(vm, args)
    }
}

#[pyclass(with(Callable, Unconstructible), flags(HAS_DICT))]
impl PyNativeFunction {
    #[pygetset]
    fn __module__(zelf: NativeFunctionOrMethod) -> Option<&'static PyStrInterned> {
        zelf.0.module
    }

    #[pygetset]
    fn __name__(zelf: NativeFunctionOrMethod) -> &'static str {
        zelf.0.value.name
    }

    #[pygetset]
    fn __qualname__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf.0;
        let flags = zelf.value.flags;
        // if flags.contains(PyMethodFlags::CLASS) || flags.contains(PyMethodFlags::STATIC) {
        let qualname = if let Some(bound) = &zelf.zelf {
            let prefix = if flags.contains(PyMethodFlags::CLASS) {
                bound
                    .get_attr("__qualname__", vm)
                    .unwrap()
                    .str(vm)
                    .unwrap()
                    .to_string()
            } else {
                bound.class().name().to_string()
            };
            vm.ctx.new_str(format!("{}.{}", prefix, &zelf.value.name))
        } else {
            vm.ctx.intern_str(zelf.value.name).to_owned()
        };
        Ok(qualname)
    }

    #[pygetset]
    fn __doc__(zelf: NativeFunctionOrMethod) -> Option<&'static str> {
        zelf.0.value.doc
    }

    #[pygetset(name = "__self__")]
    fn __self__(_zelf: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }

    #[pymethod]
    const fn __reduce__(&self) -> &'static str {
        // TODO: return (getattr, (self.object, self.name)) if this is a method
        self.value.name
    }

    #[pymethod]
    fn __reduce_ex__(zelf: PyObjectRef, _ver: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_special_method(&zelf, identifier!(vm, __reduce__), ())
    }

    #[pygetset]
    fn __text_signature__(zelf: NativeFunctionOrMethod) -> Option<&'static str> {
        let doc = zelf.0.value.doc?;
        let signature = type_::get_text_signature_from_internal_doc(zelf.0.value.name, doc)?;
        Some(signature)
    }
}

impl Representable for PyNativeFunction {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!("<built-in function {}>", zelf.value.name))
    }
}

impl Unconstructible for PyNativeFunction {}

// `PyCMethodObject` in CPython
#[pyclass(name = "builtin_method", module = false, base = "PyNativeFunction")]
pub struct PyNativeMethod {
    pub(crate) func: PyNativeFunction,
    pub(crate) class: &'static Py<PyType>, // TODO: the actual life is &'self
}

#[pyclass(
    with(Unconstructible, Callable, Comparable, Representable),
    flags(HAS_DICT)
)]
impl PyNativeMethod {
    #[pygetset]
    fn __qualname__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let prefix = zelf.class.name().to_string();
        Ok(vm
            .ctx
            .new_str(format!("{}.{}", prefix, &zelf.func.value.name)))
    }

    #[pymethod]
    fn __reduce__(
        &self,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, (PyObjectRef, &'static str))> {
        // TODO: return (getattr, (self.object, self.name)) if this is a method
        let getattr = vm.builtins.get_attr("getattr", vm)?;
        let target = self
            .func
            .zelf
            .clone()
            .unwrap_or_else(|| self.class.to_owned().into());
        let name = self.func.value.name;
        Ok((getattr, (target, name)))
    }

    #[pygetset(name = "__self__")]
    fn __self__(zelf: PyRef<Self>, _vm: &VirtualMachine) -> Option<PyObjectRef> {
        zelf.func.zelf.clone()
    }
}

impl PyPayload for PyNativeMethod {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.builtin_method_type
    }
}

impl fmt::Debug for PyNativeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "builtin method of {:?} with {:?}",
            &*self.class.name(),
            &self.func
        )
    }
}

impl Comparable for PyNativeMethod {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            if let Some(other) = other.downcast_ref::<Self>() {
                let eq = match (zelf.func.zelf.as_ref(), other.func.zelf.as_ref()) {
                    (Some(z), Some(o)) => z.is(o),
                    (None, None) => true,
                    _ => false,
                };
                let eq = eq && std::ptr::eq(zelf.func.value, other.func.value);
                Ok(eq.into())
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        })
    }
}

impl Callable for PyNativeMethod {
    type Args = FuncArgs;

    #[inline]
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if let Some(zelf) = &zelf.func.zelf {
            args.prepend_arg(zelf.clone());
        }
        (zelf.func.value.func)(vm, args)
    }
}

impl Representable for PyNativeMethod {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<built-in method {} of {} object at ...>",
            &zelf.func.value.name,
            zelf.class.name()
        ))
    }
}

impl Unconstructible for PyNativeMethod {}

pub fn init(context: &Context) {
    PyNativeFunction::extend_class(context, context.types.builtin_function_or_method_type);
    PyNativeMethod::extend_class(context, context.types.builtin_method_type);
}

struct NativeFunctionOrMethod(PyRef<PyNativeFunction>);

impl TryFromObject for NativeFunctionOrMethod {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = vm.ctx.types.builtin_function_or_method_type;
        if obj.fast_isinstance(class) {
            Ok(Self(unsafe { obj.downcast_unchecked() }))
        } else {
            Err(vm.new_downcast_type_error(class, &obj))
        }
    }
}

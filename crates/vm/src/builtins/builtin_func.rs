use super::{PyStrInterned, PyStrRef, PyType, type_};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::wtf8::Wtf8,
    convert::TryFromObject,
    function::{FuncArgs, PyComparisonValue, PyMethodDef, PyMethodFlags, PyNativeFn},
    types::{Callable, Comparable, PyComparisonOp, Representable},
};
use alloc::fmt;

// PyCFunctionObject in CPython
#[repr(C)]
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
    pub fn get_self(&self) -> Option<&PyObject> {
        if self.value.flags.contains(PyMethodFlags::STATIC) {
            return None;
        }
        self.zelf.as_deref()
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
            // STATIC methods store the class in zelf for qualname/repr purposes,
            // but should not prepend it to args (the Rust function doesn't expect it).
            if !zelf.value.flags.contains(PyMethodFlags::STATIC) {
                args.prepend_arg(z.clone());
            }
        }
        (zelf.value.func)(vm, args)
    }
}

// meth_richcompare in CPython
impl Comparable for PyNativeFunction {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            if let Some(other) = other.downcast_ref::<Self>() {
                let eq = match (zelf.zelf.as_ref(), other.zelf.as_ref()) {
                    (Some(z), Some(o)) => z.is(o),
                    (None, None) => true,
                    _ => false,
                };
                let eq = eq && core::ptr::eq(zelf.value, other.value);
                Ok(eq.into())
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        })
    }
}

// meth_repr in CPython
impl Representable for PyNativeFunction {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        if let Some(bound) = zelf
            .zelf
            .as_ref()
            .filter(|b| !b.class().is(vm.ctx.types.module_type))
        {
            Ok(format!(
                "<built-in method {} of {} object at {:#x}>",
                zelf.value.name,
                bound.class().name(),
                bound.get_id()
            ))
        } else {
            Ok(format!("<built-in function {}>", zelf.value.name))
        }
    }
}

#[pyclass(
    with(Callable, Comparable, Representable),
    flags(HAS_DICT, DISALLOW_INSTANTIATION)
)]
impl PyNativeFunction {
    #[pygetset]
    fn __module__(zelf: NativeFunctionOrMethod) -> Option<&'static PyStrInterned> {
        zelf.0.module
    }

    #[pygetset]
    fn __name__(zelf: NativeFunctionOrMethod) -> &'static str {
        zelf.0.value.name
    }

    // meth_get__qualname__ in CPython
    #[pygetset]
    fn __qualname__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf.0;
        let qualname = if let Some(bound) = &zelf.zelf {
            if bound.class().is(vm.ctx.types.module_type) {
                return Ok(vm.ctx.intern_str(zelf.value.name).to_owned());
            }
            let prefix = if bound.class().is(vm.ctx.types.type_type) {
                // m_self is a type: use PyType_GetQualName(m_self)
                bound.get_attr("__qualname__", vm)?.str(vm)?.to_string()
            } else {
                // m_self is an instance: use Py_TYPE(m_self).__qualname__
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

    // meth_get__self__ in CPython
    #[pygetset]
    fn __self__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyObjectRef {
        zelf.0.zelf.clone().unwrap_or_else(|| vm.ctx.none())
    }

    // meth_reduce in CPython
    #[pymethod]
    fn __reduce__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyResult {
        let zelf = zelf.0;
        if zelf.zelf.is_none() || zelf.module.is_some() {
            Ok(vm.ctx.new_str(zelf.value.name).into())
        } else {
            let getattr = vm.builtins.get_attr("getattr", vm)?;
            let target = zelf.zelf.clone().unwrap();
            Ok(vm.new_tuple((getattr, (target, zelf.value.name))).into())
        }
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

// PyCMethodObject in CPython
// repr(C) ensures `func` is at offset 0, allowing safe cast from PyNativeMethod to PyNativeFunction
#[repr(C)]
#[pyclass(name = "builtin_function_or_method", module = false, base = PyNativeFunction, ctx = "builtin_function_or_method_type")]
pub struct PyNativeMethod {
    pub(crate) func: PyNativeFunction,
    pub(crate) class: &'static Py<PyType>, // TODO: the actual life is &'self
}

// All Python-visible behavior (getters, slots) is registered by PyNativeFunction::extend_class.
// PyNativeMethod only extends the Rust-side struct with the defining class reference.
// The func field at offset 0 (#[repr(C)]) allows NativeFunctionOrMethod to read it safely.
#[pyclass(flags(HAS_DICT, DISALLOW_INSTANTIATION))]
impl PyNativeMethod {}

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

pub fn init(context: &Context) {
    PyNativeFunction::extend_class(context, context.types.builtin_function_or_method_type);
}

/// Wrapper that provides access to the common PyNativeFunction data
/// for both PyNativeFunction and PyNativeMethod (which has func as its first field).
struct NativeFunctionOrMethod(PyRef<PyNativeFunction>);

impl TryFromObject for NativeFunctionOrMethod {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = vm.ctx.types.builtin_function_or_method_type;
        if obj.fast_isinstance(class) {
            // Both PyNativeFunction and PyNativeMethod share the same type now.
            // PyNativeMethod has `func: PyNativeFunction` as its first field,
            // so we can safely treat the data pointer as PyNativeFunction for reading.
            Ok(Self(unsafe { obj.downcast_unchecked() }))
        } else {
            Err(vm.new_downcast_type_error(class, &obj))
        }
    }
}

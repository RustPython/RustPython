use super::{PyStrInterned, PyStrRef, PyType, type_};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::wtf8::Wtf8,
    convert::TryFromObject,
    function::{Callee, FuncArgs, PyComparisonValue, PyMethodDef, PyMethodFlags, PyNativeFn},
    object::{Traverse, TraverseFn},
    types::{Callable, Comparable, PyComparisonOp, Representable},
};
use alloc::fmt;

// PyCFunctionObject in CPython
#[repr(C)]
#[pyclass(
    name = "builtin_function_or_method",
    module = false,
    traverse = "manual"
)]
pub struct PyNativeFunction {
    pub(crate) value: &'static PyMethodDef,
    pub(crate) zelf: Option<PyObjectRef>,
    // Module that owns this function. Not passed as a call argument.
    pub(crate) module_object: Option<PyObjectRef>,
    #[pymember(name = "__module__", writable)]
    pub(crate) module: crate::object::PyAtomicRef<Option<PyObject>>,
    /// Prevent HeapMethodDef from being freed while this function references it
    pub(crate) _method_def_owner: Option<PyObjectRef>,
}

unsafe impl Traverse for PyNativeFunction {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.zelf.traverse(tracer_fn);
        self.module_object.traverse(tracer_fn);
        self.module.traverse(tracer_fn);
        self._method_def_owner.traverse(tracer_fn);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        out.extend(self.zelf.take());
        out.extend(self.module_object.take());
        // GC has exclusive access while clearing this unreachable object.
        out.extend(unsafe { self.module.swap(None) });
        // Keep the definition owner until deallocation: `value` borrows it.
    }
}

impl PyPayload for PyNativeFunction {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.builtin_function_or_method_type
    }
}

impl fmt::Debug for PyNativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let module = match self
            .module
            .deref()
            .and_then(|m| m.downcast_ref::<crate::builtins::PyStr>())
        {
            Some(module) => module.as_wtf8().to_owned(),
            None => Wtf8::new("<unknown>").to_owned(),
        };
        write!(
            f,
            "builtin function {}.{} ({:?}) self as instance of {:?}",
            module,
            self.value.name,
            self.value.flags,
            self.zelf.as_ref().map(|z| z.class().name().to_owned())
        )
    }
}

impl PyNativeFunction {
    pub fn with_module(self, module: &'static PyStrInterned) -> Self {
        drop(self.module.store(Some(module.to_owned().into())));
        self
    }

    pub fn with_module_object(mut self, module: PyObjectRef) -> Self {
        self.module_object = Some(module);
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
        self.zelf.as_deref().or(self.module_object.as_deref())
    }

    pub const fn as_func(&self) -> &dyn PyNativeFn {
        self.value.func
    }
}

impl Callable for PyNativeFunction {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.kwargs.is_empty() && !zelf.value.flags.contains(PyMethodFlags::KEYWORDS) {
            return Err(native_no_keywords_error(zelf, vm)?);
        }
        let mut callee = Callee::named(zelf.value.name);
        if let Some(z) = &zelf.zelf {
            // STATIC methods store the class in zelf for qualname/repr purposes,
            // but should not prepend it to args (the Rust function doesn't expect it).
            if !zelf.value.flags.contains(PyMethodFlags::STATIC) {
                args.prepend_arg(z.clone());
                callee = callee.with_instance_arg(true);
            }
        }
        (zelf.value.func)(vm, args, callee)
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
                let eq = eq
                    && match (zelf.module_object.as_ref(), other.module_object.as_ref()) {
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
    flags(HAS_WEAKREF, DISALLOW_INSTANTIATION)
)]
impl PyNativeFunction {
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
            vm.ctx.new_str(format!("{}.{}", prefix, zelf.value.name))
        } else {
            vm.ctx.intern_str(zelf.value.name).to_owned()
        };
        Ok(qualname)
    }

    // meth_get__doc__ in CPython
    #[pygetset]
    fn __doc__(zelf: NativeFunctionOrMethod) -> Option<&'static str> {
        type_::rendered_item_doc(zelf.0.value.name, zelf.0.value.item_doc())
    }

    // meth_get__self__ in CPython
    #[pygetset]
    fn __self__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(bound) = &zelf.0.zelf {
            return bound.clone();
        }
        if let Some(module) = &zelf.0.module_object {
            return module.clone();
        }
        vm.ctx.none()
    }

    // meth_reduce: the name when unbound or bound to a module, otherwise
    // `(getattr, (self, name))`. `__module__` is not part of the decision.
    #[pymethod]
    fn __reduce__(zelf: NativeFunctionOrMethod, vm: &VirtualMachine) -> PyResult {
        let zelf = zelf.0;
        // PyModule_Check: the type or a subtype, same as meth_reduce.
        if let Some(bound) = zelf
            .zelf
            .as_ref()
            .filter(|bound| !bound.class().is_subtype(vm.ctx.types.module_type))
        {
            let getattr = vm.builtins.get_attr("getattr", vm)?;
            Ok(vm
                .new_tuple((getattr, (bound.clone(), zelf.value.name)))
                .into())
        } else {
            Ok(vm.ctx.new_str(zelf.value.name).into())
        }
    }

    #[pymethod]
    fn __reduce_ex__(zelf: PyObjectRef, _ver: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_special_method(&zelf, identifier!(vm, __reduce__), ())
    }

    #[pygetset]
    fn __text_signature__(zelf: NativeFunctionOrMethod) -> Option<&'static str> {
        let doc = zelf.0.value.doc?;
        type_::get_text_signature_from_internal_doc(zelf.0.value.name, doc)
    }
}

// Bound METH_METHOD object. The payload starts with PyNativeFunction so it can
// be read as a builtin function. The Python type is `builtin_method`.
#[repr(C)]
#[pyclass(
    name = "builtin_method",
    module = false,
    base = PyNativeFunction,
    ctx = "builtin_method_type",
    traverse = "manual"
)]
pub struct PyNativeMethod {
    pub(crate) func: PyNativeFunction,
    pub(crate) class: &'static Py<PyType>,
}

unsafe impl Traverse for PyNativeMethod {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.func.traverse(tracer_fn);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        self.func.clear(out);
    }
}

#[pyclass(flags(HAS_WEAKREF, DISALLOW_INSTANTIATION))]
impl PyNativeMethod {}

impl fmt::Debug for PyNativeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "builtin method of {:?} with {:?}",
            &*self.class.name(),
            self.func
        )
    }
}

/// Vectorcall for builtin functions (PEP 590).
/// Avoids `prepend_arg` O(n) shift by building args with self at front.
fn vectorcall_native_function(
    zelf_obj: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyNativeFunction> = zelf_obj.downcast_ref().unwrap();

    if kwnames.is_some_and(|names| !names.is_empty())
        && !zelf.value.flags.contains(PyMethodFlags::KEYWORDS)
    {
        return Err(native_no_keywords_error(zelf, vm)?);
    }

    // Build FuncArgs with self already at position 0 (no insert(0) needed)
    let needs_self = zelf
        .zelf
        .as_ref()
        .is_some_and(|_| !zelf.value.flags.contains(PyMethodFlags::STATIC));

    let func_args = if needs_self {
        let self_obj = zelf.zelf.as_ref().unwrap().clone();
        let mut all_args = Vec::with_capacity(args.len() + 1);
        all_args.push(self_obj);
        all_args.extend(args);
        FuncArgs::from_vectorcall_owned(all_args, nargs + 1, kwnames)
    } else {
        FuncArgs::from_vectorcall_owned(args, nargs, kwnames)
    };

    let callee = Callee::named(zelf.value.name).with_instance_arg(needs_self);
    (zelf.value.func)(vm, func_args, callee)
}

#[cold]
fn native_no_keywords_error(
    zelf: &Py<PyNativeFunction>,
    vm: &VirtualMachine,
) -> PyResult<crate::exceptions::types::PyBaseExceptionRef> {
    if zelf.value.flags.contains(PyMethodFlags::VARARGS) {
        return Ok(vm.new_type_error(format!("{}() takes no keyword arguments", zelf.value.name)));
    }
    let qualname = PyNativeFunction::__qualname__(NativeFunctionOrMethod(zelf.to_owned()), vm)?;
    let module = zelf
        .module
        .load_owned()
        .filter(|module| !vm.is_none(module));
    let module = module.map(|module| module.str(vm)).transpose()?;
    let name = if let Some(module) = module
        && module.to_string() != "builtins"
    {
        format!("{module}.{qualname}")
    } else {
        qualname.to_string()
    };
    Ok(vm.new_type_error(format!("{name}() takes no keyword arguments")))
}

pub(crate) fn init(context: &'static Context) {
    PyNativeFunction::extend_class(context, context.types.builtin_function_or_method_type);
    context
        .types
        .builtin_function_or_method_type
        .slots
        .vectorcall
        .store(Some(vectorcall_native_function));
    PyNativeMethod::extend_class(context, context.types.builtin_method_type);
}

/// Wrapper that provides access to the common PyNativeFunction data
/// for both PyNativeFunction and PyNativeMethod (which has func as its first field).
struct NativeFunctionOrMethod(PyRef<PyNativeFunction>);

impl TryFromObject for NativeFunctionOrMethod {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = vm.ctx.types.builtin_function_or_method_type;
        if obj.fast_isinstance(class) {
            // `builtin_method` is a subclass; the payload starts with PyNativeFunction.
            Ok(Self(unsafe { obj.downcast_unchecked() }))
        } else {
            Err(vm.new_downcast_type_error(class, &obj))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Interpreter, PyObjectRef,
        function::{PyMethodDef, PyMethodFlags},
    };

    #[test]
    fn builtin_method_is_subclass_of_builtin_function() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let function_type = vm.ctx.types.builtin_function_or_method_type;
            let method_type = vm.ctx.types.builtin_method_type;
            assert!(!function_type.is(method_type));
            assert_eq!(&*function_type.name(), "builtin_function_or_method");
            assert_eq!(&*method_type.name(), "builtin_method");
            assert!(method_type.fast_issubclass(function_type));
        });
    }

    #[test]
    fn bound_instance_method_uses_function_type() {
        fn identity(value: PyObjectRef) -> PyObjectRef {
            value
        }
        const DEF: PyMethodDef = PyMethodDef::new_const(
            "identity",
            identity,
            PyMethodFlags::METHOD,
            crate::function::ItemDoc::NONE,
        );

        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let bound = DEF.build_bound_function(&vm.ctx, vm.ctx.none());
            assert!(
                bound
                    .class()
                    .is(vm.ctx.types.builtin_function_or_method_type)
            );
            assert!(!bound.as_object().downcastable::<PyNativeMethod>());

            let method = DEF.build_bound_method(&vm.ctx, vm.ctx.none(), vm.ctx.types.object_type);
            assert!(method.class().is(vm.ctx.types.builtin_method_type));
            assert!(method.as_object().downcastable::<PyNativeFunction>());
            assert!(method.as_object().downcastable::<PyNativeMethod>());
        });
    }
}

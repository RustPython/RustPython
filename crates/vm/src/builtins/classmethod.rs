use super::{
    PyBoundMethod, PyGenericAlias, PyStr, PyStrInterned, PyType, PyTypeRef, object_get_dict,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::{PyClassDef, PyClassImpl},
    function::{FuncArgs, PySetterValue},
    object::PyAtomicRef,
    types::{Constructor, GetDescriptor, Initializer},
};

#[pyclass(module = false, name = "classmethod", traverse)]
#[derive(Debug)]
pub struct PyClassMethod {
    #[pymember(name = "__func__")]
    #[pymember(name = "__wrapped__")]
    callable: PyAtomicRef<PyObject>,
}

impl From<PyObjectRef> for PyClassMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyAtomicRef::from(callable),
        }
    }
}

impl PyPayload for PyClassMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.classmethod_type
    }
}

impl GetDescriptor for PyClassMethod {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(zelf, obj, vm)?;
        let cls = match cls {
            Some(cls) => cls.to_owned(),
            None => _obj.class().to_owned().into(),
        };
        let callable = zelf.callable.load_owned();
        Ok(PyBoundMethod::new(cls, callable).into_ref(&vm.ctx).into())
    }
}

#[derive(FromArgs)]
pub struct ClassMethodArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
}

impl Constructor for PyClassMethod {
    type Args = ClassMethodArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: ClassMethodArgs = args.bind_for(vm, Self::NAME)?;
        let result = Self {
            callable: PyAtomicRef::from(args.function.clone()),
        }
        .into_ref_with_type(vm, cls)?;
        functools_wraps(result.as_object(), &args.function, vm)?;
        Ok(result.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyClassMethod {
    type Args = ClassMethodArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        descriptor_set_callable(zelf.as_object(), &zelf.callable, args.function, vm)
    }
}

impl PyClassMethod {
    #[deprecated(note = "use PyClassMethod::from(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::from(callable).into_ref(ctx)
    }
}

#[pyclass(
    with(GetDescriptor, Constructor, Initializer),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
)]
impl Py<PyClassMethod> {
    #[pygetset]
    fn __annotations__(zelf: &Self, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotations__(zelf: &Self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            value,
            "classmethod",
            vm,
        )
    }

    #[pygetset]
    fn __annotate__(zelf: &Self, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotate__(zelf: &Self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            value,
            "classmethod",
            vm,
        )
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.callable.load_owned().is_abstract(vm)
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }

    #[inline]
    #[pymethod]
    fn __repr__(zelf: &Self, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.load_owned().repr(vm)?;
        let class = PyClassMethod::class(&vm.ctx);

        let module = class.__module__(vm)?;
        let repr = match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_wtf8()),
            module.downcast_ref::<PyStr>().map(|m| m.as_wtf8()),
        ) {
            (None, _) => return Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => {
                format!("<{module}.{qualname}({callable})>")
            }
            _ => format!("<{}({})>", class.slot_name(), callable),
        };
        Ok(repr)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyClassMethod::extend_class(context, context.types.classmethod_type);
}

pub(crate) fn functools_wraps(
    wrapper: &PyObject,
    wrapped: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<()> {
    for attr in [
        identifier!(vm.ctx, __module__),
        identifier!(vm.ctx, __name__),
        identifier!(vm.ctx, __qualname__),
        identifier!(vm.ctx, __doc__),
    ] {
        if let Some(value) = vm.get_attribute_opt(wrapped, attr)? {
            wrapper.set_attr(attr, value, vm)?;
        }
    }
    Ok(())
}

pub(crate) fn descriptor_get_wrapped_attribute(
    wrapped: &PyObject,
    obj: &PyObject,
    name: &'static PyStrInterned,
    vm: &VirtualMachine,
) -> PyResult {
    let dict = object_get_dict(obj.to_owned(), vm)?;
    if let Some(res) = dict.get_item_opt(name, vm)? {
        return Ok(res);
    }
    let res = wrapped.get_attr(name, vm)?;
    dict.set_item(name, res.clone(), vm)?;
    Ok(res)
}

pub(crate) fn descriptor_set_wrapped_attribute(
    obj: &PyObject,
    name: &'static PyStrInterned,
    value: PySetterValue,
    type_name: &str,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let dict = object_get_dict(obj.to_owned(), vm)?;
    match value {
        PySetterValue::Delete => match dict.del_item(name, vm) {
            Ok(()) => Ok(()),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => Err(vm
                .new_attribute_error(format!(
                    "'{type_name}' object has no attribute '{}'",
                    name.as_str()
                ))),
            Err(e) => Err(e),
        },
        PySetterValue::Assign(value) => dict.set_item(name, value, vm),
    }
}

// Both wrapper types publish the callable before metadata callbacks. A replaced
// callable's finalizer may reinitialize the descriptor, so reload after dropping it.
pub(crate) fn descriptor_set_callable(
    wrapper: &PyObject,
    slot: &PyAtomicRef<PyObject>,
    callable: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if slot.load_owned().is(&callable) {
        return Ok(());
    }
    drop(slot.store(callable.clone()));
    let current = slot.load_owned();
    let result = functools_wraps(wrapper, &current, vm);
    drop(current);
    drop(callable);
    result
}

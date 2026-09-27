use super::{
    PyGenericAlias, PyStr, PyType, PyTypeRef,
    classmethod::{
        descriptor_get_wrapped_attribute, descriptor_set_wrapped_attribute, functools_wraps,
    },
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::{PyClassDef, PyClassImpl},
    function::{FuncArgs, PySetterValue},
    object::PyAtomicRef,
    types::{Callable, Constructor, GetDescriptor, Initializer, Representable},
};

#[pyclass(module = false, name = "staticmethod", traverse)]
#[derive(Debug)]
pub struct PyStaticMethod {
    #[pymember(name = "__func__")]
    #[pymember(name = "__wrapped__")]
    pub callable: PyAtomicRef<PyObject>,
}

impl PyPayload for PyStaticMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.staticmethod_type
    }
}

impl GetDescriptor for PyStaticMethod {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(zelf, obj, vm)?;
        Ok(zelf.callable.load_owned())
    }
}

impl From<PyObjectRef> for PyStaticMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyAtomicRef::from(callable),
        }
    }
}

#[derive(FromArgs)]
pub struct StaticMethodArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
}

impl Constructor for PyStaticMethod {
    type Args = StaticMethodArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Validate the signature here, but defer storing the callable and
        // copying its attributes to `__init__` so that subclasses overriding
        // `__init__` without calling `super().__init__()` see `__func__` as
        // `None`, matching CPython.
        let _: StaticMethodArgs = args.bind_for(vm, Self::NAME)?;
        let result = Self {
            callable: PyAtomicRef::from(vm.ctx.none()),
        }
        .into_ref_with_type(vm, cls)?;
        Ok(PyObjectRef::from(result))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl PyStaticMethod {
    #[must_use]
    pub fn new(callable: PyObjectRef) -> Self {
        Self {
            callable: PyAtomicRef::from(callable),
        }
    }

    #[deprecated(note = "use PyStaticMethod::new(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::new(callable).into_ref(ctx)
    }
}

impl Initializer for PyStaticMethod {
    type Args = StaticMethodArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let callable = args.function;
        zelf.callable.store(callable.clone());
        functools_wraps(zelf.as_object(), &callable, vm)
    }
}

#[pyclass(
    with(Callable, GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
)]
impl PyStaticMethod {
    #[pygetset]
    fn __annotations__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotations__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            value,
            "staticmethod",
            vm,
        )
    }

    #[pygetset]
    fn __annotate__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotate__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            value,
            "staticmethod",
            vm,
        )
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyObjectRef {
        let callable = self.callable.load_owned();

        if let Ok(Some(is_abstract)) = vm.get_attribute_opt(&callable, "__isabstractmethod__") {
            is_abstract
        } else {
            vm.ctx.new_bool(false).into()
        }
    }

    #[pygetset(setter)]
    fn set___isabstractmethod__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable
            .load_owned()
            .set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }
}

impl Callable for PyStaticMethod {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        callable.call(args, vm)
    }
}

impl Representable for PyStaticMethod {
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.load_owned().repr(vm)?;
        let class = Self::class(&vm.ctx);

        match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_wtf8()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_wtf8()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => {
                Ok(format!("<{module}.{qualname}({callable})>"))
            }
            _ => Ok(format!("<{}({})>", class.slot_name(), callable)),
        }
    }
}

pub(crate) fn init(context: &'static Context) {
    PyStaticMethod::extend_class(context, context.types.staticmethod_type);
}

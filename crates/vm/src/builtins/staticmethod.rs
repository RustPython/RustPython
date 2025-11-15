use super::{PyGenericAlias, PyStr, PyType, PyTypeRef};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::lock::PyMutex,
    function::FuncArgs,
    types::{Callable, Constructor, GetDescriptor, Initializer, Representable},
};

#[pyclass(module = false, name = "staticmethod", traverse)]
#[derive(Debug)]
pub struct PyStaticMethod {
    pub callable: PyMutex<PyObjectRef>,
}

impl PyPayload for PyStaticMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.staticmethod_type
    }
}

impl GetDescriptor for PyStaticMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(&zelf, obj, vm)?;
        Ok(zelf.callable.lock().clone())
    }
}

impl From<PyObjectRef> for PyStaticMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyMutex::new(callable),
        }
    }
}

impl Constructor for PyStaticMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let doc = callable.get_attr("__doc__", vm);

        let result = Self {
            callable: PyMutex::new(callable),
        }
        .into_ref_with_type(vm, cls)?;
        let obj = PyObjectRef::from(result);

        if let Ok(doc) = doc {
            obj.set_attr("__doc__", doc, vm)?;
        }

        Ok(obj)
    }
}

impl PyStaticMethod {
    pub fn new(callable: PyObjectRef) -> Self {
        Self {
            callable: PyMutex::new(callable),
        }
    }
    #[deprecated(note = "use PyStaticMethod::new(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::new(callable).into_ref(ctx)
    }
}

impl Initializer for PyStaticMethod {
    type Args = PyObjectRef;

    fn init(zelf: PyRef<Self>, callable: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        *zelf.callable.lock() = callable;
        Ok(())
    }
}

#[pyclass(
    with(Callable, GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT)
)]
impl PyStaticMethod {
    #[pygetset]
    fn __func__(&self) -> PyObjectRef {
        self.callable.lock().clone()
    }

    #[pygetset]
    fn __wrapped__(&self) -> PyObjectRef {
        self.callable.lock().clone()
    }

    #[pygetset]
    fn __module__(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__module__", vm)
    }

    #[pygetset]
    fn __qualname__(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__qualname__", vm)
    }

    #[pygetset]
    fn __name__(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__name__", vm)
    }

    #[pygetset]
    fn __annotations__(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__annotations__", vm)
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyObjectRef {
        match vm.get_attribute_opt(self.callable.lock().clone(), "__isabstractmethod__") {
            Ok(Some(is_abstract)) => is_abstract,
            _ => vm.ctx.new_bool(false).into(),
        }
    }

    #[pygetset(setter)]
    fn set___isabstractmethod__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable
            .lock()
            .set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }

    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

impl Callable for PyStaticMethod {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.lock().clone();
        callable.call(args, vm)
    }
}

impl Representable for PyStaticMethod {
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.lock().repr(vm).unwrap();
        let class = Self::class(&vm.ctx);

        match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_str()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => {
                Ok(format!("<{module}.{qualname}({callable})>"))
            }
            _ => Ok(format!("<{}({})>", class.slot_name(), callable)),
        }
    }
}

pub fn init(context: &Context) {
    PyStaticMethod::extend_class(context, context.types.staticmethod_type);
}

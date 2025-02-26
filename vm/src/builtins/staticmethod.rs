use super::{PyStr, PyType, PyTypeRef};
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
        let x = Ok(zelf.callable.lock().clone());
        x
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

        let result = PyStaticMethod {
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
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(
            Self {
                callable: PyMutex::new(callable),
            },
            ctx.types.staticmethod_type.to_owned(),
            None,
        )
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
    #[pygetset(magic)]
    fn func(&self) -> PyObjectRef {
        self.callable.lock().clone()
    }

    #[pygetset(magic)]
    fn wrapped(&self) -> PyObjectRef {
        self.callable.lock().clone()
    }

    #[pygetset(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__module__", vm)
    }

    #[pygetset(magic)]
    fn qualname(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__qualname__", vm)
    }

    #[pygetset(magic)]
    fn name(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__name__", vm)
    }

    #[pygetset(magic)]
    fn annotations(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__annotations__", vm)
    }

    #[pygetset(magic)]
    fn isabstractmethod(&self, vm: &VirtualMachine) -> PyObjectRef {
        match vm.get_attribute_opt(self.callable.lock().clone(), "__isabstractmethod__") {
            Ok(Some(is_abstract)) => is_abstract,
            _ => vm.ctx.new_bool(false).into(),
        }
    }

    #[pygetset(magic, setter)]
    fn set_isabstractmethod(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable
            .lock()
            .set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
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
                .qualname(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class.module(vm).downcast_ref::<PyStr>().map(|m| m.as_str()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name".into())),
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

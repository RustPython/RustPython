use super::{PyBoundMethod, PyStr, PyType, PyTypeRef};
use crate::{
    class::PyClassImpl,
    common::lock::PyMutex,
    types::{Constructor, GetDescriptor, Initializer},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

/// classmethod(function) -> method
///
/// Convert a function to be a class method.
///
/// A class method receives the class as implicit first argument,
/// just like an instance method receives the instance.
/// To declare a class method, use this idiom:
///
///   class C:
///       @classmethod
///       def f(cls, arg1, arg2, ...):
///           ...
///
/// It can be called either on the class (e.g. C.f()) or on an instance
/// (e.g. C().f()).  The instance is ignored except for its class.
/// If a class method is called for a derived class, the derived class
/// object is passed as the implied first argument.
///
/// Class methods are different than C++ or Java static methods.
/// If you want those, see the staticmethod builtin.
#[pyclass(module = false, name = "classmethod")]
#[derive(Debug)]
pub struct PyClassMethod {
    callable: PyMutex<PyObjectRef>,
}

impl From<PyObjectRef> for PyClassMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyMutex::new(callable),
        }
    }
}

impl PyPayload for PyClassMethod {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.classmethod_type
    }
}

impl GetDescriptor for PyClassMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| _obj.class().clone().into());
        let call_descr_get: PyResult<PyObjectRef> = zelf.callable.lock().get_attr("__get__", vm);
        match call_descr_get {
            Err(_) => Ok(PyBoundMethod::new_ref(cls, zelf.callable.lock().clone(), &vm.ctx).into()),
            Ok(call_descr_get) => vm.invoke(&call_descr_get, (cls.clone(), cls)),
        }
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let doc = callable.get_attr("__doc__", vm);

        let result = PyClassMethod {
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

impl Initializer for PyClassMethod {
    type Args = PyObjectRef;

    fn init(zelf: PyRef<Self>, callable: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        *zelf.callable.lock() = callable;
        Ok(())
    }
}

impl PyClassMethod {
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(
            Self {
                callable: PyMutex::new(callable),
            },
            ctx.types.classmethod_type.to_owned(),
            None,
        )
    }
}

#[pyclass(with(GetDescriptor, Constructor), flags(BASETYPE, HAS_DICT))]
impl PyClassMethod {
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

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> Option<String> {
        let callable = self.callable.lock().repr(vm).unwrap();
        let class = Self::class(vm);

        match (
            class
                .qualname(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class.module(vm).downcast_ref::<PyStr>().map(|m| m.as_str()),
        ) {
            (None, _) => None,
            (Some(qualname), Some(module)) if module != "builtins" => {
                Some(format!("<{}.{}({})>", module, qualname, callable))
            }
            _ => Some(format!("<{}({})>", class.slot_name(), callable)),
        }
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

pub(crate) fn init(context: &Context) {
    PyClassMethod::extend_class(context, context.types.classmethod_type);
}

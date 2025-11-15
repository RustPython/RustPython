use super::{PyBoundMethod, PyGenericAlias, PyStr, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::lock::PyMutex,
    types::{Constructor, GetDescriptor, Initializer, Representable},
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
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.classmethod_type
    }
}

impl GetDescriptor for PyClassMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(&zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| _obj.class().to_owned().into());
        let call_descr_get: PyResult<PyObjectRef> = zelf.callable.lock().get_attr("__get__", vm);
        match call_descr_get {
            Err(_) => Ok(PyBoundMethod::new(cls, zelf.callable.lock().clone())
                .into_ref(&vm.ctx)
                .into()),
            Ok(call_descr_get) => call_descr_get.call((cls.clone(), cls), vm),
        }
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        // Create a dictionary to hold copied attributes
        let dict = vm.ctx.new_dict();

        // Copy attributes from the callable to the dict
        // This is similar to functools.wraps in CPython
        if let Ok(doc) = callable.get_attr("__doc__", vm) {
            dict.set_item(identifier!(vm.ctx, __doc__), doc, vm)?;
        }
        if let Ok(name) = callable.get_attr("__name__", vm) {
            dict.set_item(identifier!(vm.ctx, __name__), name, vm)?;
        }
        if let Ok(qualname) = callable.get_attr("__qualname__", vm) {
            dict.set_item(identifier!(vm.ctx, __qualname__), qualname, vm)?;
        }
        if let Ok(module) = callable.get_attr("__module__", vm) {
            dict.set_item(identifier!(vm.ctx, __module__), module, vm)?;
        }
        if let Ok(annotations) = callable.get_attr("__annotations__", vm) {
            dict.set_item(identifier!(vm.ctx, __annotations__), annotations, vm)?;
        }

        // Create PyClassMethod instance with the pre-populated dict
        let classmethod = Self {
            callable: PyMutex::new(callable),
        };

        let result = PyRef::new_ref(classmethod, cls, Some(dict));
        Ok(PyObjectRef::from(result))
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
    #[deprecated(note = "use PyClassMethod::from(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::from(callable).into_ref(ctx)
    }
}

#[pyclass(
    with(GetDescriptor, Constructor, Representable),
    flags(BASETYPE, HAS_DICT)
)]
impl PyClassMethod {
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

impl Representable for PyClassMethod {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.lock().repr(vm).unwrap();
        let class = Self::class(&vm.ctx);

        let repr = match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_str()),
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

pub(crate) fn init(context: &Context) {
    PyClassMethod::extend_class(context, context.types.classmethod_type);
}

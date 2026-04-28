use super::{PyBoundMethod, PyGenericAlias, PyStr, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::lock::PyMutex,
    function::{FuncArgs, PySetterValue},
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
        let callable = zelf.callable.lock().clone();
        Ok(PyBoundMethod::new(cls, callable).into_ref(&vm.ctx).into())
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Validate the signature here, but defer storing the callable and
        // copying its attributes to `__init__` so that subclasses overriding
        // `__init__` without calling `super().__init__()` see `__func__` as
        // `None`, matching CPython.
        let _: Self::Args = args.bind(vm)?;
        let classmethod = Self {
            callable: PyMutex::new(vm.ctx.none()),
        };
        let result = PyRef::new_ref(classmethod, cls, Some(vm.ctx.new_dict()));
        Ok(PyObjectRef::from(result))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyClassMethod {
    type Args = PyObjectRef;

    fn init(zelf: PyRef<Self>, callable: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        *zelf.callable.lock() = callable.clone();
        // Copy wrapper attributes from the callable, mirroring functools.wraps.
        let dict = zelf.as_object().dict().expect("classmethod has __dict__");
        for attr in [
            identifier!(vm.ctx, __doc__),
            identifier!(vm.ctx, __name__),
            identifier!(vm.ctx, __qualname__),
            identifier!(vm.ctx, __module__),
            identifier!(vm.ctx, __annotations__),
        ] {
            if let Ok(value) = callable.get_attr(attr, vm) {
                dict.set_item(attr, value, vm)?;
            }
        }
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
    with(GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
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

    #[pygetset(setter)]
    fn set___annotations__(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        match value {
            PySetterValue::Assign(v) => self.callable.lock().set_attr("__annotations__", v, vm),
            PySetterValue::Delete => Ok(()), // Silently ignore delete like CPython
        }
    }

    #[pygetset]
    fn __annotate__(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.lock().get_attr("__annotate__", vm)
    }

    #[pygetset(setter)]
    fn set___annotate__(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        match value {
            PySetterValue::Assign(v) => self.callable.lock().set_attr("__annotate__", v, vm),
            PySetterValue::Delete => Ok(()), // Silently ignore delete like CPython
        }
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
                .map(|n| n.as_wtf8()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_wtf8()),
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

use super::{PyBoundMethod, PyType, PyTypeRef};
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
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| obj.class().clone().into());
        let callable = zelf.callable.lock().clone();
        Ok(PyBoundMethod::new_ref(cls, callable, &vm.ctx).into())
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyClassMethod {
            callable: PyMutex::new(callable),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
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

#[pyimpl(with(GetDescriptor, Constructor), flags(BASETYPE, HAS_DICT))]
impl PyClassMethod {
    #[pyproperty(magic)]
    fn func(&self) -> PyObjectRef {
        self.callable.lock().clone()
    }

    #[pyproperty(magic)]
    fn wrapped(&self) -> PyObjectRef {
        self.callable.clone()
    }

    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__module__", vm)
    }

    #[pyproperty(magic)]
    fn qualname(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__qualname__", vm)
    }

    #[pyproperty(magic)]
    fn name(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__name__", vm)
    }

    #[pyproperty(magic)]
    fn annotations(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__annotations__", vm)
    }

    #[pyproperty(magic)]
    fn doc(&self, vm: &VirtualMachine) -> PyResult {
        self.callable.get_attr("__doc__", vm)
    }

    #[pyproperty(magic)]
    fn isabstractmethod(&self, vm: &VirtualMachine) -> PyObjectRef {
        match vm.get_attribute_opt(self.callable.lock().clone(), "__isabstractmethod__") {
            Ok(Some(is_abstract)) => is_abstract,
            _ => vm.ctx.new_bool(false).into(),
        }
    }

    #[pyproperty(magic, setter)]
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

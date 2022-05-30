use super::{PyBoundMethod, PyType, PyTypeRef};
use crate::{
    class::PyClassImpl,
    types::{Constructor, GetDescriptor},
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
#[derive(Clone, Debug)]
pub struct PyClassMethod {
    callable: PyObjectRef,
}

impl From<PyObjectRef> for PyClassMethod {
    fn from(value: PyObjectRef) -> Self {
        Self { callable: value }
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
        Ok(PyBoundMethod::new_ref(cls, zelf.callable.clone(), &vm.ctx).into())
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyClassMethod { callable }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl PyClassMethod {
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(
            Self { callable },
            ctx.types.classmethod_type.to_owned(),
            None,
        )
    }
}

#[pyclass(with(GetDescriptor, Constructor), flags(BASETYPE, HAS_DICT))]
impl PyClassMethod {
    #[pyproperty(magic)]
    fn func(&self) -> PyObjectRef {
        self.callable.clone()
    }

    #[pyproperty(magic)]
    fn isabstractmethod(&self, vm: &VirtualMachine) -> PyObjectRef {
        match vm.get_attribute_opt(self.callable.clone(), "__isabstractmethod__") {
            Ok(Some(is_abstract)) => is_abstract,
            _ => vm.ctx.new_bool(false).into(),
        }
    }

    #[pyproperty(magic, setter)]
    fn set_isabstractmethod(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable.set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }
}

pub(crate) fn init(context: &Context) {
    PyClassMethod::extend_class(context, context.types.classmethod_type);
}

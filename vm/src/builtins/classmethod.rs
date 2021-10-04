use super::PyTypeRef;
use crate::{
    slots::{SlotConstructor, SlotDescriptor},
    PyClassImpl, PyContext, PyObjectRef, PyResult, PyValue, TypeProtocol, VirtualMachine,
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

impl PyValue for PyClassMethod {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.classmethod_type
    }
}

impl SlotDescriptor for PyClassMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| obj.clone_class().into());
        Ok(vm.ctx.new_bound_method(zelf.callable.clone(), cls))
    }
}

impl SlotConstructor for PyClassMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyClassMethod { callable }.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotDescriptor, SlotConstructor), flags(BASETYPE, HAS_DICT))]
impl PyClassMethod {
    #[pyproperty(magic)]
    fn func(&self) -> PyObjectRef {
        self.callable.clone()
    }
}

pub(crate) fn init(context: &PyContext) {
    PyClassMethod::extend_class(context, &context.types.classmethod_type);
}

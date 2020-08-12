use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

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
#[pyclass]
#[derive(Clone, Debug)]
pub struct PyClassMethod {
    callable: PyObjectRef,
}
pub type PyClassMethodRef = PyRef<PyClassMethod>;

impl From<PyObjectRef> for PyClassMethod {
    fn from(value: PyObjectRef) -> Self {
        Self { callable: value }
    }
}

impl PyValue for PyClassMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.classmethod_type()
    }
}

impl SlotDescriptor for PyClassMethod {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: OptionalArg<PyObjectRef>,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| obj.class().into_object());
        Ok(vm.ctx.new_bound_method(zelf.callable.clone(), cls))
    }
}

#[pyimpl(with(SlotDescriptor), flags(BASETYPE, HAS_DICT))]
impl PyClassMethod {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        callable: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyClassMethodRef> {
        PyClassMethod {
            callable: callable.clone(),
        }
        .into_ref_with_type(vm, cls)
    }

    #[pyproperty(name = "__func__")]
    fn func(&self) -> PyObjectRef {
        self.callable.clone()
    }
}

pub(crate) fn init(context: &PyContext) {
    PyClassMethod::extend_class(context, &context.types.classmethod_type);
}

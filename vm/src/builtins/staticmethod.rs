use super::PyTypeRef;
use crate::{
    slots::{SlotConstructor, SlotDescriptor},
    PyClassImpl, PyContext, PyObjectRef, PyResult, PyValue, VirtualMachine,
};

#[pyclass(module = false, name = "staticmethod")]
#[derive(Clone, Debug)]
pub struct PyStaticMethod {
    pub callable: PyObjectRef,
}

impl PyValue for PyStaticMethod {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.staticmethod_type
    }
}

impl SlotDescriptor for PyStaticMethod {
    fn descr_get(
        zelf: PyObjectRef,
        _obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let zelf = Self::_zelf(zelf, vm)?;
        Ok(zelf.callable.clone())
    }
}

impl From<PyObjectRef> for PyStaticMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self { callable }
    }
}

impl SlotConstructor for PyStaticMethod {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, callable: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyStaticMethod { callable }.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotDescriptor, SlotConstructor), flags(BASETYPE, HAS_DICT))]
impl PyStaticMethod {}

pub fn init(context: &PyContext) {
    PyStaticMethod::extend_class(context, &context.types.staticmethod_type);
}

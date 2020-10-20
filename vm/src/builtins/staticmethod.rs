use super::pytype::PyTypeRef;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

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

#[pyimpl(with(SlotDescriptor), flags(BASETYPE, HAS_DICT))]
impl PyStaticMethod {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, callable: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyStaticMethod { callable }.into_ref_with_type(vm, cls)
    }
}

pub fn init(context: &PyContext) {
    PyStaticMethod::extend_class(context, &context.types.staticmethod_type);
}

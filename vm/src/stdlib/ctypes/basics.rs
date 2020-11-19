use crate::builtins::PyTypeRef;
use crate::pyobject::{PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;

// This class is the equivalent of PyCData_Type on tp_base for
// PyCStructType_Type, UnionType_Type, PyCPointerType_Type
// PyCArrayType_Type, PyCSimpleType_Type, PyCFuncPtrType_Type
#[pyclass(module = false, name = "_CData")]
#[derive(Debug)]
pub struct PyCData {}

impl PyValue for PyCData {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_baseclass()
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyCData {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here

    // #[pyslot]
    // fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
    //     PyCData {}.into_ref_with_type(vm, cls)
    // }
}

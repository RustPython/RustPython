use crate::builtins::PyTypeRef;
use crate::pyobject::{PyObjectRc, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::PyCData_as_buffer;

// This class is the equivalent of PyCData_Type on tp_base for
// Struct_Type, Union_Type, PyCPointer_Type
// PyCArray_Type, PyCSimple_Type, PyCFuncPtr_Type

#[pyclass(module = false, name = "_CData")]
#[derive(Debug)]
pub struct PyCData {
    _objects: Vec<PyObjectRc>,
}

impl PyValue for PyCData {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::init_bare_type()
    }
}

#[pyimpl]
impl PyCData {
    // Methods here represent PyCData_methods

    #[pymethod]
    pub fn __ctypes_from_outparam__() {}

    #[pymethod]
    pub fn __reduce__() {}

    #[pymethod]
    pub fn __setstate__() {}
}

// #[pyimpl]
// impl PyCData_as_buffer for PyCData {

// }

// This class has no attributes and we care about it's methods
#[pyclass(module = false, name = "_CDataMeta")]
#[derive(Debug)]
pub struct PyCDataMeta;

impl PyValue for PyCDataMeta {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_baseclass()
    }
}

#[pyimpl]
impl PyCDataMeta {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here

    // The default methods (representing CDataType_methods) here are for:
    // StructType_Type
    // UnionType_Type
    // PyCArrayType_Type
    // PyCFuncPtrType_Type

    #[pymethod]
    pub fn from_param() {}

    #[pymethod]
    pub fn from_address() {}

    #[pymethod]
    pub fn from_buffer() {}

    #[pymethod]
    pub fn from_buffer_copy() {}

    #[pymethod]
    pub fn in_dll() {}
}

// CDataType_as_sequence methods are default for all types inherinting from PyCDataMeta
// Basically the sq_repeat slot is CDataType_repeat
// which transforms into a Array

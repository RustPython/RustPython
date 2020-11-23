use crate::builtins::int::PyInt;
use crate::builtins::memory::Buffer;
use crate::builtins::pystr::PyStrRef;
use crate::function::OptionalArg;
use crate::builtins::pytype::{PyType, PyTypeRef};
use crate::pyobject::{
    PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
};
use crate::slots::BufferProtocol;
use crate::VirtualMachine;

// GenericPyCData_new -> PyResult<PyObjectRef>
pub fn generic_pycdata_new(type_: PyTypeRef, vm: &VirtualMachine) {
    // @TODO: To be used on several places
}

#[pyimpl]
pub trait PyCDataMethods: PyValue {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here

    // The default methods (representing CDataType_methods) here are for:
    // StructType_Type
    // UnionType_Type
    // PyCArrayType_Type
    // PyCFuncPtrType_Type

    #[pyclassmethod]
    fn from_param(cls: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine)
        -> PyResult<PyRef<Self>>;

    #[pyclassmethod]
    fn from_address(
        cls: PyTypeRef,
        address: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>>;

    #[pyclassmethod]
    fn from_buffer(
        cls: PyTypeRef,
        obj: PyObjectRef,
        offset: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>>;

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        obj: PyObjectRef,
        offset: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>>;

    #[pyclassmethod]
    fn in_dll(
        cls: PyTypeRef,
        dll: PyObjectRef,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>>;
}

// This trait will be used by all types
pub trait PyCDataBuffer: BufferProtocol {
    // @TODO: Translate PyCData_NewGetBuffer
    fn get_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>>;
}

// This Trait is the equivalent of PyCData_Type on tp_base for
// Struct_Type, Union_Type, PyCPointer_Type
// PyCArray_Type, PyCSimple_Type, PyCFuncPtr_Type
#[pyclass(module = "ctypes", name = "_CData")]
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

    #[pymethod(name = "__ctypes_from_outparam__")]
    pub fn ctypes_from_outparam(&self) {}

    #[pymethod(name = "__reduce__")]
    pub fn reduce(&self) {}

    #[pymethod(name = "__setstate__")]
    pub fn setstate(&self) {}

    // CDataType_as_sequence methods are default for all types implementing PyCDataMethods
    // Basically the sq_repeat slot is CDataType_repeat
    // which transforms into a Array

    // #[pymethod(name = "__mul__")]
    // fn mul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
    // }

    // #[pymethod(name = "__rmul__")]
    // fn rmul(&self, counter: isize, vm: &VirtualMachine) -> PyObjectRef {
    //     self.mul(counter, &vm)
    // }
}

// #[pyimpl]
// impl PyCDataBuffer for PyCData {

// }

// #[pyimpl]
// impl PyCDataMethods for PyCData {
//     #[pyclassmethod]
//     fn from_address(
//         cls: PyTypeRef,
//         address: PyObjectRef,
//         vm: &VirtualMachine
//     ) -> PyResult<PyRef<Self>> {
//         if let Ok(obj) = address.downcast_exact::<PyInt>(vm) {
//             if let Ok(v) = usize::try_from_object(vm, obj.into_object()) {

//             } else {
//                 Err(vm.new_runtime_error("casting pointer failed".to_string()))
//             }
//         } else {
//             Err(vm.new_type_error("integer expected".to_string()))
//         }
//     }
// }

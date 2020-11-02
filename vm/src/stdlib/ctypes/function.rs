use libloading::{Library};
use ::std::{sync::Arc};

use crate::builtins::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::builtins::list::PyListRef;
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::pyobject::{PyValue, StaticType, PyRef, PyResult, PyObjectRef};

use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{CDataObject,FunctionProxy};


#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCFuncPtr {
    __name__: Option<String>,
    _argtypes_: Vec<PyRef<CDataObject>>,
    _restype_: Option<PyRef<CDataObject>>,
    ext_func: Option<FunctionProxy>,
}

impl PyValue for PyCFuncPtr {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PyCFuncPtr {
    // If a subclass fill the parameters, construct new objects with it
    // Else use default

    // #[inline]
    // pub fn new() -> PyCFuncPtr {
    //     PyCFuncPtr {
    //         __name__: None,
    //         _argtypes_: Vec::new(),
    //         _restype_: None,
    //         ext_func: None,
    //     }
    
    // }

    // #[pyproperty]
    // pub fn __name__(&self) -> PyResult<PyStrRef> {
    //     self.__name__.into()
    // }

    // #[pyproperty(setter)]
    // pub fn set___name__(&self, _name: PyStrRef) {
    //     self.__name__ = _name.to_string();
    // }

    // #[pyproperty]
    // pub fn argtypes(&self) -> Option<PyObjectRef> {
    //     // I think this needs to return a tuple reference to the objects that have CDataObject implementations
    //     // This kind off also wrong in the CPython's way, they allow classes with _as_parameter_ object attribute...
    //     convert_to_tuple(self._argtypes_)    
    // }

    // #[pyproperty(setter)]
    // pub fn set_argtypes(&self, argtypes: PyObjectRef, vm: &VirtualMachine) {
    //     if vm.isinstance(argtypes, PyListRef) || vm.isinstance(argtypes, PyTupleRef) {
    //         let args: Vec<PyRef<CDataObject>> = vm.extract_elements(argtypes).into_iter().filter(|obj|vm.isinstance(obj, CDataObjectRef)).collect();

    //         if args.len() > 0 {
    //             self._argtypes_ = args;
    //         } else {
    //             // Exception here
    //             // Err(vm.new_value_error(""))       
    //         }
    //     }

    //     // Exception here
    //     // Err(vm.new_type_error(""))
    // }

    // #[pyproperty]
    // pub fn restype(&self) -> Option<PyTupleRef> {
    //     convert_to_tuple(self._restype_)    
    // }

    // #[pyproperty(setter)]
    // pub fn set_restype(&self, restype: PyTupleRef) {
    //     self._restype_ = convert_from_tuple(restype)
    // }

    #[pymethod(name = "__call__")]
    pub fn call(&self) {
        if self.__name__.is_none() {
            // Exception here
            // Err(vm.new_value_error(""))
        }

        if self._argtypes_.len() == 0 {
            // Exception here
            // Err(vm.new_value_error(""))
        }

        if self._restype_.is_none() {
            // Exception here
            // Err(vm.new_value_error(""))
        }

        // Make the function call here
    }
}
use ::std::sync::Arc;
use libloading::Library;

use crate::builtins::list::PyListRef;
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::tuple::PyTupleRef;
use crate::builtins::PyTypeRef;

use crate::function::FuncArgs;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, StaticType, TypeProtocol};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{CDataObject, FunctionProxy};

use crate::slots::Callable;
use crate::stdlib::ctypes::dll::{dlsym, SharedLibrary};

extern crate libffi;

use libffi::middle::*;

use std::borrow::Borrow;

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCFuncPtr {
    _name_: Option<String>,
    _argtypes_: Vec<PyRef<CDataObject>>,
    _restype_: Option<PyRef<CDataObject>>,
    _callable_: Option<extern "C" fn()>,
    ext_func: Option<FunctionProxy>,
}

impl PyValue for PyCFuncPtr {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(with(Callable), flags(BASETYPE))]
impl PyCFuncPtr {
    #[inline]
    pub fn new() -> PyCFuncPtr {
        PyCFuncPtr {
            _name_: None,
            _argtypes_: Vec::new(),
            _restype_: None,
            _callable_: None,
            ext_func: None,
        }
    }

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

    // #[pymethod(name = "__call__")]
    // pub fn call(&self) {

    // }

    // @TODO: Needs to check and implement other forms of new
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        func_name: PyStrRef,
        dll: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        Self::from_dll(cls, func_name, dll, vm)
    }

    /// Returns a PyCFuncPtr from a Python DLL object
    /// # Arguments
    ///
    /// * `func_name` - A string that names the function symbol
    /// * `dll` - A Python object with _handle attribute of tpye SharedLibrary
    ///
    fn from_dll(
        cls: PyTypeRef,
        func_name: PyStrRef,
        dll: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        if let Ok(h) = vm.get_attribute(dll.clone(), "_handle") {
            if let Some(handle) = h.payload::<SharedLibrary>() {
                if let Ok(ext_func) = dlsym(handle, func_name) {
                    return PyCFuncPtr {
                        _name_: None,
                        _argtypes_: Vec::new(),
                        _restype_: None,
                        _callable_: Some(ext_func),
                        ext_func: None,
                    }
                    .into_ref_with_type(vm, cls);
                }
            }
        }

        PyCFuncPtr {
            _name_: None,
            _argtypes_: Vec::new(),
            _restype_: None,
            _callable_: None,
            ext_func: None,
        }
        .into_ref_with_type(vm, cls)
    }
}

impl Callable for PyCFuncPtr {
    // @TODO: Build args e result before calling.
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let c_args = vec![Type::pointer(), Type::c_int()];
        let cif = Cif::new(c_args.into_iter(), Type::c_int());

        unsafe {
            if let Some(ext_func) = zelf._callable_ {
                let n: *mut std::ffi::c_void = cif.call(
                    CodePtr(ext_func as *mut _),
                    &[arg(&String::from("Hello")), arg(&2)],
                );
            }
        }

        Ok(vm.ctx.none())
    }
}

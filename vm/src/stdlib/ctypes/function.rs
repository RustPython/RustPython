extern crate libffi;

use std::{mem, os::raw::c_void};

use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;

use crate::function::FuncArgs;
use crate::pyobject::{
    PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{CDataObject, Function, SharedLibrary, SIMPLE_TYPE_CHARS};

use crate::slots::Callable;
use crate::stdlib::ctypes::dll::dlsym;

fn map_types_to_res(args: &[PyObjectRc], vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
    args.iter()
        .enumerate()
        .map(|(idx, inner_obj)| {
            match vm.isinstance(inner_obj, CDataObject::static_type()) {
                Ok(_) => match vm.get_attribute(inner_obj.clone(), "_type_") {
                    Ok(_type_) if SIMPLE_TYPE_CHARS.contains(_type_.to_string().as_str()) => {
                        Ok(_type_)
                    }
                    Ok(_type_) => Err(vm.new_attribute_error("invalid _type_ value".to_string())),
                    Err(_) => Err(vm.new_attribute_error("atribute _type_ not found".to_string())),
                },
                // @TODO: Needs to return the name of the type, not String(inner_obj)
                Err(_) => Err(vm.new_type_error(format!(
                    "object at {} is not an instance of _CDataObject, type {} found",
                    idx,
                    inner_obj.to_string()
                ))),
            }
        })
        .collect()
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: PyRwLock<Vec<PyObjectRef>>,
    pub _restype_: PyRwLock<Box<PyObjectRef>>,
    _handle: PyObjectRc,
    _f: PyRwLock<Box<Function>>,
}

impl PyValue for PyCFuncPtr {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(with(Callable), flags(BASETYPE))]
impl PyCFuncPtr {
    #[pyproperty(name = "_argtypes_")]
    fn argtypes(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(self._argtypes_.read().clone())
    }

    #[pyproperty(name = "_restype_")]
    fn restype(&self, vm: &VirtualMachine) -> PyObjectRef {
        self._restype_.read().as_ref().clone()
    }

    #[pyproperty(name = "_argtypes_", setter)]
    fn set_argtypes(&self, argtypes: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.isinstance(&argtypes, &vm.ctx.types.list_type).is_ok()
            || vm.isinstance(&argtypes, &vm.ctx.types.tuple_type).is_ok()
        {
            let args: Vec<PyObjectRef> = vm.extract_elements(&argtypes).unwrap();

            let c_args = map_types_to_res(&args, vm)?;

            self._argtypes_.write().clear();
            self._argtypes_.write().extend(c_args.clone().into_iter());

            let mut f_guard = self._f.write();
            let fn_ptr = f_guard.as_mut();

            let str_types: Result<Vec<String>, _> = c_args
                .iter()
                .map(|obj| {
                    if let Ok(attr) = vm.get_attribute(obj.clone(), "_type_") {
                        Ok(attr.to_string())
                    } else {
                        Err(())
                    }
                })
                .collect();

            fn_ptr.set_args(str_types.unwrap());

            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "argtypes must be Tuple or List, {} found.",
                argtypes.to_string()
            )))
        }
    }

    #[pyproperty(name = "_restype_", setter)]
    fn set_restype(&self, restype: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match vm.isinstance(&restype, CDataObject::static_type()) {
            Ok(_) => match vm.get_attribute(restype.clone(), "_type_") {
                Ok(_type_)
                    if vm.isinstance(&_type_, &vm.ctx.types.str_type)?
                        && _type_.to_string().len() == 1
                        && SIMPLE_TYPE_CHARS.contains(_type_.to_string().as_str()) =>
                {
                    let mut r_guard = self._restype_.write();
                    mem::replace(r_guard.as_mut(), restype.clone());

                    let mut a_guard = self._f.write();
                    let fn_ptr = a_guard.as_mut();
                    fn_ptr.set_ret(_type_.to_string().as_str());

                    Ok(())
                }
                Ok(_type_) => Err(vm.new_attribute_error("invalid _type_ value".to_string())),
                Err(_) => Err(vm.new_attribute_error("atribute _type_ not found".to_string())),
            },
            // @TODO: Needs to return the name of the type, not String(inner_obj)
            Err(_) => Err(vm.new_type_error(format!(
                "value is not an instance of _CDataObject, type {} found",
                restype
            ))),
        }
    }

    // @TODO: Needs to check and implement other forms of new
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        func_name: PyStrRef,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        match vm.get_attribute(cls.as_object().to_owned(), "_argtypes_") {
            Ok(_) => Self::from_dll(cls, func_name, arg, vm),
            Err(_) => Err(vm.new_type_error(
                "cannot construct instance of this class: no argtypes".to_string(),
            )),
        }
    }

    /// Returns a PyCFuncPtr from a Python DLL object
    /// # Arguments
    ///
    /// * `func_name` - A string that names the function symbol
    /// * `dll` - A Python object with _handle attribute of type SharedLibrary
    ///
    fn from_dll(
        cls: PyTypeRef,
        func_name: PyStrRef,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        if let Ok(h) = vm.get_attribute(arg.clone(), "_handle") {
            if let Ok(handle) = h.downcast::<SharedLibrary>() {
                let handle_obj = handle.into_object();
                let ptr_fn = dlsym(handle_obj.clone(), func_name.clone().into_object(), vm)?;
                let fn_ptr = usize::try_from_object(vm, ptr_fn.into_object(vm))? as *mut c_void;

                PyCFuncPtr {
                    _name_: func_name.to_string(),
                    _argtypes_: PyRwLock::new(Vec::new()),
                    _restype_: PyRwLock::new(Box::new(vm.ctx.none())),
                    _handle: handle_obj.clone(),
                    _f: PyRwLock::new(Box::new(Function::new(
                        fn_ptr,
                        Vec::new(),
                        "P", // put a default here
                    ))),
                }
                .into_ref_with_type(vm, cls)
            } else {
                // @TODO: Needs to return the name of the type, not String(inner_obj)
                Err(vm.new_type_error(format!(
                    "_handle must be SharedLibrary not {}",
                    arg.to_string()
                )))
            }
        } else {
            Err(vm.new_attribute_error(
                "positional argument 2 must have _handle attribute".to_string(),
            ))
        }
    }
}

impl Callable for PyCFuncPtr {
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if args.args.len() != zelf._argtypes_.read().len() {
            return Err(vm.new_runtime_error(format!(
                "invalid number of arguments, required {}, but {} found",
                zelf._argtypes_.read().len(),
                args.args.len()
            )));
        }

        // Needs to check their types and convert to middle::Arg based on zelf._argtypes_
        let arg_vec = map_types_to_res(&args.args, vm)?;

        zelf._f.write().as_mut().call(arg_vec, vm)
    }
}

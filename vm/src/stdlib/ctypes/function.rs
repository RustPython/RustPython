extern crate libffi;

use std::{fmt, os::raw::c_void};

use crossbeam_utils::atomic::AtomicCell;

use crate::builtins::pystr::PyStrRef;
use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;

use crate::function::FuncArgs;
use crate::pyobject::{
    PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;
use crate::stdlib::ctypes::common::{Function, SharedLibrary};

use crate::slots::Callable;
use crate::stdlib::ctypes::dll::dlsym;

fn map_types_to_res(args: &[PyObjectRc], vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
    args.iter()
        .enumerate()
        .map(|(idx, inner_obj)| {
            match vm.isinstance(inner_obj, PyCData::static_type()) {
                // @TODO: checks related to _type_ are temporary
                Ok(_) => Ok(vm.get_attribute(inner_obj.clone(), "_type_").unwrap()),
                Err(_) => Err(vm.new_type_error(format!(
                    "object at {} is not an instance of _CData, type {} found",
                    idx,
                    inner_obj.class().name
                ))),
            }
        })
        .collect()
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: AtomicCell<Vec<PyObjectRef>>,
    pub _restype_: AtomicCell<PyObjectRef>,
    _handle: PyObjectRc,
    _f: PyRwLock<Function>,
}

impl fmt::Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCFuncPtr {{ _name_, _argtypes_, _restype_}}")
    }
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
        vm.ctx
            .new_list(unsafe { &*self._argtypes_.as_ptr() }.clone())
    }

    #[pyproperty(name = "_restype_")]
    fn restype(&self, _vm: &VirtualMachine) -> PyObjectRef {
        unsafe { &*self._restype_.as_ptr() }.clone()
    }

    #[pyproperty(name = "_argtypes_", setter)]
    fn set_argtypes(&self, argtypes: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.isinstance(&argtypes, &vm.ctx.types.list_type).is_ok()
            || vm.isinstance(&argtypes, &vm.ctx.types.tuple_type).is_ok()
        {
            let args: Vec<PyObjectRef> = vm.extract_elements(&argtypes).unwrap();

            let c_args = map_types_to_res(&args, vm)?;

            self._argtypes_.store(c_args.clone());

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

            let mut fn_ptr = self._f.write();
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
        match vm.isinstance(&restype, PyCData::static_type()) {
            // @TODO: checks related to _type_ are temporary
            Ok(_) => match vm.get_attribute(restype.clone(), "_type_") {
                Ok(_type_) => {
                    self._restype_.store(restype.clone());

                    let mut fn_ptr = self._f.write();
                    fn_ptr.set_ret(_type_.to_string().as_str());

                    Ok(())
                }
                Err(_) => Err(vm.new_attribute_error("atribute _type_ not found".to_string())),
            },

            Err(_) => Err(vm.new_type_error(format!(
                "value is not an instance of _CData, type {} found",
                restype.class().name
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
                "cannot construct instance of this class: no argtypes slot".to_string(),
            )),
        }
    }

    /// Returns a PyCFuncPtr from a Python DLL object
    /// # Arguments
    ///
    /// * `func_name` - A string that names the function symbol
    /// * `arg` - A Python object with _handle attribute of type SharedLibrary
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
                    _argtypes_: AtomicCell::default(),
                    _restype_: AtomicCell::new(vm.ctx.none()),
                    _handle: handle_obj.clone(),
                    _f: PyRwLock::new(Function::new(
                        fn_ptr,
                        Vec::new(),
                        "P", // put a default here
                    )),
                }
                .into_ref_with_type(vm, cls)
            } else {
                Err(vm.new_type_error(format!(
                    "_handle must be SharedLibrary not {}",
                    arg.class().name
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
        let inner_args = unsafe { &*zelf._argtypes_.as_ptr() };

        if args.args.len() != inner_args.len() {
            return Err(vm.new_runtime_error(format!(
                "invalid number of arguments, required {}, but {} found",
                inner_args.len(),
                args.args.len()
            )));
        }

        let arg_vec = map_types_to_res(&args.args, vm)?;

        (*zelf._f.write()).call(arg_vec, vm)
    }
}

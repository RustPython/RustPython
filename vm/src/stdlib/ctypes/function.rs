extern crate libffi;

use ::std::mem;

use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::PyTypeRef;
use crate::common::rc::PyRc;
use crate::function::FuncArgs;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, StaticType, TypeProtocol};
use crate::VirtualMachine;

use crate::stdlib::ctypes::common::{
    convert_type, lib_call, CDataObject, SharedLibrary, SIMPLE_TYPE_CHARS,
};

use crate::slots::Callable;
use crate::stdlib::ctypes::dll::dlsym;

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "CDataObject")]
#[derive(Debug)]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: Vec<PyObjectRef>,
    pub _restype_: Box<String>,
    _handle: PyRef<SharedLibrary>,
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
        vm.ctx.new_list(self._argtypes_.clone())
    }

    #[pyproperty(name = "_restype_")]
    fn restype(&self, vm: &VirtualMachine) -> PyObjectRef {
        PyStr::from(self._restype_.as_str()).into_object(vm)
    }

    #[pyproperty(name = "_argtypes_", setter)]
    fn set_argtypes(&self, argtypes: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if vm.isinstance(&argtypes, &vm.ctx.types.list_type).is_ok()
            || vm.isinstance(&argtypes, &vm.ctx.types.tuple_type).is_ok()
        {
            let args: Vec<PyObjectRef> = vm.extract_elements(&argtypes).unwrap();

            let c_args: Result<Vec<PyObjectRef>, _> = args
                .iter()
                .enumerate()
                .map(|(idx, inner_obj)| {
                    match vm.isinstance(inner_obj, CDataObject::static_type()) {
                        Ok(_) => match vm.get_attribute(inner_obj.clone(), "_type_") {
                            Ok(_type_)
                                if SIMPLE_TYPE_CHARS.contains(_type_.to_string().as_str()) =>
                            {
                                Ok(_type_)
                            }
                            Ok(_type_) => {
                                Err(vm.new_attribute_error("invalid _type_ value".to_string()))
                            }
                            Err(_) => {
                                Err(vm.new_attribute_error("atribute _type_ not found".to_string()))
                            }
                        },
                        Err(_) => Err(vm.new_type_error(format!(
                            "object at {} is not an instance of _CDataObject, type {} found",
                            idx,
                            inner_obj.class()
                        ))),
                    }
                })
                .collect();

            self._argtypes_.clear();
            self._argtypes_.extend(c_args?.into_iter());

            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "argtypes must be Tuple or List, {} found.",
                argtypes.class()
            )))
        }
    }

    #[pyproperty(name = "_restype_", setter)]
    fn set_restype(&self, restype: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match vm.isinstance(&restype, CDataObject::static_type()) {
            Ok(_) => match vm.get_attribute(restype, "_type_") {
                Ok(_type_)
                    if vm.isinstance(&_type_, &vm.ctx.types.str_type)?
                        && _type_.to_string().len() == 1
                        && SIMPLE_TYPE_CHARS.contains(_type_.to_string().as_str()) =>
                {
                    let old = self._restype_.as_mut();
                    let new = _type_.to_string();
                    mem::replace(old, new);
                    Ok(())
                }
                Ok(_type_) => Err(vm.new_attribute_error("invalid _type_ value".to_string())),
                Err(_) => Err(vm.new_attribute_error("atribute _type_ not found".to_string())),
            },
            Err(_) => Err(vm.new_type_error(format!(
                "value is not an instance of _CDataObject, type {} found",
                restype.class()
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
            if let Some(handle) = h.payload::<SharedLibrary>() {
                PyCFuncPtr {
                    _name_: func_name.to_string(),
                    _argtypes_: Vec::new(),
                    _restype_: Box::new("".to_string()),
                    _handle: handle.into_ref(vm),
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
    // @TODO: Build args e result before calling.
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if args.args.len() != zelf._argtypes_.len() {
            return Err(vm.new_runtime_error(format!(
                "invalid number of arguments, required {}, but {} found",
                zelf._argtypes_.len(),
                args.args.len()
            )));
        }

        // Needs to check their types and convert to middle::Arg based on zelf._argtypes_
        // Something similar to the set of _argtypes_
        // arg_vec = ...

        // This is not optimal, but I can't simply store a vector of middle::Type inside PyCFuncPtr
        let c_args = zelf
            ._argtypes_
            .iter()
            .map(|str_type| convert_type(str_type.to_string().as_str()))
            .collect();

        let arg_vec = Vec::new();

        let ret_type = convert_type(zelf._restype_.to_string().as_ref());

        let name_py_ref = PyStr::from(&zelf._name_).into_ref(vm);
        let ptr_fn = dlsym(zelf._handle.into_ref(vm), name_py_ref, vm).ok();
        let ret = lib_call(c_args, ret_type, arg_vec, ptr_fn, vm);

        Ok(vm.new_pyobj(ret))
    }
}

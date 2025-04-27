// cspell:disable

use crate::builtins::{PyNone, PyStr, PyTupleRef, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::Either;
use crate::function::FuncArgs;
use crate::stdlib::ctypes::PyCData;
use crate::stdlib::ctypes::array::PyCArray;
use crate::stdlib::ctypes::base::{PyCSimple, ffi_type_from_str};
use crate::types::Representable;
use crate::types::{Callable, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Symbol;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::ffi::{self, c_void};
use std::fmt::Debug;

// See also: https://github.com/python/cpython/blob/4f8bb3947cfbc20f970ff9d9531e1132a9e95396/Modules/_ctypes/callproc.c#L15

type FP = unsafe extern "C" fn();

pub trait ArgumentType {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> PyResult<Type>;
    fn convert_object(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<Arg>;
}

impl ArgumentType for PyTypeRef {
    fn to_ffi_type(&self, vm: &VirtualMachine) -> PyResult<Type> {
        let typ = self
            .get_class_attr(vm.ctx.intern_str("_type_"))
            .ok_or(vm.new_type_error("Unsupported argument type".to_string()))?;
        let typ = typ
            .downcast_ref::<PyStr>()
            .ok_or(vm.new_type_error("Unsupported argument type".to_string()))?;
        let typ = typ.to_string();
        let typ = typ.as_str();
        let converted_typ = ffi_type_from_str(typ);
        if let Some(typ) = converted_typ {
            Ok(typ)
        } else {
            Err(vm.new_type_error(format!("Unsupported argument type: {}", typ)))
        }
    }

    fn convert_object(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<Arg> {
        // if self.fast_isinstance::<PyCArray>(vm) {
        //     let array = value.downcast::<PyCArray>()?;
        //     return Ok(Arg::from(array.as_ptr()));
        // }
        if let Ok(simple) = value.downcast::<PyCSimple>() {
            let typ = ArgumentType::to_ffi_type(self, vm)?;
            let arg = simple.to_arg(typ, vm).ok_or(vm.new_type_error("Unsupported argument type".to_string()))?;
            return Ok(arg);
        }
        Err(vm.new_type_error("Unsupported argument type".to_string()))
    }
}

pub trait ReturnType {
    fn to_ffi_type(&self) -> Option<Type>;
    fn from_ffi_type(
        &self,
        value: *mut ffi::c_void,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>>;
}

impl ReturnType for PyTypeRef {
    fn to_ffi_type(&self) -> Option<Type> {
        ffi_type_from_str(self.name().to_string().as_str())
    }

    fn from_ffi_type(
        &self,
        _value: *mut ffi::c_void,
        _vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        todo!()
    }
}

impl ReturnType for PyNone {
    fn to_ffi_type(&self) -> Option<Type> {
        ffi_type_from_str("void")
    }

    fn from_ffi_type(
        &self,
        _value: *mut ffi::c_void,
        _vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        Ok(None)
    }
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
#[derive(PyPayload)]
pub struct PyCFuncPtr {
    pub ptr: PyRwLock<Option<CodePtr>>,
    pub needs_free: AtomicCell<bool>,
    pub arg_types: PyRwLock<Option<Vec<PyTypeRef>>>,
    pub res_type: PyRwLock<Option<PyObjectRef>>,
    pub _flags_: AtomicCell<i32>,
    pub handler: PyObjectRef,
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("flags", &self._flags_)
            .finish()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = (PyTupleRef, FuncArgs);

    fn py_new(_cls: PyTypeRef, (tuple, _args): Self::Args, vm: &VirtualMachine) -> PyResult {
        let name = tuple
            .first()
            .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements".to_string()))?
            .downcast_ref::<PyStr>()
            .ok_or(vm.new_type_error("Expected a string".to_string()))?
            .to_string();
        let handler = tuple
            .into_iter()
            .nth(1)
            .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements".to_string()))?
            .clone();
        let handle = handler.try_int(vm)?.as_bigint().clone();
        let library_cache = crate::stdlib::ctypes::library::libcache().read();
        let library = library_cache
            .get_lib(
                handle
                    .to_usize()
                    .ok_or(vm.new_value_error("Invalid handle".to_string()))?,
            )
            .ok_or_else(|| vm.new_value_error("Library not found".to_string()))?;
        let inner_lib = library.lib.lock();

        let terminated = format!("{}\0", &name);
        let code_ptr = if let Some(lib) = &*inner_lib {
            let pointer: Symbol<'_, FP> = unsafe {
                lib.get(terminated.as_bytes())
                    .map_err(|err| err.to_string())
                    .map_err(|err| vm.new_attribute_error(err))?
            };
            Some(CodePtr(*pointer as *mut _))
        } else {
            None
        };
        Ok(Self {
            ptr: PyRwLock::new(code_ptr),
            needs_free: AtomicCell::new(false),
            arg_types: PyRwLock::new(None),
            _flags_: AtomicCell::new(0),
            res_type: PyRwLock::new(None),
            handler,
        }
        .to_pyobject(vm))
    }
}

impl Callable for PyCFuncPtr {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // This is completely seperate from the C python implementation

        // Cif init
        let arg_types = zelf.arg_types.read();
        let ffi_arg_types = arg_types
            .as_ref()
            .ok_or_else(|| vm.new_type_error("argtypes not set".to_string()))?
            .iter()
            .map(|t| ArgumentType::to_ffi_type(&t))
            .collect::<Vec<_>>();
        let return_type = zelf.res_type.read();
        let ffi_return_type = return_type
            .as_ref()
            .map(|t| ReturnType::to_ffi_type(t))
            .unwrap_or_else(|| Type::i32());
        let cif = Cif::new(ffi_arg_types, ffi_return_type);

        // Call the function
        let ffi_args = args
            .into_iter()
            .map(|arg| {
                let arg_type = arg_types
                    .get(0)
                    .ok_or_else(|| vm.new_type_error("argtypes not set".to_string()))?;
                arg_type.convert_object(arg, vm)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pointer = zelf.ptr.read();
        let code_ptr = pointer
            .as_ref()
            .ok_or_else(|| vm.new_type_error("Function pointer not set".to_string()))?;
        let output: c_void = unsafe { cif.call(*code_ptr, &args) };
        let return_type = return_type
            .map(|f| f.from_ffi_type(output)?)
            .unwrap_or_else(|| vm.ctx.new_int(output as i32));
        if let Some(return_type) = return_type {
            Ok(return_type)
        } else {
            Ok(vm.get_none())
        }
    }
}

impl Representable for PyCFuncPtr {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let index = zelf.ptr.read();
        let index = index.map(|ptr| ptr.0 as usize).unwrap_or(0);
        let type_name = zelf.class().name();
        #[cfg(windows)]
        {
            let index = index - 0x1000;
            return Ok(format!("<COM method offset {index:#x} {type_name}>"));
        }
        Ok(format!("<{type_name} object at {index:#x}>"))
    }
}

// TODO: fix
unsafe impl Send for PyCFuncPtr {}
unsafe impl Sync for PyCFuncPtr {}

#[pyclass(flags(BASETYPE), with(Callable, Constructor, Representable))]
impl PyCFuncPtr {
    #[pygetset(name = "_restype_")]
    fn restype(&self) -> Option<PyObjectRef> {
        self.res_type.read().as_ref().cloned()
    }

    #[pygetset(name = "_restype_", setter)]
    fn set_restype(&self, restype: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // has to be type, callable, or none
        // TODO: Callable support
        if vm.is_none(&restype) || restype.isinstance::<PyTypeRef>(vm) {
            *self.res_type.write() = Some(restype);
            Ok(())
        } else {
            Err(vm.new_type_error("restype must be a type, a callable, or None".to_string()))
        }
    }

    #[pygetset(name = "argtypes")]
    fn argtypes(&self) -> Vec<PyTypeRef> {
        self.arg_types.read().clone()
    }

    #[pygetset(name = "argtypes", setter)]
    fn set_argtypes(
        &self,
        argtypes: Either<PyNone, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match argtypes {
            Either::A(_) => {
                *self.arg_types.write() = None;
                Ok(())
            }
            Either::B(tuple) => {
                *self.arg_types.write() = Some(
                    tuple
                        .iter()
                        .map(|obj| {
                            obj.downcast_ref::<PyType>().unwrap()
                        })
                        .collect::<Vec<_>>()
                );
                Ok(())
            }
        }
    }
}

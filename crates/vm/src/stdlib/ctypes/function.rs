// spell-checker:disable

use crate::builtins::{PyNone, PyStr, PyTuple, PyTupleRef, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::stdlib::ctypes::PyCData;
use crate::stdlib::ctypes::base::{PyCSimple, ffi_type_from_str};
use crate::stdlib::ctypes::thunk::PyCThunk;
use crate::types::Representable;
use crate::types::{Callable, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};
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
            let arg = simple
                .to_arg(typ, vm)
                .ok_or(vm.new_type_error("Unsupported argument type".to_string()))?;
            return Ok(arg);
        }
        Err(vm.new_type_error("Unsupported argument type".to_string()))
    }
}

pub trait ReturnType {
    fn to_ffi_type(&self) -> Option<Type>;
    #[allow(clippy::wrong_self_convention)]
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
        value: *mut ffi::c_void,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        // Get the type code from _type_ attribute
        let type_code = self
            .get_class_attr(vm.ctx.intern_str("_type_"))
            .and_then(|t| t.downcast_ref::<PyStr>().map(|s| s.to_string()));

        let result = match type_code.as_deref() {
            Some("b") => vm
                .ctx
                .new_int(unsafe { *(value as *const i8) } as i32)
                .into(),
            Some("B") => vm
                .ctx
                .new_int(unsafe { *(value as *const u8) } as i32)
                .into(),
            Some("c") => vm
                .ctx
                .new_bytes(vec![unsafe { *(value as *const u8) }])
                .into(),
            Some("h") => vm
                .ctx
                .new_int(unsafe { *(value as *const i16) } as i32)
                .into(),
            Some("H") => vm
                .ctx
                .new_int(unsafe { *(value as *const u16) } as i32)
                .into(),
            Some("i") => vm.ctx.new_int(unsafe { *(value as *const i32) }).into(),
            Some("I") => vm.ctx.new_int(unsafe { *(value as *const u32) }).into(),
            Some("l") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_long) })
                .into(),
            Some("L") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_ulong) })
                .into(),
            Some("q") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_longlong) })
                .into(),
            Some("Q") => vm
                .ctx
                .new_int(unsafe { *(value as *const libc::c_ulonglong) })
                .into(),
            Some("f") => vm
                .ctx
                .new_float(unsafe { *(value as *const f32) } as f64)
                .into(),
            Some("d") => vm.ctx.new_float(unsafe { *(value as *const f64) }).into(),
            Some("P") | Some("z") | Some("Z") => vm.ctx.new_int(value as usize).into(),
            Some("?") => vm
                .ctx
                .new_bool(unsafe { *(value as *const u8) } != 0)
                .into(),
            None => {
                // No _type_ attribute, try to create an instance of the type
                // This handles cases like Structure or Array return types
                return Ok(Some(
                    vm.ctx.new_int(unsafe { *(value as *const i32) }).into(),
                ));
            }
            _ => return Err(vm.new_type_error("Unsupported return type".to_string())),
        };
        Ok(Some(result))
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

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = PyCData)]
pub struct PyCFuncPtr {
    pub name: PyRwLock<Option<String>>,
    pub ptr: PyRwLock<Option<CodePtr>>,
    #[allow(dead_code)]
    pub needs_free: AtomicCell<bool>,
    pub arg_types: PyRwLock<Option<Vec<PyTypeRef>>>,
    pub res_type: PyRwLock<Option<PyObjectRef>>,
    pub _flags_: AtomicCell<i32>,
    #[allow(dead_code)]
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
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Handle different argument forms like CPython:
        // 1. Empty args: create uninitialized
        // 2. One integer argument: function address
        // 3. Tuple argument: (name, dll) form

        if args.args.is_empty() {
            return PyCFuncPtr {
                ptr: PyRwLock::new(None),
                needs_free: AtomicCell::new(false),
                arg_types: PyRwLock::new(None),
                _flags_: AtomicCell::new(0),
                res_type: PyRwLock::new(None),
                name: PyRwLock::new(None),
                handler: vm.ctx.none(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        let first_arg = &args.args[0];

        // Check if first argument is an integer (function address)
        if let Ok(addr) = first_arg.try_int(vm) {
            let ptr_val = addr.as_bigint().to_usize().unwrap_or(0);
            return PyCFuncPtr {
                ptr: PyRwLock::new(Some(CodePtr(ptr_val as *mut _))),
                needs_free: AtomicCell::new(false),
                arg_types: PyRwLock::new(None),
                _flags_: AtomicCell::new(0),
                res_type: PyRwLock::new(None),
                name: PyRwLock::new(Some(format!("CFuncPtr@{:#x}", ptr_val))),
                handler: vm.ctx.new_int(ptr_val).into(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        // Check if first argument is a tuple (name, dll) form
        if let Some(tuple) = first_arg.downcast_ref::<PyTuple>() {
            let name = tuple
                .first()
                .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements"))?
                .downcast_ref::<PyStr>()
                .ok_or(vm.new_type_error("Expected a string"))?
                .to_string();
            let handler = tuple
                .iter()
                .nth(1)
                .ok_or(vm.new_type_error("Expected a tuple with at least 2 elements"))?
                .clone();

            // Get library handle and load function
            let handle = handler.try_int(vm);
            let handle = match handle {
                Ok(handle) => handle.as_bigint().clone(),
                Err(_) => handler
                    .get_attr("_handle", vm)?
                    .try_int(vm)?
                    .as_bigint()
                    .clone(),
            };
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

            return PyCFuncPtr {
                ptr: PyRwLock::new(code_ptr),
                needs_free: AtomicCell::new(false),
                arg_types: PyRwLock::new(None),
                _flags_: AtomicCell::new(0),
                res_type: PyRwLock::new(None),
                name: PyRwLock::new(Some(name)),
                handler,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        // Check if first argument is a Python callable (callback creation)
        if first_arg.is_callable() {
            // Get argument types and result type from the class
            let argtypes = cls.get_attr(vm.ctx.intern_str("_argtypes_"));
            let restype = cls.get_attr(vm.ctx.intern_str("_restype_"));

            // Create the thunk (C-callable wrapper for the Python function)
            let thunk = PyCThunk::new(first_arg.clone(), argtypes.clone(), restype.clone(), vm)?;
            let code_ptr = thunk.code_ptr();

            // Parse argument types for storage
            let arg_type_vec: Option<Vec<PyTypeRef>> = if let Some(ref args) = argtypes {
                if vm.is_none(args) {
                    None
                } else {
                    let mut types = Vec::new();
                    for item in args.try_to_value::<Vec<PyObjectRef>>(vm)? {
                        types.push(item.downcast::<PyType>().map_err(|_| {
                            vm.new_type_error("_argtypes_ must be a sequence of types".to_string())
                        })?);
                    }
                    Some(types)
                }
            } else {
                None
            };

            // Store the thunk as a Python object to keep it alive
            let thunk_ref: PyRef<PyCThunk> = thunk.into_ref(&vm.ctx);

            return PyCFuncPtr {
                ptr: PyRwLock::new(Some(code_ptr)),
                needs_free: AtomicCell::new(true),
                arg_types: PyRwLock::new(arg_type_vec),
                _flags_: AtomicCell::new(0),
                res_type: PyRwLock::new(restype),
                name: PyRwLock::new(Some("<callback>".to_string())),
                handler: thunk_ref.into(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into);
        }

        Err(vm.new_type_error("Expected an integer address or a tuple"))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Callable for PyCFuncPtr {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // This is completely seperate from the C python implementation

        // Cif init
        let arg_types: Vec<_> = match zelf.arg_types.read().clone() {
            Some(tys) => tys,
            None => args
                .args
                .clone()
                .into_iter()
                .map(|a| a.class().as_object().to_pyobject(vm).downcast().unwrap())
                .collect(),
        };
        let ffi_arg_types = arg_types
            .clone()
            .iter()
            .map(|t| ArgumentType::to_ffi_type(t, vm))
            .collect::<PyResult<Vec<_>>>()?;
        let return_type = zelf.res_type.read();
        let ffi_return_type = return_type
            .as_ref()
            .and_then(|t| t.clone().downcast::<PyType>().ok())
            .and_then(|t| ReturnType::to_ffi_type(&t))
            .unwrap_or_else(Type::i32);
        let cif = Cif::new(ffi_arg_types, ffi_return_type);

        // Call the function
        let ffi_args = args
            .args
            .into_iter()
            .enumerate()
            .map(|(n, arg)| {
                let arg_type = arg_types
                    .get(n)
                    .ok_or_else(|| vm.new_type_error("argument amount mismatch".to_string()))?;
                arg_type.convert_object(arg, vm)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pointer = zelf.ptr.read();
        let code_ptr = pointer
            .as_ref()
            .ok_or_else(|| vm.new_type_error("Function pointer not set".to_string()))?;
        let mut output: c_void = unsafe { cif.call(*code_ptr, &ffi_args) };
        let return_type = return_type
            .as_ref()
            .and_then(|f| f.clone().downcast::<PyType>().ok())
            .map(|f| f.from_ffi_type(&mut output, vm).ok().flatten())
            .unwrap_or_else(|| Some(vm.ctx.new_int(output as i32).as_object().to_pyobject(vm)));
        if let Some(return_type) = return_type {
            Ok(return_type)
        } else {
            Ok(vm.ctx.none())
        }
    }
}

impl Representable for PyCFuncPtr {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let index = zelf.ptr.read();
        let index = index.map(|ptr| ptr.0 as usize).unwrap_or(0);
        let type_name = zelf.class().name();
        if cfg!(windows) {
            let index = index - 0x1000;
            Ok(format!("<COM method offset {index:#x} {type_name}>"))
        } else {
            Ok(format!("<{type_name} object at {index:#x}>"))
        }
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
        if vm.is_none(&restype) || restype.downcast_ref::<PyType>().is_some() {
            *self.res_type.write() = Some(restype);
            Ok(())
        } else {
            Err(vm.new_type_error("restype must be a type, a callable, or None".to_string()))
        }
    }

    #[pygetset(name = "argtypes")]
    fn argtypes(&self, vm: &VirtualMachine) -> PyTupleRef {
        PyTuple::new_ref(
            self.arg_types
                .read()
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.to_pyobject(vm))
                .collect(),
            &vm.ctx,
        )
    }

    #[pygetset(name = "argtypes", setter)]
    fn set_argtypes(&self, argtypes: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let none = vm.is_none(&argtypes);
        if none {
            *self.arg_types.write() = None;
            Ok(())
        } else {
            let tuple = argtypes.downcast::<PyTuple>().unwrap();
            *self.arg_types.write() = Some(
                tuple
                    .iter()
                    .map(|obj| obj.clone().downcast::<PyType>().unwrap())
                    .collect::<Vec<_>>(),
            );
            Ok(())
        }
    }

    #[pygetset]
    fn __name__(&self) -> Option<String> {
        self.name.read().clone()
    }

    #[pygetset(setter)]
    fn set___name__(&self, name: String) -> PyResult<()> {
        *self.name.write() = Some(name);
        // TODO: update handle and stuff
        Ok(())
    }
}

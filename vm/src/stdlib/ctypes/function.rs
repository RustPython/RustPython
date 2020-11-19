extern crate libffi;

use std::{fmt, os::raw::*, ptr};

use crossbeam_utils::atomic::AtomicCell;

use libffi::low::{
    call as ffi_call, ffi_abi_FFI_DEFAULT_ABI as ABI, ffi_cif, ffi_type, prep_cif, CodePtr,
    Error as FFIError,
};
use libffi::middle;
use num_bigint::BigInt;

use crate::builtins::pystr::PyStrRef;
use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;

use crate::function::FuncArgs;
use crate::pyobject::{
    PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::PyCData;
use crate::stdlib::ctypes::common::SharedLibrary;

use crate::slots::Callable;
use crate::stdlib::ctypes::dll::dlsym;

macro_rules! ffi_type {
    ($name: ident) => {
        middle::Type::$name().as_raw_ptr()
    };
}

macro_rules! match_ffi_type {
    (
        $pointer: expr,

        $(
            $($type: ident)|+ => $body: expr
        )+
    ) => {
        match $pointer {
            $(
                $(
                    t if t == ffi_type!($type) => { $body }
                )+
            )+
            _ => unreachable!()
        }
    };
    (
        $kind: expr,

        $(
            $($type: tt)|+ => $body: ident
        )+
    ) => {
        match $kind {
            $(
                $(
                    t if t == $type => { ffi_type!($body) }
                )+
            )+
            _ => unreachable!()
        }
    }
}

pub fn str_to_type(ty: &str) -> *mut ffi_type {
    match_ffi_type!(
        ty,
        "c" => c_schar
        "u" => c_int
        "b" => i8
        "h" => c_short
        "H" => c_ushort
        "i" => c_int
        "I" => c_uint
        "l" => c_long
        "q" => c_longlong
        "L" => c_ulong
        "Q" => c_ulonglong
        "f" => f32
        "d" => f64
        "g" => longdouble
        "?" | "B" => c_uchar
        "z" | "Z" => pointer
        "P" => void
    )
}

fn py_to_ffi(ty: *mut *mut ffi_type, obj: PyObjectRef, vm: &VirtualMachine) -> *mut c_void {
    match_ffi_type!(
        unsafe { *ty },
        c_schar => {
            let mut r = i8::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_int => {
            let mut r = i32::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_short => {
            let mut r = i16::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_ushort => {
            let mut r = u16::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_uint => {
            let mut r = u32::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        //@ TODO: Convert c*longlong from BigInt?
        c_long | c_longlong => {
            let mut r = i64::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_ulong | c_ulonglong => {
            let mut r = u64::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        f32 => {
            let mut r = f32::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        f64 | longdouble=> {
            let mut r = f64::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        c_uchar => {
            let mut r = u8::try_from_object(vm, obj).unwrap();
            &mut r as *mut _ as *mut c_void
        }
        pointer => {
            usize::try_from_object(vm, obj).unwrap() as *mut c_void
        }
        void => {
            ptr::null_mut()
        }
    )
}

#[derive(Debug)]
pub struct Function {
    pointer: *const c_void,
    cif: ffi_cif,
    arguments: Vec<*mut ffi_type>,
    return_type: Box<*mut ffi_type>,
}

impl Function {
    pub fn new(fn_ptr: *const c_void, arguments: Vec<String>, return_type: &str) -> Function {
        Function {
            pointer: fn_ptr,
            cif: Default::default(),
            arguments: arguments.iter().map(|s| str_to_type(s.as_str())).collect(),

            return_type: Box::new(str_to_type(return_type)),
        }
    }
    pub fn set_args(&mut self, args: Vec<String>) {
        self.arguments.clear();
        self.arguments
            .extend(args.iter().map(|s| str_to_type(s.as_str())));
    }

    pub fn set_ret(&mut self, ret: &str) {
        (*self.return_type.as_mut()) = str_to_type(ret);
        // mem::replace(self.return_type.as_mut(), str_to_type(ret));
    }

    pub fn call(
        &mut self,
        arg_ptrs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRc> {
        let return_type: *mut ffi_type = &mut unsafe { self.return_type.read() };

        let result = unsafe {
            prep_cif(
                &mut self.cif,
                ABI,
                self.arguments.len(),
                return_type,
                self.arguments.as_mut_ptr(),
            )
        };

        if let Err(FFIError::Typedef) = result {
            return Err(vm.new_runtime_error(
                "The type representation is invalid or unsupported".to_string(),
            ));
        } else if let Err(FFIError::Abi) = result {
            return Err(vm.new_runtime_error("The ABI is invalid or unsupported".to_string()));
        }

        let mut argument_pointers: Vec<*mut c_void> = arg_ptrs
            .iter()
            .zip(self.arguments.iter_mut())
            .map(|(o, t)| {
                let tt: *mut *mut ffi_type = t;
                py_to_ffi(tt, o.clone(), vm)
            })
            .collect();

        let cif_ptr = &self.cif as *const _ as *mut _;
        let fun_ptr = CodePtr::from_ptr(self.pointer);
        let args_ptr = argument_pointers.as_mut_ptr();

        let ret_ptr = unsafe {
            match_ffi_type!(
                return_type,
                c_schar => {
                    let r: c_schar = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as i8)
                }
                c_int => {
                    let r: c_int = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as i32)
                }
                c_short => {
                    let r: c_short = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as i16)
                }
                c_ushort => {
                    let r: c_ushort = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u16)
                }
                c_uint => {
                    let r: c_uint = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u32)
                }
                c_long => {
                    let r: c_long = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as i64)
                }
                c_longlong => {
                    let r: c_longlong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(BigInt::from(r as i128))
                }
                c_ulong => {
                    let r: c_ulong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u64)
                }
                c_ulonglong => {
                    let r: c_ulonglong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(BigInt::from(r as u128))
                }
                f32 => {
                    let r: c_float = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as f32)
                }
                f64 | longdouble => {
                    let r: c_double = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as f64)
                }
                c_uchar => {
                    let r: c_uchar = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u8)
                }
                pointer => {
                    let r: *mut c_void = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as *const _ as usize)
                }
                void => {
                    vm.ctx.none()
                }
            )
        };

        Ok(ret_ptr)
    }
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

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

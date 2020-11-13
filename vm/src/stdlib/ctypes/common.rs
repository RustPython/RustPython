extern crate lazy_static;
extern crate libffi;
extern crate libloading;

use ::std::{collections::HashMap, mem, os::raw::*};

use libffi::low::{
    call as ffi_call, ffi_abi_FFI_DEFAULT_ABI as ABI, ffi_cif, ffi_type, prep_cif, CodePtr,
    Error as FFIError,
};
use libffi::middle;
use libloading::Library;

use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;
use crate::pyobject::{PyObjectRc, PyObjectRef, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;

pub const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfuzZqQP?g";

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
            _ => ffi_type!(void)
        }
    }
}

fn str_to_type(ty: &str) -> *mut ffi_type {
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

#[derive(Debug)]
pub struct Function {
    pointer: *const c_void,
    cif: ffi_cif,
    arguments: Vec<*mut ffi_type>,
    return_type: Box<*mut ffi_type>,
    // @TODO: Do we need to free the memory of these ffi_type?
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
        mem::replace(self.return_type.as_mut(), str_to_type(ret));
    }

    pub fn call(
        &mut self,
        arg_ptrs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRc> {
        let mut return_type: *mut ffi_type = &mut unsafe { self.return_type.read() };

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

        let cif_ptr = &self.cif as *const _ as *mut _;
        let fun_ptr = CodePtr::from_ptr(self.pointer);
        let mut args_ptr = self
            .arguments
            .iter_mut()
            .map(|p: &mut *mut ffi_type| p as *mut _ as *mut c_void)
            .collect()
            .as_mut_ptr();

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
                    vm.new_pyobj(r as u64)
                }
                c_longlong => {
                    let r: c_longlong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as i64)
                    // vm.new_pyobj(r as i128)
                }
                c_ulong => {
                    let r: c_ulong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u64)
                }
                c_ulonglong => {
                    let r: c_ulonglong = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as u64)
                    // vm.new_pyobj(r as u128)
                }
                f32 => {
                    let r: c_float = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as f32)
                }
                f64 => {
                    let r: c_double = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.new_pyobj(r as f64)
                }
                longdouble => {
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
                    let r: c_void = ffi_call(cif_ptr, fun_ptr, args_ptr);
                    vm.ctx.none()
                }
            )
        };

        Ok(ret_ptr)
    }
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

#[pyclass(module = false, name = "SharedLibrary")]
#[derive(Debug)]
pub struct SharedLibrary {
    path_name: String,
    lib: Library,
    is_open_g: Box<bool>,
}

impl PyValue for SharedLibrary {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl SharedLibrary {
    pub fn new(name: &str) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            path_name: name.to_string(),
            lib: Library::new(name.to_string())?,
            is_open_g: Box::new(true),
        })
    }

    pub fn get_sym(&self, name: &str) -> Result<*mut c_void, libloading::Error> {
        unsafe {
            self.lib
                .get(name.as_bytes())
                .map(|f: libloading::Symbol<*mut c_void>| *f)
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open_g.as_ref().clone()
    }

    pub fn close(&self) -> Result<(), libloading::Error> {
        if let Err(e) = self.lib.close() {
            Err(e)
        } else {
            mem::replace(self.is_open_g.as_mut(), false);
            Ok(())
        }
    }
}

pub struct ExternalLibs {
    pub libraries: HashMap<String, PyRef<SharedLibrary>>,
}

impl ExternalLibs {
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    pub fn get_or_insert_lib(
        &mut self,
        library_path: &str,
        vm: &VirtualMachine,
    ) -> Result<&PyRef<SharedLibrary>, libloading::Error> {
        let library = self
            .libraries
            .entry(library_path.to_string())
            .or_insert(SharedLibrary::new(library_path)?.into_ref(vm));

        if !library.is_open() {
            if let Some(l) = self.libraries.insert(
                library_path.to_string(),
                SharedLibrary::new(library_path)?.into_ref(vm),
            ) {
                // Ok(self.libraries.get_mut(library_path.to_string()))
                Ok(&l)
            } else {
                // @TODO: What this error should be?
                Err(libloading::Error::DlOpenUnknown)
            }
        } else {
            Ok(library)
        }
    }
}

#[pyclass(module = false, name = "_CDataObject")]
#[derive(Debug)]
pub struct CDataObject {}

impl PyValue for CDataObject {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_metaclass()
    }
}

#[pyimpl(flags(BASETYPE))]
impl CDataObject {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here
}

lazy_static::lazy_static! {
    pub static ref CDATACACHE: PyRwLock<ExternalLibs> = PyRwLock::new(ExternalLibs::new());
}

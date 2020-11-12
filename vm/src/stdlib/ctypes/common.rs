extern crate lazy_static;
extern crate libffi;
extern crate libloading;

use ::std::{collections::HashMap, mem, os::raw::*};

use libffi::low::{
    call as ffi_call, ffi_abi_FFI_DEFAULT_ABI as ABI, ffi_cif, ffi_type, prep_cif, types, CodePtr,
    Error as FFIError,
};
use libffi::middle;

use libloading::Library;

use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;
use crate::pyobject::{PyObjectRc, PyObjectRef, PyRef, PyValue, StaticType};
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

fn ffi_to_rust(ty: *mut ffi_type) -> NativeType {
    match_ffi_type!(
        ty,
        c_schar => NativeType::Byte(ty as i8)
        c_int => NativeType::Int(ty as i32)
        c_short => NativeType::Short(ty as i16)
        c_ushort => NativeType::UShort(ty as u16)
        c_uint => NativeType::UInt(ty as u32)
        c_long => NativeType::Long(ty as i64)
        c_longlong => NativeType::LongLong(ty as i128)
        c_ulong => NativeType::ULong(ty as u64)
        c_ulonglong => NativeType::ULL(ty as u128)
        f32 => NativeType::Float(unsafe{*(ty as *mut f32)})
        f64 => NativeType::Double(unsafe {*(ty as *mut f64)})
        longdouble => NativeType::LongDouble(unsafe {*(ty as *mut f64)})
        c_uchar => NativeType::UByte(ty as u8)
        pointer => NativeType::Pointer(ty as *mut c_void)
        void => NativeType::Void
    )
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

enum NativeType {
    Byte(i8),
    Short(i16),
    UShort(u16),
    Int(i32),
    UInt(u32),
    Long(i64),
    LongLong(i128),
    ULong(u64),
    ULL(u128),
    Float(f32),
    Double(f64),
    LongDouble(f64),
    UByte(u8),
    Pointer(*mut c_void),
    Void,
}

#[derive(Debug)]
pub struct Function {
    pointer: mem::MaybeUninit<c_void>,
    cif: ffi_cif,
    arguments: Vec<*mut ffi_type>,
    return_type: *mut ffi_type,
}

impl Function {
    pub fn new(
        fn_ptr: mem::MaybeUninit<c_void>,
        arguments: Vec<String>,
        return_type: &str,
    ) -> Function {
        Function {
            pointer: fn_ptr,
            cif: Default::default(),
            arguments: arguments.iter().map(|s| str_to_type(s.as_str())).collect(),

            return_type: str_to_type(return_type),
        }
    }
    pub fn set_args(&mut self, args: Vec<String>) {
        self.arguments = args.iter().map(|s| str_to_type(s.as_str())).collect();
    }

    pub fn set_ret(&mut self, ret: &str) {
        self.return_type = str_to_type(ret);
    }

    pub fn call(
        &self,
        arg_ptrs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> Result<PyObjectRc, FFIError> {
    }
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

#[pyclass(module = false, name = "SharedLibrary")]
#[derive(Debug)]
pub struct SharedLibrary {
    path_name: String,
    lib: Library,
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
        })
    }

    pub fn get_sym(&self, name: &str) -> Result<*mut c_void, libloading::Error> {
        unsafe {
            self.lib
                .get(name.as_bytes())
                .map(|f: libloading::Symbol<*mut c_void>| *f)
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

    pub fn get_or_insert_lib<'a, 'b>(
        &'b mut self,
        library_path: &'a str,
        vm: &'a VirtualMachine,
    ) -> Result<&PyRef<SharedLibrary>, libloading::Error> {
        let library = self
            .libraries
            .entry(library_path.to_string())
            .or_insert(SharedLibrary::new(library_path)?.into_ref(vm));

        Ok(library)
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

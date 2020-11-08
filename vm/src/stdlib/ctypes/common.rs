extern crate lazy_static;
extern crate libffi;
extern crate libloading;

use ::std::collections::HashMap;

use libffi::middle;
use libloading::Library;

use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;
use crate::common::rc::PyRc;
use crate::pyobject::{PyObjectRc, PyValue, StaticType};
use crate::VirtualMachine;

pub const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfuzZqQP?g";

pub fn convert_type(ty: &str) -> middle::Type {
    match ty {
        "c" => middle::Type::c_schar(),
        "u" => middle::Type::c_int(),
        "b" => middle::Type::i8(),
        "h" => middle::Type::c_ushort(),
        "H" => middle::Type::u16(),
        "i" => middle::Type::c_int(),
        "I" => middle::Type::c_uint(),
        "l" => middle::Type::c_long(),
        "q" => middle::Type::c_longlong(),
        "L" => middle::Type::c_ulong(),
        "Q" => middle::Type::c_ulonglong(),
        "f" => middle::Type::f32(),
        "d" => middle::Type::f64(),
        "g" => middle::Type::longdouble(),
        "?" | "B" => middle::Type::c_uchar(),
        "z" | "Z" => middle::Type::pointer(),
        "P" | _ => middle::Type::void(),
    }
}

pub fn lib_call(
    c_args: Vec<middle::Type>,
    restype: middle::Type,
    arg_vec: Vec<middle::Arg>,
    ptr_fn: Option<*const i32>,
    _vm: &VirtualMachine,
) {
    let cif = middle::Cif::new(c_args.into_iter(), restype);

    if ptr_fn.is_some() {
        // Here it needs a type to return
        unsafe {
            cif.call(
                middle::CodePtr::from_ptr(ptr_fn.unwrap() as *const _ as *const libc::c_void),
                arg_vec.as_slice(),
            )
        }
    }
}
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

#[pyimpl(flags(BASETYPE))]
impl SharedLibrary {
    pub fn new(name: &str) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            path_name: name.to_string(),
            lib: Library::new(name.to_string())?,
        })
    }

    pub fn get_sym(&self, name: &str) -> Result<*const i32, libloading::Error> {
        unsafe { self.lib.get(name.as_bytes()).map(|f| *f) }
    }
}

pub struct ExternalFunctions {
    libraries: HashMap<String, PyRc<SharedLibrary>>,
}

impl ExternalFunctions {
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    pub unsafe fn get_or_insert_lib<'a, 'b>(
        &'b mut self,
        library_path: &'a str,
        vm: &'a VirtualMachine,
    ) -> Result<PyObjectRc, libloading::Error> {
        let library = self
            .libraries
            .entry(library_path.to_string())
            .or_insert(PyRc::new(SharedLibrary::new(library_path)?));

        Ok(library.clone().into_object(vm))
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
    pub static ref CDATACACHE: PyRwLock<ExternalFunctions> = PyRwLock::new(ExternalFunctions::new());
}

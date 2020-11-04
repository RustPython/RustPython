extern crate lazy_static;
extern crate libffi;
extern crate libloading;

use ::std::{collections::HashMap, sync::Arc};
use libffi::{low, middle};
use libloading::Library;

use crate::builtins::pystr::PyStrRef;
use crate::builtins::PyTypeRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;

pub const SIMPLE_TYPE_CHARS: &'static str = "cbBhHiIlLdfuzZqQP?g";

pub fn convert_type(ty: &str) -> middle::Type {
    match ty {
        "?" => middle::Type::c_uchar(),
        "c" => middle::Type::c_schar(),
        "u" => middle::Type::c_int(),
        "b" => middle::Type::i8(),
        "B" => middle::Type::c_uchar(),
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
        "z" => middle::Type::pointer(),
        "Z" => middle::Type::pointer(),
        "P" => middle::Type::void(),
    }
}

pub struct ExternalFunctions {
    functions: HashMap<String, Arc<FunctionProxy>>,
    libraries: HashMap<String, Arc<Library>>,
}

impl ExternalFunctions {
    pub unsafe fn get_or_insert_lib(
        &mut self,
        library_path: String,
    ) -> Result<&mut Arc<Library>, libloading::Error> {
        let library = self
            .libraries
            .entry(library_path)
            .or_insert(Arc::new(Library::new(library_path)?));

        Ok(library)
    }

    pub fn get_or_insert_fn(
        &mut self,
        func_name: String,
        library_path: String,
        library: Arc<Library>,
        vm: &VirtualMachine,
    ) -> PyResult<Arc<FunctionProxy>> {
        let f_name = format!("{}_{}", library_path, func_name);

        Ok(self
            .functions
            .entry(f_name)
            .or_insert(Arc::new(FunctionProxy {
                _name: f_name,
                _lib: library,
            }))
            .clone())
    }
}

lazy_static::lazy_static! {
    pub static ref FUNCTIONS: ExternalFunctions = ExternalFunctions {
        functions:HashMap::new(),
        libraries:HashMap::new()
    };
}

#[derive(Debug, Clone)]
pub struct FunctionProxy {
    _name: String,
    _lib: Arc<Library>,
}

impl FunctionProxy {
    #[inline]
    pub fn get_name(&self) -> String {
        return self._name;
    }

    #[inline]
    pub fn get_lib(&self) -> &Library {
        self._lib.as_ref()
    }

    pub fn call(
        &self,
        c_args: Vec<middle::Type>,
        restype: Option<PyStrRef>,
        arg_vec: Vec<middle::Arg>,
        ptr_fn: Option<*const i32>,
        vm: &VirtualMachine,
    ) {
        let cas_ret = restype
            .and_then(|r| Some(r.to_string().as_str()))
            .unwrap_or("P");

        let cif = middle::Cif::new(c_args.into_iter(), convert_type(cas_ret));

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
}

impl PyValue for FunctionProxy {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

#[pyclass(module = false, name = "_CDataObject")]
#[derive(Debug)]
pub struct CDataObject {}

impl PyValue for CDataObject {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::init_bare_type()
    }
}

#[pyimpl(flags(BASETYPE))]
impl CDataObject {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here
}

extern crate lazy_static;
extern crate libloading;

use ::std::{collections::HashMap, sync::Arc};
use libloading::Library;

use crate::builtins::PyTypeRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue, StaticType};
use crate::VirtualMachine;
#[derive(Copy, Clone)]
pub enum Value {
    Bool,
    Char,
    Wchar,
    Byte,
    Ubyte,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    LongLong,
    ULong,
    ULL,
    SizeT,
    SsizeT,
    Float,
    Double,
    // LongDoudle(...) ???,
    CharP,
    WcharP,
    Void,
}

pub struct ExternalFunctions {
    functions: HashMap<String, Arc<FunctionProxy>>,
    libraries: HashMap<String, Arc<Library>>,
}

impl ExternalFunctions {
    pub fn call_function(
        &self,
        function: &str,
        arguments: Vec<CDataObject>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        match self.functions.get(function) {
            Some(func_proxy) => func_proxy.call(arguments),
            _ => Err(vm.new_runtime_error(format!("Function {} not found", function))),
        }
    }

    pub unsafe fn get_or_insert_lib(
        &mut self,
        library_path: String,
    ) -> Result<&Library, libloading::Error> {
        let library = self
            .libraries
            .entry(library_path)
            .or_insert({ Arc::new(Library::new(library_path)?) });

        Ok(library.as_ref())
    }

    pub fn get_or_insert_fn(
        &mut self,
        func_name: String,
        library_path: String,
        vm: &VirtualMachine,
    ) -> PyResult<&mut Arc<FunctionProxy>> {
        let f_name = format!("{}_{}", library_path, func_name);

        match self.libraries.get(&library_path) {
            Some(library) => Ok(self.functions.entry(f_name).or_insert({
                Arc::new(FunctionProxy {
                    _name: f_name,
                    _lib: *library,
                })
            })),
            _ => Err(vm.new_runtime_error(format!("Library of path {} not found", library_path))),
        }
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
    // fn call(&self, args: Vec<CDataObject>) -> PyResult<PyObjectRef> {

    // }
}

impl PyValue for FunctionProxy {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

#[pyclass(module = false, name = "_CDataObject")]
#[derive(Debug)]
pub struct CDataObject;

pub type CDataObjectRef = PyRef<CDataObject>;

impl PyValue for CDataObject {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(flags(BASETYPE))]
impl CDataObject {
    // A lot of the logic goes in this trait
    // There's also other traits that should have different implementations for some functions
    // present here
}

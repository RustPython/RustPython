use crate::builtins::PyTypeRef;
use crate::stdlib::ctypes::PyCData;
use crate::types::{Callable, Constructor};
use crate::{Py, PyObjectRef, PyResult, VirtualMachine};
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::lock::{PyMutex, PyRwLock};
use std::ffi::c_void;
use std::fmt::Debug;
use std::sync::Arc;

// https://github.com/python/cpython/blob/4f8bb3947cfbc20f970ff9d9531e1132a9e95396/Modules/_ctypes/callproc.c#L15

#[derive(Debug)]
pub enum FunctionArgument {
    Float(std::ffi::c_float),
    Double(std::ffi::c_double),
    // TODO: Duplicate char stuff
    UChar(std::ffi::c_uchar),
    SChar(std::ffi::c_schar),
    Char(std::ffi::c_char),
    UShort(std::ffi::c_ushort),
    Short(std::ffi::c_short),
    UInt(std::ffi::c_uint),
    Int(std::ffi::c_int),
    ULong(std::ffi::c_ulong),
    Long(std::ffi::c_long),
    ULongLong(std::ffi::c_ulonglong),
    LongLong(std::ffi::c_longlong),
}

#[derive(Debug)]
pub struct Function {
    // TODO: no protection from use-after-free
    pointer: Arc<PyMutex<*mut c_void>>,
    arguments: Vec<FunctionArgument>,
    return_type: PyTypeRef,
}

unsafe impl Send for Function {}
unsafe impl Sync for Function {}

impl Function {
    pub unsafe fn load(
        library: &libloading::Library,
        function: &str,
        args: Vec<PyObjectRef>,
        return_type: PyTypeRef,
    ) -> PyResult<Self> {
        let terminated = format!("{}\0", function);
        let pointer = library
            .get(terminated.as_bytes())
            .map_err(|err| err.to_string())
            .unwrap();
        Ok(Function {
            pointer: Arc::new(PyMutex::new(*pointer)),
            arguments: args
                .iter()
                .map(|arg| todo!("convert PyObjectRef to FunctionArgument"))
                .collect(),
            return_type,
        })
    }

    pub unsafe fn call(&self, vm: &VirtualMachine) -> PyObjectRef {
        // assemble function type signature
        let pointer = self.pointer.lock();
        let f: extern "C" fn() = std::mem::transmute(*pointer);
        f();
        vm.ctx.none()
    }
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
#[derive(PyPayload)]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: AtomicCell<Vec<PyObjectRef>>,
    pub _restype_: AtomicCell<PyObjectRef>,
    _handle: PyObjectRef,
    _f: PyRwLock<Function>,
}

impl Debug for PyCFuncPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCFuncPtr")
            .field("_name_", &self._name_)
            .finish()
    }
}

impl Constructor for PyCFuncPtr {
    type Args = ();

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        todo!("PyCFuncPtr::py_new")
    }
}

impl Callable for PyCFuncPtr {
    type Args = Vec<PyObjectRef>;

    fn call(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        todo!()
    }
}

#[pyclass(flags(BASETYPE), with(Callable, Constructor))]
impl PyCFuncPtr {}

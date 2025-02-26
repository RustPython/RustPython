use crate::PyObjectRef;
use crate::stdlib::ctypes::PyCData;
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::lock::PyRwLock;
use std::ffi::c_void;

#[derive(Debug)]
pub struct Function {
    _pointer: *mut c_void,
    _arguments: Vec<()>,
    _return_type: Box<()>,
}

#[pyclass(module = "_ctypes", name = "CFuncPtr", base = "PyCData")]
pub struct PyCFuncPtr {
    pub _name_: String,
    pub _argtypes_: AtomicCell<Vec<PyObjectRef>>,
    pub _restype_: AtomicCell<PyObjectRef>,
    _handle: PyObjectRef,
    _f: PyRwLock<Function>,
}

#[pyclass]
impl PyCFuncPtr {}

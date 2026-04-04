use core::ffi::c_long;
pub use rustpython_vm::{PyObject};

extern crate alloc;

pub mod bytesobject;
pub mod import;
pub mod longobject;
pub mod object;
pub mod pyerrors;
pub mod pylifecycle;
pub mod pystate;
pub mod refcount;
pub mod traceback;
pub mod tupleobject;
pub mod unicodeobject;


#[repr(C)]
pub struct PyThreadState {
    _private: [u8; 0],
}

#[repr(C)]
pub struct PyLongObject {
    ob_base: PyObject,
    value: c_long,
}

#[inline]
pub(crate) fn log_stub(name: &str) {
    eprintln!("[rustpython-capi stub] {name} called");
}

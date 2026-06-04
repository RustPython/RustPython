use crate::PyObject;

#[repr(C)]
pub struct PyCriticalSection;

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection_Begin(_c: *mut PyCriticalSection, _op: *mut PyObject) {}

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection_End(_c: *mut PyCriticalSection) {}

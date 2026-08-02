use crate::PyObject;

#[repr(C)]
pub struct PyCriticalSection;

#[repr(C)]
pub struct PyCriticalSection2;

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection_Begin(c: *mut PyCriticalSection, op: *mut PyObject) {
    let _ = (c, op);
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection_End(c: *mut PyCriticalSection) {
    let _ = c;
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection2_Begin(
    c: *mut PyCriticalSection2,
    a: *mut PyObject,
    b: *mut PyObject,
) {
    let _ = (c, a, b);
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCriticalSection2_End(c: *mut PyCriticalSection2) {
    let _ = c;
}

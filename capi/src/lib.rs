use std::ffi;

use rustpython_vm as vm;

use malachite_bigint::BigInt;

#[repr(C)]
pub enum PyStatusType {
    PyStatusTypeOk = 0,
    PyStatusTypeError = 1,
    PyStatusTypeExit = 2,
}

#[repr(C)]
pub struct PyStatus {
    _type: PyStatusType,
    pub func: *const ffi::c_char,
    pub err_msg: *const ffi::c_char,
    pub exitcode: ffi::c_int, 
}

#[unsafe(export_name = "PyStatus_Ok")]
pub unsafe extern "C" fn status_ok() -> PyStatus {
    PyStatus {
        _type: PyStatusType::PyStatusTypeOk,
        exitcode: 0,
        err_msg: std::ptr::null(),
        func: std::ptr::null(),
    }
}

#[unsafe(export_name = "Py_Initialize")]
pub unsafe extern "C" fn initialize() {
    todo!()
}

#[unsafe(export_name = "Py_InitializeEx")]
pub unsafe extern "C" fn initialize_ex(_initsigs: ffi::c_int) {
    todo!()
}

#[repr(C)]
pub struct PyInterpreterConfig {
    use_main_obmalloc: i32,
    allow_fork: i32,
    allow_exec: i32,
    allow_threads: i32,
    allow_daemon_threads: i32,
    check_multi_interp_extensions: i32,
    gil: i32,
}

macro_rules! pylong_from_num {
    ($name:ident) => {
        {
            let big_int = BigInt::from($name);
            let pyi = vm::builtins::PyInt::from(big_int);
            let pyi = Box::new(pyi);
            Box::into_raw(pyi) as *mut ffi::c_void
        }    
    };
}

#[unsafe(export_name = "PyLong_FromLong")]
pub extern "C" fn pylong_from_long(v: ffi::c_long) -> *mut ffi::c_void {
    pylong_from_num!(v)
}

#[unsafe(export_name = "PyLong_FromUnsignedLong")]
pub extern "C" fn pylong_from_unsigned_long(v: ffi::c_ulong) -> *mut ffi::c_void {
    pylong_from_num!(v)
}

#[unsafe(export_name = "PyLong_FromUnsignedLongLong")]
pub extern "C" fn pylong_from_unsigned_long_long(v: ffi::c_ulonglong) -> *mut ffi::c_void {
    pylong_from_num!(v)
}

use std::{cell::RefCell, ffi, sync::Arc};

use rustpython_vm as vm;

mod error;

thread_local ! {
    pub static VM: RefCell<Option<Arc<vm::VirtualMachine>>> = RefCell::new(None);
}

fn get_vm() -> Arc<vm::VirtualMachine> {
    VM.with(|vm| vm.borrow().as_ref().unwrap().clone())
}

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


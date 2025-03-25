use std::{cell::RefCell, ffi, sync::Arc};

use rustpython_vm as vm;

mod error;
mod int;
mod tuple;

thread_local! {
    pub static VM: RefCell<Option<Arc<vm::VirtualMachine>>> = const { RefCell::new(None) };
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

#[allow(clippy::missing_safety_doc)]
#[unsafe(export_name = "PyStatus_Ok")]
pub unsafe extern "C" fn status_ok() -> PyStatus {
    PyStatus {
        _type: PyStatusType::PyStatusTypeOk,
        exitcode: 0,
        err_msg: std::ptr::null(),
        func: std::ptr::null(),
    }
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

thread_local! {
    pub static INTERP: RefCell<Option<vm::Interpreter>> = const { RefCell::new(None) };
}

#[unsafe(export_name = "Py_Initialize")]
pub unsafe extern "C" fn initialize() {
    // TODO: This sort of reimplemented what has already been done in the bin/lib crate, try reusing that.
    let settings = vm::Settings::default();
    let init_hooks: Vec<Box<dyn FnOnce(&mut vm::VirtualMachine)>> = vec![];
    let interp = vm::Interpreter::with_init(settings, |vm| {
        for hook in init_hooks {
            hook(vm);
        }
    });
    VM.with(|vm_ref| {
        *vm_ref.borrow_mut() = Some(interp.vm.clone());
    });
    INTERP.with(|interp_ref| {
        *interp_ref.borrow_mut() = Some(interp);
    });
}

#[unsafe(export_name = "Py_IsInitialized")]
pub unsafe extern "C" fn is_initialized() -> i32 {
    VM.with(|vm_ref| vm_ref.borrow().is_some() as i32)
}

#[unsafe(export_name = "Py_Finalize")]
pub unsafe extern "C" fn finalize() {
    VM.with(|vm_ref| {
        *vm_ref.borrow_mut() = None;
    });
    INTERP.with(|interp_ref| {
        *interp_ref.borrow_mut() = None;
    });
}

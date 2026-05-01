use crate::log_stub;
use crate::handles::init_exported_builtin_objects;
use crate::pyerrors::init_exception_statics;
use core::ffi::{c_char, c_int};
use core::sync::atomic::{AtomicBool, Ordering};
use rustpython_vm::VirtualMachine;
use rustpython_vm::vm::thread::try_with_current_vm;

pub(crate) static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn initialize_for_vm(vm: &mut VirtualMachine) {
    unsafe {
        init_exception_statics(&vm.ctx.exceptions);
        init_exported_builtin_objects(&vm.ctx);
    }
    INITIALIZED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    INITIALIZED.load(Ordering::Acquire) as _
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    let _ = try_with_current_vm(|vm| unsafe {
        init_exception_statics(&vm.ctx.exceptions);
        init_exported_builtin_objects(&vm.ctx);
    });
    INITIALIZED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Finalize() {
    let _ = Py_FinalizeEx();
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_FinalizeEx() -> c_int {
    log_stub("Py_FinalizeEx");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsFinalizing() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetVersion() -> *const c_char {
    c"3.14.0 RustPython".as_ptr()
}

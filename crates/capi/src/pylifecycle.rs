use crate::get_main_interpreter;
use crate::pyerrors::init_exception_statics;
use crate::pystate::ensure_thread_has_vm_attached;
use core::ffi::c_int;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use rustpython_vm::{Context, Interpreter, Settings};
use std::sync::Mutex;

pub(crate) static MAIN_INTERP: Mutex<Option<Interpreter>> = Mutex::new(None);

/// Request a thread local vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    get_main_interpreter()
        .as_ref()
        .expect("Interpreter not initialized")
        .enter(|vm| vm.new_thread())
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    get_main_interpreter().is_some() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    let mut interp = get_main_interpreter();
    if interp.is_none() {
        // Safety: Interpreter was not initialized before, so we can safely assume the statics are not used
        unsafe { init_exception_statics(&Context::genesis().exceptions) };

        let settings = Settings::default();
        let mut builder = Interpreter::builder(settings);

        let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
        builder = builder.add_native_modules(&defs);

        #[cfg(test)]
        {
            use rustpython_vm::common::rc::PyRc;
            builder = builder
                .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
                .init_hook(|vm| {
                    let state = PyRc::get_mut(&mut vm.state).unwrap();
                    state.config.paths.stdlib_dir = Some(rustpython_pylib::LIB_PATH.to_owned());
                });
        }

        *interp = Some(builder.build());
        drop(interp);
        ensure_thread_has_vm_attached();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Finalize() {
    let _ = Py_FinalizeEx();
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_FinalizeEx() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsFinalizing() -> c_int {
    0
}

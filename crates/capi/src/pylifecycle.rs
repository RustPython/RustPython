use crate::get_main_interpreter;
use crate::pyerrors::init_exception_statics;
use crate::pystate::ensure_thread_has_vm_attached;
use alloc::ffi::CString;
use core::ffi::{c_char, c_int, c_ulong};
use rustpython_vm::common::rc::PyRc;
use rustpython_vm::stdlib::sys;
use rustpython_vm::version::{MAJOR, MICRO, MINOR, VERSION_HEX};
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use rustpython_vm::{Context, Interpreter};
use std::sync::{LazyLock, Mutex};

pub(crate) static MAIN_INTERP: Mutex<Option<Interpreter>> = Mutex::new(None);

/// Request a thread local vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    get_main_interpreter()
        .as_ref()
        .expect("Interpreter not initialized")
        .enter(|vm| vm.new_thread())
}

#[unsafe(no_mangle)]
pub static Py_Version: c_ulong = VERSION_HEX as c_ulong;

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
        let builder = Interpreter::builder(Default::default());
        let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
        *interp = builder
            .add_native_modules(&defs)
            .init_hook(|vm| {
                let state = PyRc::get_mut(&mut vm.state).unwrap();
                let path = rustpython_pylib::LIB_PATH.to_owned();

                state.config.paths.stdlib_dir = Some(path.clone());
                state.config.paths.module_search_paths.insert(0, path);
            })
            .build()
            .into();
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

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetVersion() -> *const c_char {
    static VERSION: LazyLock<CString> = LazyLock::new(|| {
        CString::new(format!("{MAJOR}.{MINOR}.{MICRO}"))
            .expect("version string must not contain interior NULs")
    });
    VERSION.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetBuildInfo() -> *const c_char {
    static BUILD_INFO: LazyLock<CString> = LazyLock::new(|| {
        CString::new(sys::BUILD_INFO).expect("build info must not contain interior NULs")
    });
    BUILD_INFO.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetCompiler() -> *const c_char {
    sys::COMPILER.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetCopyright() -> *const c_char {
    sys::COPYRIGHT.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetPlatform() -> *const c_char {
    sys::PLATFORM.as_ptr()
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn get_version() {
        Python::attach(|py| {
            let version = py.version_info();
            assert!(version >= (3, 14));
        });

        assert!(unsafe { pyo3::ffi::Py_Version } >= 0x030d0000);
    }
}

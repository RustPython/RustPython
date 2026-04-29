use crate::log_stub;
use crate::pyerrors::init_exception_statics;
use crate::pystate::attach_vm_to_thread;
use core::ffi::{c_char, c_int};
use rustpython_vm::version::get_version;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use rustpython_vm::{Context, Interpreter};
use std::sync::{LazyLock, Once, OnceLock, mpsc};

static VM_REQUEST_TX: OnceLock<mpsc::Sender<mpsc::SyncSender<ThreadedVirtualMachine>>> =
    OnceLock::new();
pub(crate) static INITIALIZED: Once = Once::new();

/// Request a vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    let tx = VM_REQUEST_TX
        .get()
        .expect("VM request channel not initialized");
    let (response_tx, response_rx) = mpsc::sync_channel(1);
    tx.send(response_tx).expect("Failed to send VM request");
    response_rx.recv().expect("Failed to receive VM response")
}

/// Initialize the static type pointers. This should be called once during interpreter initialization,
/// and before any of the static type pointers are used.
///
/// Panics:
/// Panics when the interpreter is already initialized.
#[allow(static_mut_refs)]
pub(crate) fn init_static_type_pointers() {
    assert!(
        !INITIALIZED.is_completed(),
        "Python already initialized, we should not touch the static type pointers"
    );
    let context = Context::genesis();

    unsafe {
        init_exception_statics(&context.exceptions);
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IsInitialized() -> c_int {
    INITIALIZED.is_completed() as _
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_Initialize() {
    Py_InitializeEx(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_InitializeEx(_initsigs: c_int) {
    if INITIALIZED.is_completed() {
        panic!("Initialize called multiple times");
    }

    INITIALIZED.call_once(|| {
        init_static_type_pointers();

        let (tx, rx) = mpsc::channel();
        VM_REQUEST_TX
            .set(tx)
            .expect("VM request channel was already initialized");

        std::thread::spawn(move || {
            let interp = Interpreter::with_init(Default::default(), |_vm| {});
            interp.enter(|vm| {
                while let Ok(request) = rx.recv() {
                    request
                        .send(vm.new_thread())
                        .expect("Failed to send VM response");
                }
            })
        });
    });

    attach_vm_to_thread();
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
    static VERSION: LazyLock<String> = LazyLock::new(get_version);
    VERSION.as_str().as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn test_get_version() {
        Python::attach(|py| {
            let version = py.version_info();
            assert!(version >= (3, 14));
        });
    }
}

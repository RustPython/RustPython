use crate::log_stub;
use core::ffi::c_int;
use rustpython_vm::Interpreter;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use std::sync::{Once, OnceLock, mpsc};

static VM_REQUEST_TX: OnceLock<mpsc::Sender<mpsc::Sender<ThreadedVirtualMachine>>> =
    OnceLock::new();
static INITIALIZED: Once = Once::new();

/// Request a vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    let tx = VM_REQUEST_TX
        .get()
        .expect("VM request channel not initialized");
    let (response_tx, response_rx) = mpsc::channel();
    tx.send(response_tx).expect("Failed to send VM request");
    response_rx.recv().expect("Failed to receive VM response")
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
        let (tx, rx) = mpsc::channel();
        VM_REQUEST_TX.set(tx).expect("VM request channel was already initialized");

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

use crate::log_stub;
use crate::object::{PyLong_Type, PyTuple_Type, PyType_Type, PyUnicode_Type};
use crate::pystate::attach_vm_to_thread;
use core::ffi::c_int;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;
use rustpython_vm::{Context, Interpreter};
use std::sync::{Once, OnceLock, mpsc};

static VM_REQUEST_TX: OnceLock<mpsc::Sender<mpsc::Sender<ThreadedVirtualMachine>>> =
    OnceLock::new();
pub(crate) static INITIALIZED: Once = Once::new();

/// Request a vm from the main interpreter
pub(crate) fn request_vm_from_interpreter() -> ThreadedVirtualMachine {
    let tx = VM_REQUEST_TX
        .get()
        .expect("VM request channel not initialized");
    let (response_tx, response_rx) = mpsc::channel();
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
    let types = &context.types;

    unsafe {
        PyType_Type.write(types.type_type);
        PyLong_Type.write(types.int_type);
        PyTuple_Type.write(types.tuple_type);
        PyUnicode_Type.write(types.str_type);
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

use crate::pylifecycle::request_vm_from_interpreter;
use core::cell::RefCell;
use core::ffi::c_int;
use core::ptr;
use rustpython_vm::VirtualMachine;
use rustpython_vm::vm::thread::{ThreadedVirtualMachine, VM_CURRENT, with_current_vm};

thread_local! {
    static VM: RefCell<Option<ThreadedVirtualMachine>> = const { RefCell::new(None) };
}

pub(crate) fn with_vm<R>(f: impl FnOnce(&VirtualMachine) -> R) -> R {
    if VM_CURRENT.is_set() {
        // We have an active VM set, so use that.
        with_current_vm(f)
    } else {
        // We do not have an active vm running in this thread. Let's use our own.
        // This will panic if `PyGILState_Ensure` was not called beforehand.
        VM.with(|vm_ref| {
            let vm = vm_ref.borrow();
            let vm = vm
                .as_ref()
                .expect("Thread was not attached to an interpreter");
            vm.run(|vm| f(vm))
        })
    }
}

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;

#[repr(C)]
pub struct PyThreadState {
    _interp: *mut core::ffi::c_void,
}

/// Make sure this thread has a running vm attached. This only creates a new vm if we don't already
/// have one. So this will only create a new vm when we are in a new thread created outside RustPython.
pub(crate) fn ensure_thread_has_vm_attached() {
    if !VM_CURRENT.is_set() {
        VM.with(|vm| {
            vm.borrow_mut()
                .get_or_insert_with(request_vm_from_interpreter);
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Ensure() -> PyGILState_STATE {
    ensure_thread_has_vm_attached();

    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Release(_state: PyGILState_STATE) {}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_SaveThread() -> *mut PyThreadState {
    ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use crate::pystate::{VM, with_vm};
    use pyo3::prelude::*;
    use rustpython_vm::vm::thread::VM_CURRENT;

    #[test]
    fn test_new_thread() {
        Python::attach(|_py| {
            with_vm(|vm| {
                assert!(
                    VM_CURRENT.is_set(),
                    "This thread did not have a vm attached"
                )
            });

            std::thread::spawn(move || {
                Python::attach(|_py| {
                    with_vm(|vm| {
                        assert!(
                            VM_CURRENT.is_set(),
                            "This thread did not have a vm attached"
                        )
                    });
                });
            })
            .join()
            .unwrap();
        })
    }
}

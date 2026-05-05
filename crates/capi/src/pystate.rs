use crate::pylifecycle::request_vm_from_interpreter;
use crate::util::FfiResult;
use core::ffi::c_int;
use core::ptr;
use rustpython_vm::VirtualMachine;
use rustpython_vm::vm::thread::{
    CurrentVmAttachState, attach_current_thread, release_current_thread, with_current_vm,
};

pub(crate) fn with_vm<R: FfiResult<O>, O>(f: impl FnOnce(&VirtualMachine) -> R) -> O {
    with_current_vm(|vm| f(vm).into_output(vm))
}

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;
const PYGILSTATE_LOCKED: PyGILState_STATE = 0;
const PYGILSTATE_UNLOCKED: PyGILState_STATE = 1;

#[repr(C)]
pub struct PyThreadState {
    _interp: *mut core::ffi::c_void,
}

/// Make sure this thread has a running vm attached. This only creates a new vm if we don't already
/// have one. So this will only create a new vm when we are in a new thread created outside RustPython.
pub(crate) fn ensure_thread_has_vm_attached() -> CurrentVmAttachState {
    attach_current_thread(request_vm_from_interpreter)
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Ensure() -> PyGILState_STATE {
    match ensure_thread_has_vm_attached() {
        CurrentVmAttachState::AlreadyAttached => PYGILSTATE_LOCKED,
        CurrentVmAttachState::Attached => PYGILSTATE_UNLOCKED,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Release(state: PyGILState_STATE) {
    if state == PYGILSTATE_UNLOCKED {
        release_current_thread(CurrentVmAttachState::Attached);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_SaveThread() -> *mut PyThreadState {
    ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use crate::get_main_interpreter;
    use crate::pystate::{PyGILState_Ensure, PyGILState_Release};
    use pyo3::prelude::*;
    use rustpython_vm::vm::thread::{current_vm_is_set, with_current_vm};

    #[test]
    fn test_new_thread() {
        Python::attach(|_py| {
            with_current_vm(|_vm| {
                assert!(
                    current_vm_is_set(),
                    "This thread did not have a vm attached"
                )
            });

            std::thread::spawn(move || {
                Python::attach(|_py| {
                    with_current_vm(|_vm| {
                        assert!(
                            current_vm_is_set(),
                            "This thread did not have a vm attached"
                        )
                    });
                });
            })
            .join()
            .unwrap();
        })
    }

    #[test]
    fn test_current_vm_main_thread() {
        Python::initialize();

        // let RustPython create a vm for this thread.
        let vm = get_main_interpreter()
            .as_ref()
            .unwrap()
            .enter(|vm| vm.new_thread());

        // Attach the vm using RustPython
        vm.run(|_vm| {
            assert!(current_vm_is_set(), "This thread should have a vm attached");

            Python::attach(|_py| {
                with_current_vm(|_vm| {
                    assert!(current_vm_is_set());
                })
            })
        });
    }

    #[test]
    fn test_gilstate_release_detaches_external_thread() {
        Python::initialize();

        std::thread::spawn(|| {
            let state = PyGILState_Ensure();
            assert!(current_vm_is_set());
            PyGILState_Release(state);
            assert!(!current_vm_is_set());
        })
        .join()
        .unwrap();
    }
}

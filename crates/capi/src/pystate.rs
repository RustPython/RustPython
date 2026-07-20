use crate::get_main_interpreter;
use crate::pylifecycle::request_vm_from_interpreter;
use crate::util::FfiResult;
use core::ffi::c_int;
use rustpython_vm::vm::thread::{
    CurrentVmAttachState, SavedThreadState, attach_current_thread, release_current_thread,
    restore_current_thread, save_current_thread, with_current_vm,
};
use rustpython_vm::{Interpreter, VirtualMachine};

pub(crate) fn with_vm<R: FfiResult<O>, O>(f: impl FnOnce(&VirtualMachine) -> R) -> O {
    with_current_vm(|vm| f(vm).into_output(vm))
}

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;
const PYGILSTATE_LOCKED: PyGILState_STATE = 0;
const PYGILSTATE_UNLOCKED: PyGILState_STATE = 1;

pub type PyInterpreterState = Interpreter;

#[repr(C)]
pub struct PyThreadState {
    pub interp: *mut PyInterpreterState,
    vm: SavedThreadState,
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
    let interp = PyInterpreterState_Get();
    let state = Box::new(PyThreadState {
        interp,
        vm: save_current_thread(),
    });
    Box::into_raw(state)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyEval_RestoreThread(state: *mut PyThreadState) {
    assert!(!state.is_null(), "PyEval_RestoreThread called with null");
    // SAFETY: PyEval_SaveThread returns this allocation and CPython's API
    // requires callers to restore exactly that thread state once.
    let state = unsafe { Box::from_raw(state) };
    restore_current_thread(state.vm);
}

#[unsafe(no_mangle)]
pub extern "C" fn PyInterpreterState_Get() -> *mut PyInterpreterState {
    get_main_interpreter()
        .as_ref()
        .map(|interp| interp as *const PyInterpreterState)
        .expect("PyInterpreterState_Get called but no main interpreter was found")
        .cast_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyInterpreterState_GetID(interp: *mut PyInterpreterState) -> i64 {
    with_vm(|vm| {
        if interp.is_null() {
            return Err(vm.new_system_error("PyInterpreterState_GetID called with null interp"));
        }
        Ok(interp as usize as i64)
    })
}

#[cfg(test)]
mod tests {
    use crate::get_main_interpreter;
    use crate::pystate::{PyGILState_Ensure, PyGILState_Release};
    use pyo3::prelude::*;
    use rustpython_vm::vm::thread::{current_vm_is_set, with_current_vm};

    #[test]
    fn new_thread() {
        Python::attach(|py| {
            with_current_vm(|_vm| {
                assert!(
                    current_vm_is_set(),
                    "This thread did not have a vm attached"
                )
            });

            let handle = std::thread::spawn(move || {
                Python::attach(|_py| {
                    with_current_vm(|vm| {
                        assert!(
                            current_vm_is_set(),
                            "This thread did not have a vm attached"
                        );
                        vm.state.stop_the_world.stop_the_world(vm);
                        vm.state.stop_the_world.start_the_world(vm);
                    });
                });
            });

            py.detach(|| {
                assert!(!current_vm_is_set());
                handle.join().unwrap();
            });
            assert!(current_vm_is_set());
        })
    }

    #[test]
    fn current_vm_main_thread() {
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
    fn gilstate_release_detaches_external_thread() {
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

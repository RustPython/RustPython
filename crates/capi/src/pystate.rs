use crate::pylifecycle::request_vm_from_interpreter;
use crate::util::FfiResult;
use core::cell::RefCell;
use core::ffi::c_int;
use core::ptr;
use rustpython_vm::VirtualMachine;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;

thread_local! {
    static VM: RefCell<Option<ThreadedVirtualMachine>> = const { RefCell::new(None) };
}

pub(crate) fn with_vm<R: FfiResult<O>, O>(f: impl FnOnce(&VirtualMachine) -> R) -> O {
    VM.with(|vm_ref| {
        let vm = vm_ref.borrow();
        let vm = vm
            .as_ref()
            .expect("Thread was not attached to an interpreter");
        vm.run(|vm| f(vm).into_output(vm))
    })
}

#[allow(non_camel_case_types)]
type PyGILState_STATE = c_int;

#[repr(C)]
pub struct PyThreadState {
    _interp: *mut core::ffi::c_void,
}

pub(crate) fn attach_vm_to_thread() {
    VM.with(|vm| {
        vm.borrow_mut()
            .get_or_insert_with(request_vm_from_interpreter);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Ensure() -> PyGILState_STATE {
    attach_vm_to_thread();

    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyGILState_Release(_state: PyGILState_STATE) {}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_SaveThread() -> *mut PyThreadState {
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyEval_RestoreThread(_tstate: *mut PyThreadState) {}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyInt;

    #[test]
    fn test_new_thread() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123).unbind();

            std::thread::spawn(move || {
                Python::attach(|py| {
                    let number = number.bind(py);
                    assert!(number.is_instance_of::<PyInt>());
                });
            })
            .join()
            .unwrap();
        })
    }
}

use crate::pylifecycle::request_vm_from_interpreter;
use core::cell::RefCell;
use core::ffi::c_int;
use core::ptr;
use rustpython_vm::vm::thread::ThreadedVirtualMachine;

thread_local! {
    static VM: RefCell<Option<ThreadedVirtualMachine>> = const { RefCell::new(None) };
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

#[cfg(test)]
mod tests {
    use crate::pystate::VM;
    use pyo3::prelude::*;

    #[test]
    fn test_new_thread() {
        Python::attach(|_py| {
            assert!(
                VM.with(|vm| vm.borrow().is_some()),
                "This thread did not have a vm attached"
            );

            std::thread::spawn(move || {
                Python::attach(|_py| {
                    assert!(
                        VM.with(|vm| vm.borrow().is_some()),
                        "This thread did not have a vm attached"
                    );
                });
            })
            .join()
            .unwrap();
        })
    }
}

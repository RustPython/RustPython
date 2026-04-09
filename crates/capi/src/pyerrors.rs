use crate::{PyObject, with_vm};
use core::ffi::{c_char, c_int};
use rustpython_vm::builtins::{PyTuple, PyType};
use std::ffi::CStr;
use std::mem::MaybeUninit;

#[unsafe(no_mangle)]
pub static mut PyExc_BaseException: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_SystemError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_TypeError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_OverflowError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GetRaisedException() -> *mut PyObject {
    with_vm(|vm| vm.take_raised_exception())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetRaisedException(exc: *mut PyObject) {
    with_vm(|vm| {
        let exception = unsafe { (&*exc).to_owned().downcast_unchecked() };
        vm.push_exception(Some(exception));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetObject(exception: *mut PyObject, value: *mut PyObject) {
    with_vm(|vm| {
        let exc_type = unsafe { (&*exception).to_owned() };
        let exc_val = unsafe { (&*value).to_owned() };

        let normalized = vm
            .normalize_exception(exc_type, exc_val, vm.ctx.none())
            .unwrap_or_else(|_| {
                vm.new_type_error("exceptions must derive from BaseException".to_owned())
            });

        vm.push_exception(Some(normalized));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetString(_exception: *mut PyObject, _message: *const c_char) {
    crate::log_stub("PyErr_SetString");
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_PrintEx(_set_sys_last_vars: c_int) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_PrintEx");

        vm.print_exception(exception);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_WriteUnraisable(obj: *mut PyObject) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_WriteUnraisable");

        let object = unsafe { vm.unwrap_or_none(obj.as_ref().map(|obj| obj.to_owned())) };

        vm.run_unraisable(exception, None, object)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_NewException(
    name: *const c_char,
    base: *mut PyObject,
    dict: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let (module, name) = unsafe {
            CStr::from_ptr(name)
                .to_str()
                .expect("Exception name is not valid UTF-8")
                .rsplit_once('.')
                .expect("Exception name must be of the form 'module.ExceptionName'")
        };

        let bases = unsafe { base.as_ref() }.map(|bases| {
            if let Some(ty) = bases.downcast_ref::<PyType>() {
                vec![ty.to_owned()]
            } else if let Some(tuple) = bases.downcast_ref::<PyTuple>() {
                tuple
                    .iter()
                    .map(|item| item.to_owned().downcast())
                    .collect::<Result<Vec<_>, _>>()
                    .expect("PyErr_NewException base tuple must contain only types")
            } else {
                panic!("PyErr_NewException base must be a type or a tuple of types");
            }
        });

        assert!(
            dict.is_null(),
            "PyErr_NewException with non-null dict is not supported yet"
        );

        vm.ctx.new_exception_type(module, name, bases)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_NewExceptionWithDoc(
    name: *const c_char,
    _doc: *const c_char,
    base: *mut PyObject,
    dict: *mut PyObject,
) -> *mut PyObject {
    PyErr_NewException(name, base, dict)
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_GetTraceback(_exc: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyException_GetTraceback");
    std::ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use pyo3::exceptions::PyTypeError;
    use pyo3::prelude::*;

    #[test]
    fn test_raised_exception() {
        Python::attach(|py| {
            PyTypeError::new_err("This is a type error").restore(py);
            assert!(PyErr::take(py).is_some());
        })
    }
}

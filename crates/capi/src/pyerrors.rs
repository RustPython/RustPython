use crate::{PyObject, with_vm};
use core::convert::Infallible;
use core::ffi::CStr;
use core::ffi::{c_char, c_int};
use core::mem::MaybeUninit;
use rustpython_vm::PyResult;
use rustpython_vm::builtins::{PyTuple, PyType};
use rustpython_vm::convert::IntoObject;

#[unsafe(no_mangle)]
pub static mut PyExc_BaseException: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_Exception: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_SystemError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_TypeError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_OverflowError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_IndexError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_AttributeError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_RuntimeError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyExc_ValueError: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GetRaisedException() -> *mut PyObject {
    with_vm(|vm| vm.take_raised_exception())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetRaisedException(exc: *mut PyObject) {
    with_vm::<PyResult<Infallible>, _>(|_vm| {
        let exception = unsafe { (&*exc).to_owned().downcast_unchecked() };
        Err(exception)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetObject(exception: *mut PyObject, value: *mut PyObject) {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let exc_type = unsafe { (&*exception).to_owned() };
        let exc_val = unsafe { (&*value).to_owned() };

        let normalized = vm.normalize_exception(exc_type, exc_val, vm.ctx.none())?;

        Err(normalized)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetString(exception: *mut PyObject, message: *const c_char) {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let exc_type = unsafe { &*exception }.try_downcast_ref::<PyType>(vm)?;

        let message = unsafe { CStr::from_ptr(message) }
            .to_str()
            .expect("Exception message is not valid UTF-8");

        let exc = vm.invoke_exception(
            exc_type.to_owned(),
            vec![vm.ctx.new_str(message).into_object()],
        )?;

        Err(exc)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_PrintEx(_set_sys_last_vars: c_int) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_PrintEx");

        vm.print_exception(exception);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_WriteUnraisable(obj: *mut PyObject) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_WriteUnraisable");

        let object = unsafe { vm.unwrap_or_none(obj.as_ref().map(|obj| obj.to_owned())) };

        vm.run_unraisable(exception, None, object)
    })
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
pub extern "C" fn PyException_GetTraceback(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        exc.get_attr("__traceback__", vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_SetCause(exc: *mut PyObject, cause: *mut PyObject) {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        let cause = unsafe { cause.as_ref() }.map(|obj| obj.to_owned());
        exc.set_attr("__cause__", vm.unwrap_or_none(cause), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_SetTraceback(exc: *mut PyObject, tb: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        let traceback = unsafe { tb.as_ref() }.map(|obj| obj.to_owned());
        exc.set_attr("__traceback__", vm.unwrap_or_none(traceback), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GivenExceptionMatches(given: *mut PyObject, exc: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let given = unsafe { &*given };
        let exc = unsafe { &*exc };

        if let Some(exc_type) = exc.downcast_ref::<PyType>() {
            given.is_subclass(exc_type.as_ref(), vm)
        } else if let Some(exc_tuple) = exc.downcast_ref::<PyTuple>() {
            Ok(exc_tuple
                .iter()
                .any(|ty| given.is_subclass(ty, vm).unwrap_or_default()))
        } else {
            Ok(false)
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::PyTypeInfo;
    use pyo3::create_exception;
    use pyo3::exceptions::{PyException, PyTypeError};
    use pyo3::prelude::*;

    #[test]
    fn test_raised_exception() {
        Python::attach(|py| {
            PyTypeError::new_err("This is a type error").restore(py);
            assert!(PyErr::take(py).is_some());
        })
    }

    #[test]
    fn test_new_exception_type() {
        create_exception!(my_module, MyError, PyException, "Some description.");

        Python::attach(|py| {
            let exc = MyError::new_err("This is a new exception");
            assert!(exc.is_instance_of::<MyError>(py));
            let exc_type = MyError::type_object(py);
            assert_eq!(
                exc_type.fully_qualified_name().unwrap(),
                "my_module.MyError"
            );
        })
    }
}

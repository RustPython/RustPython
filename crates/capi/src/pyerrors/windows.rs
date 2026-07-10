use crate::unicodeobject::decode_fsdefault_and_size;
use crate::{PyObject, pyerrors::PyExc_OSError, pystate::with_vm};
use core::convert::Infallible;
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::PyType;
use rustpython_vm::convert::{IntoObject, ToPyObject};
use rustpython_vm::host_env::windows::get_last_error;
use rustpython_vm::{AsObject, PyObjectRef, PyResult};
use std::io::Error;

fn set_windows_error(
    exc: &PyObject,
    winerror: c_int,
    filename: Option<PyObjectRef>,
    filename2: Option<PyObjectRef>,
) -> *mut PyObject {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let exc_type = exc.try_downcast_ref::<PyType>(vm)?;
        let message = Error::from_raw_os_error(winerror).to_string();

        let args = vec![
            0.to_pyobject(vm),
            vm.ctx.new_str(message).into(),
            vm.unwrap_or_none(filename),
            winerror.to_pyobject(vm),
            vm.unwrap_or_none(filename2),
        ];
        let os_err = exc_type.as_object().call(args, vm)?.try_downcast(vm)?;
        Err(os_err)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErr(
    exc: *mut PyObject,
    err: c_int,
) -> *mut PyObject {
    let winerror = if err != 0 {
        err
    } else {
        get_last_error() as c_int
    };
    set_windows_error(unsafe { &*exc }, winerror, None, None)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilename(
    exc: *mut PyObject,
    err: c_int,
    filename: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let winerror = if err != 0 {
            err
        } else {
            get_last_error() as c_int
        };
        let filename = NonNull::new(filename.cast_mut())
            .map(|filename| {
                let filename_len = unsafe { CStr::from_ptr(filename.as_ptr()) }.count_bytes();
                decode_fsdefault_and_size(vm, filename.as_ptr(), filename_len)
            })
            .transpose()?
            .map(|filename| filename.into_object());

        Ok(set_windows_error(
            unsafe { &*exc },
            winerror,
            filename,
            None,
        ))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObject(
    exc: *mut PyObject,
    err: c_int,
    filename: *mut PyObject,
) -> *mut PyObject {
    let winerror = if err != 0 {
        err
    } else {
        get_last_error() as c_int
    };
    let filename = NonNull::new(filename).map(|filename| unsafe { filename.as_ref().to_owned() });
    set_windows_error(unsafe { &*exc }, winerror, filename, None)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetExcFromWindowsErrWithFilenameObjects(
    exc: *mut PyObject,
    err: c_int,
    filename: *mut PyObject,
    filename2: *mut PyObject,
) -> *mut PyObject {
    let winerror = if err != 0 {
        err
    } else {
        get_last_error() as c_int
    };
    let filename = NonNull::new(filename).map(|filename| unsafe { filename.as_ref().to_owned() });
    let filename2 = NonNull::new(filename2).map(|filename| unsafe { filename.as_ref().to_owned() });
    set_windows_error(unsafe { &*exc }, winerror, filename, filename2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErr(err: c_int) -> *mut PyObject {
    let winerror = if err != 0 {
        err
    } else {
        get_last_error() as c_int
    };
    set_windows_error(unsafe { &*PyExc_OSError }, winerror, None, None)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromWindowsErrWithFilename(
    err: c_int,
    filename: *const c_char,
) -> *mut PyObject {
    unsafe { PyErr_SetExcFromWindowsErrWithFilename(PyExc_OSError, err, filename) }
}

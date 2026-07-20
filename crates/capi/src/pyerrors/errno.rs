use crate::unicodeobject::decode_fsdefault_and_size;
use crate::{PyObject, pystate::with_vm};
use core::convert::Infallible;
use core::ffi::{CStr, c_char};
use core::ptr::NonNull;
use rustpython_vm::PyResult;
use rustpython_vm::builtins::PyType;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrno(exc: *mut PyObject) -> *mut PyObject {
    unsafe {
        PyErr_SetFromErrnoWithFilenameObjects(exc, core::ptr::null_mut(), core::ptr::null_mut())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilename(
    exc: *mut PyObject,
    filename: *const c_char,
) -> *mut PyObject {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let errno = rustpython_vm::host_env::os::get_errno();

        let filename = if filename.is_null() {
            None
        } else {
            let filename_len = unsafe { CStr::from_ptr(filename) }.count_bytes();
            let filename = decode_fsdefault_and_size(vm, filename, filename_len)?;
            Some(filename.into())
        };

        let exc_type = unsafe { &*exc }.try_downcast_ref::<PyType>(vm)?;
        let err = vm.new_errno_error_with_filenames(exc_type.to_owned(), errno, filename, None)?;
        Err(err)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObject(
    exc: *mut PyObject,
    filename: *mut PyObject,
) -> *mut PyObject {
    unsafe { PyErr_SetFromErrnoWithFilenameObjects(exc, filename, core::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyErr_SetFromErrnoWithFilenameObjects(
    exc: *mut PyObject,
    filename: *mut PyObject,
    filename2: *mut PyObject,
) -> *mut PyObject {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let errno = rustpython_vm::host_env::os::get_errno();

        let exc_type = unsafe { &*exc }.try_downcast_ref::<PyType>(vm)?;
        let filename =
            NonNull::new(filename).map(|filename| unsafe { filename.as_ref().to_owned() });
        let filename2 =
            NonNull::new(filename2).map(|filename| unsafe { filename.as_ref().to_owned() });
        let err =
            vm.new_errno_error_with_filenames(exc_type.to_owned(), errno, filename, filename2)?;

        Err(err)
    })
}

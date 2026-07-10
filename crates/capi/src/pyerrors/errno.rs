use crate::unicodeobject::decode_fsdefault_and_size;
use crate::{PyObject, pystate::with_vm};
use core::convert::Infallible;
use core::ffi::{CStr, c_char};
use core::ptr::NonNull;
use rustpython_vm::builtins::PyType;
use rustpython_vm::convert::ToPyObject;
use rustpython_vm::{AsObject, PyResult};

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
        if errno == libc::EINTR {
            vm.check_signals()?;
        }

        let filename = if filename.is_null() {
            None
        } else {
            let filename_len = unsafe { CStr::from_ptr(filename) }.count_bytes();
            let filename = decode_fsdefault_and_size(vm, filename, filename_len)?;
            Some(filename.into())
        };

        let exc_type = unsafe { &*exc }.try_downcast_ref::<PyType>(vm)?;
        let msg = if errno == 0 {
            vm.ctx.new_str("Error").into()
        } else {
            let msg = rustpython_vm::host_env::errno::strerror_string(errno)
                .unwrap_or_else(|| "Error".to_owned());
            vm.ctx.new_str(msg).into()
        };

        let args = if let Some(filename) = filename {
            vec![errno.to_pyobject(vm), msg, filename]
        } else {
            vec![errno.to_pyobject(vm), msg]
        };
        let err = exc_type.as_object().call(args, vm)?;
        let err = err
            .downcast()
            .map_err(|_| vm.new_type_error("errno helper expected an exception instance"))?;
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
        if errno == libc::EINTR {
            vm.check_signals()?;
        }

        let exc_type = unsafe { &*exc }.try_downcast_ref::<PyType>(vm)?;
        let msg = if errno == 0 {
            vm.ctx.new_str("Error").into()
        } else {
            let msg = rustpython_vm::host_env::errno::strerror_string(errno)
                .unwrap_or_else(|| "Error".to_owned());
            vm.ctx.new_str(msg).into()
        };

        let filename = NonNull::new(filename).map_or_else(
            || vm.ctx.none(),
            |filename| unsafe { filename.as_ref().to_owned() },
        );
        let filename2 = NonNull::new(filename2).map_or_else(
            || vm.ctx.none(),
            |filename| unsafe { filename.as_ref().to_owned() },
        );

        let args = if !vm.is_none(&filename2) {
            vec![
                errno.to_pyobject(vm),
                msg,
                filename,
                0.to_pyobject(vm),
                filename2,
            ]
        } else if !vm.is_none(&filename) {
            vec![errno.to_pyobject(vm), msg, filename]
        } else {
            vec![errno.to_pyobject(vm), msg]
        };

        let err = exc_type.as_object().call(args, vm)?;
        let err = err
            .downcast()
            .map_err(|_| vm.new_type_error("errno helper expected an exception instance"))?;

        Err(err)
    })
}

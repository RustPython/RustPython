use crate::PyObject;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int};
use rustpython_vm::stdlib::_io::{self, Fildes, OpenArgs};
use rustpython_vm::{PyResult, TryFromObject};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_FromFd(
    fd: c_int,
    _name: *const c_char,
    mode: *const c_char,
    buffering: c_int,
    encoding: *const c_char,
    errors: *const c_char,
    newline: *const c_char,
    closefd: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let mode = unsafe { mode.try_as_str(vm) }?;
        let encoding = unsafe { encoding.try_as_str_opt(vm) }?.map(|s| vm.ctx.new_utf8_str(s));
        let errors = unsafe { errors.try_as_str_opt(vm) }?.map(|s| vm.ctx.new_utf8_str(s));
        let newline = unsafe { newline.try_as_str_opt(vm) }?.map(|s| vm.ctx.new_utf8_str(s));
        _io::open(
            vm.ctx.new_int(fd).into(),
            Some(mode),
            OpenArgs {
                buffering: buffering as isize,
                encoding,
                errors,
                newline,
                closefd: closefd != 0,
                ..Default::default()
            },
            vm,
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_GetLine(file: *mut PyObject, n: c_int) -> *mut PyObject {
    with_vm(|vm| {
        let file = unsafe { file.assume_borrowed() };
        let line = if n > 0 {
            vm.call_method(file, "readline", (n,))?
        } else {
            vm.call_method(file, "readline", ())?
        };
        if n < 0 && line.length(vm)? == 0 {
            Err(vm.new_eof_error("EOF when reading a line"))
        } else {
            Ok(line)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_WriteObject(
    obj: *mut PyObject,
    file: *mut PyObject,
    flags: c_int,
) -> c_int {
    with_vm(|vm| -> PyResult<()> {
        let file = unsafe { file.assume_borrowed() };
        let text = if let Some(obj) = unsafe { obj.assume_borrowed_or_opt() } {
            if flags & 1 != 0 {
                obj.str(vm)?
            } else {
                obj.repr(vm)?
            }
        } else {
            vm.ctx.new_str("<NULL>")
        };
        vm.call_method(file, "write", (text,))?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFile_WriteString(s: *const c_char, file: *mut PyObject) -> c_int {
    with_vm(|vm| -> PyResult<()> {
        let s = unsafe { s.try_as_str(vm) }?;
        let file = unsafe { file.assume_borrowed() };
        vm.call_method(file, "write", (s,))?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_AsFileDescriptor(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() }.to_owned();
        Fildes::try_from_object(vm, obj).map(|fd| fd.0)
    })
}

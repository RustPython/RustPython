use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use rustpython_vm::convert::ToPyObject;
use rustpython_vm::function::FsPath;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyOS_FSPath(path: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let path = unsafe { path.assume_borrowed() }.to_owned();
        let fspath = FsPath::try_from_path_like(path, false, vm)?;
        Ok(fspath.to_pyobject(vm))
    })
}

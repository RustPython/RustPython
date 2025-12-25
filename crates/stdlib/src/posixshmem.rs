#[cfg(all(unix, not(target_os = "redox")))]
pub(crate) use _posixshmem::make_module;

#[cfg(all(unix, not(target_os = "redox")))]
#[pymodule]
mod _posixshmem {
    use std::ffi::CString;

    use crate::{
        common::os::errno_io_error,
        vm::{
            PyResult, VirtualMachine, convert::IntoPyException, function::OptionalArg, prelude::*,
        },
    };

    #[pyfunction]
    fn shm_open(
        name: PyStrRef,
        flags: libc::c_int,
        mode: OptionalArg<libc::mode_t>,
        vm: &VirtualMachine,
    ) -> PyResult<libc::c_int> {
        let name = CString::new(name.as_str()).map_err(|e| e.into_pyexception(vm))?;
        let mode = mode.unwrap_or(0o777);
        let fd = unsafe { libc::shm_open(name.as_ptr(), flags, mode) };
        if fd == -1 {
            Err(errno_io_error().into_pyexception(vm))
        } else {
            Ok(fd)
        }
    }

    #[pyfunction]
    fn shm_unlink(name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = CString::new(name.as_str()).map_err(|e| e.into_pyexception(vm))?;
        let ret = unsafe { libc::shm_unlink(name.as_ptr()) };
        if ret == -1 {
            Err(errno_io_error().into_pyexception(vm))
        } else {
            Ok(())
        }
    }
}

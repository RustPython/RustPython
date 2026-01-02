#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
pub(crate) use _posixshmem::make_module;

#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
#[pymodule]
mod _posixshmem {
    use alloc::ffi::CString;

    use crate::{
        common::os::errno_io_error,
        vm::{FromArgs, PyResult, VirtualMachine, builtins::PyStrRef, convert::IntoPyException},
    };

    #[derive(FromArgs)]
    struct ShmOpenArgs {
        #[pyarg(any)]
        name: PyStrRef,
        #[pyarg(any)]
        flags: libc::c_int,
        #[pyarg(any, default = 0o600)]
        mode: libc::mode_t,
    }

    #[pyfunction]
    fn shm_open(args: ShmOpenArgs, vm: &VirtualMachine) -> PyResult<libc::c_int> {
        let name = CString::new(args.name.as_str()).map_err(|e| e.into_pyexception(vm))?;
        let mode: libc::c_uint = args.mode as _;
        #[cfg(target_os = "freebsd")]
        let mode = mode.try_into().unwrap();
        // SAFETY: `name` is a NUL-terminated string and `shm_open` does not write through it.
        let fd = unsafe { libc::shm_open(name.as_ptr(), args.flags, mode) };
        if fd == -1 {
            Err(errno_io_error().into_pyexception(vm))
        } else {
            Ok(fd)
        }
    }

    #[pyfunction]
    fn shm_unlink(name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = CString::new(name.as_str()).map_err(|e| e.into_pyexception(vm))?;
        // SAFETY: `name` is a valid NUL-terminated string and `shm_unlink` only reads it.
        let ret = unsafe { libc::shm_unlink(name.as_ptr()) };
        if ret == -1 {
            Err(errno_io_error().into_pyexception(vm))
        } else {
            Ok(())
        }
    }
}

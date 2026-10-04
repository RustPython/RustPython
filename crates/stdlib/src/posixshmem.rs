#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
pub(crate) use _posixshmem::module_def;

#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
#[pymodule]
mod _posixshmem {
    use alloc::ffi::CString;

    use crate::vm::{
        FromArgs, PyResult, VirtualMachine, builtins::PyUtf8StrRef, convert::IntoPyException,
    };
    use rustpython_host_env::shm;

    #[derive(FromArgs)]
    struct ShmOpenArgs {
        #[pyarg(any)]
        path: PyUtf8StrRef,
        #[pyarg(any)]
        flags: core::ffi::c_int,
        #[pyarg(any, default = 0o777)]
        mode: shm::mode_t,
    }

    #[pyfunction]
    fn shm_open(args: ShmOpenArgs, vm: &VirtualMachine) -> PyResult<core::ffi::c_int> {
        let name = CString::new(args.path.as_str()).map_err(|e| e.into_pyexception(vm))?;
        let mode: core::ffi::c_uint = args.mode as _;
        shm::shm_open(name.as_c_str(), args.flags, mode).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn shm_unlink(path: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = CString::new(path.as_str()).map_err(|e| e.into_pyexception(vm))?;
        shm::shm_unlink(name.as_c_str()).map_err(|e| e.into_pyexception(vm))
    }
}

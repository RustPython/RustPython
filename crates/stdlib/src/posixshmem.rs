#[cfg(all(unix, not(target_os = "android")))]
pub(crate) use _posixshmem::module_def;

#[cfg(all(unix, not(target_os = "android")))]
#[pymodule]
mod _posixshmem {
    use crate::vm::{
        FromArgs, PyResult, VirtualMachine, builtins::PyUtf8StrRef, convert::IntoPyException,
        exceptions,
    };
    use core::hint::cold_path;
    use memchr::memchr;
    use rustpython_host_env::shm;

    #[derive(FromArgs)]
    #[from_args(legacy, fname = "shm_open")]
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
        let path = args.path.as_str();
        if memchr(b'\0', path.as_bytes()).is_some() {
            cold_path();
            return Err(exceptions::nul_char_error(vm));
        }
        shm::shm_open(path, args.flags, args.mode).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn shm_unlink(path: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        let path = path.as_str();
        if memchr(b'\0', path.as_bytes()).is_some() {
            cold_path();
            return Err(exceptions::nul_char_error(vm));
        }
        shm::shm_unlink(path).map_err(|e| e.into_pyexception(vm))
    }
}

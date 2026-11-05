#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
pub(crate) use _posixshmem::module_def;

#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
#[pymodule]
mod _posixshmem {
    use alloc::ffi::CString;

    use crate::vm::{
        PyResult, VirtualMachine,
        builtins::PyUtf8StrRef,
        convert::{IntoPyException, TryFromObject},
        function::{ArgSpec, FuncArgs},
    };
    use rustpython_host_env::shm;

    /// shm_open($module, /, path, flags, mode=511)
    /// --
    ///
    /// Open a shared memory object.  Returns a file descriptor (integer).
    #[pyfunction]
    fn shm_open(args: FuncArgs, vm: &VirtualMachine) -> PyResult<core::ffi::c_int> {
        let mut path = None;
        let mut flags = None;
        let mut mode: shm::mode_t = 0o777;
        ArgSpec {
            fname: "shm_open",
            keywords: &["path", "flags", "mode"],
            required: 2,
            max_positional: 3,
        }
        .parse_with(
            &args,
            |i, obj, vm| {
                match i {
                    0 => path = Some(PyUtf8StrRef::try_from_object(vm, obj.to_owned())?),
                    1 => flags = Some(core::ffi::c_int::try_from_object(vm, obj.to_owned())?),
                    2 => mode = shm::mode_t::try_from_object(vm, obj.to_owned())?,
                    _ => unreachable!(),
                }
                Ok(())
            },
            vm,
        )?;
        let path = path.unwrap();
        let flags = flags.unwrap();
        let name = CString::new(path.as_str()).map_err(|e| e.into_pyexception(vm))?;
        let mode: core::ffi::c_uint = mode as _;
        shm::shm_open(name.as_c_str(), flags, mode).map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn shm_unlink(path: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        let name = CString::new(path.as_str()).map_err(|e| e.into_pyexception(vm))?;
        shm::shm_unlink(name.as_c_str()).map_err(|e| e.into_pyexception(vm))
    }
}

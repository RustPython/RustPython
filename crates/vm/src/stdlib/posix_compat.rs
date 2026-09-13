// spell-checker:disable

//! `posix` compatible module for `not(any(unix, windows))`

pub(crate) use module::module_def;

#[pymodule(name = "posix", with(
    super::os::_os,
    #[cfg(any(unix, target_os = "wasi"))]
    super::posix_unix_like::_posix_unix_like,
))]
pub(crate) mod module {
    use crate::{
        Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::PyStrRef,
        ospath::OsPath,
        stdlib::os::{_os, DirFd, SupportFunc, SymlinkArgs, TargetIsDirectory},
    };

    #[pyfunction]
    pub(super) fn access(_path: PyStrRef, _mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        os_unimpl("os.access", vm)
    }

    #[cfg(not(target_os = "wasi"))]
    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        fs::remove_file(&path).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    pub(super) fn symlink(_args: SymlinkArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        os_unimpl("os.symlink", vm)
    }

    #[allow(dead_code)]
    fn os_unimpl<T>(func: &str, vm: &VirtualMachine) -> PyResult<T> {
        Err(vm.new_os_error(format!("{func} is not supported on this platform")))
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        Vec::new()
    }

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::os::module_exec(vm, module)?;
        Ok(())
    }
}

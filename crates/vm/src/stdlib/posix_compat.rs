// spell-checker:disable

//! `posix` compatible module for `not(any(unix, windows))`

pub(crate) use module::module_def;

#[pymodule(name = "posix", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::PyStrRef,
        ospath::OsPath,
        stdlib::os::{_os, DirFd, SupportFunc, TargetIsDirectory},
    };

    #[cfg(not(target_os = "wasi"))]
    use {crate::convert::IntoPyException, std::fs};

    #[cfg(target_os = "wasi")]
    use crate::exceptions::OSErrorBuilder;

    #[pyfunction]
    pub(super) fn access(_path: PyStrRef, _mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        os_unimpl("os.access", vm)
    }

    #[cfg(target_os = "wasi")]
    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(
        path: OsPath,
        dir_fd: DirFd<'_, { _os::UNLINK_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        rustpython_host_env::posix::unlinkat(dir_fd.get_opt(), &path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
    }

    #[cfg(not(target_os = "wasi"))]
    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        fs::remove_file(&path).map_err(|err| err.into_pyexception(vm))
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    pub(super) struct SymlinkArgs<'a> {
        src: OsPath,
        dst: OsPath,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<'a, { _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(_args: SymlinkArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        os_unimpl("os.symlink", vm)
    }

    #[cfg(target_os = "wasi")]
    #[pyattr]
    fn environ(vm: &VirtualMachine) -> crate::builtins::PyDictRef {
        use rustpython_host_env::os::ffi::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in crate::host_env::os::vars_os() {
            let key: PyObjectRef = vm.ctx.new_bytes(key.into_vec()).into();
            let value: PyObjectRef = vm.ctx.new_bytes(value.into_vec()).into();
            environ.set_item(&*key, value, vm).unwrap();
        }

        environ
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

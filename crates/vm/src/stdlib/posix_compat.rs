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

    #[derive(FromArgs)]
    struct AccessArgs<'a> {
        #[pyarg(any)]
        path: PyStrRef,
        #[pyarg(any)]
        mode: u8,
        #[pyarg(flatten)]
        dir_fd: DirFd<'a, 0>,
        #[pyarg(named, default)]
        effective_ids: bool,
        #[pyarg(named, default = true)]
        follow_symlinks: bool,
    }

    #[pyfunction]
    pub(super) fn access(args: AccessArgs<'_>, vm: &VirtualMachine) -> PyResult<bool> {
        let [] = args.dir_fd.0;
        let _ = (
            args.path,
            args.mode,
            args.effective_ids,
            args.follow_symlinks,
        );
        os_unimpl("os.access", vm)
    }

    #[cfg(not(target_os = "wasi"))]
    #[derive(FromArgs)]
    struct RemoveArgs<'a> {
        #[pyarg(any)]
        path: OsPath,
        #[pyarg(flatten)]
        dir_fd: DirFd<'a, 0>,
    }

    #[cfg(not(target_os = "wasi"))]
    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(args: RemoveArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = args.dir_fd.0;
        fs::remove_file(&args.path).map_err(|err| err.into_pyexception(vm))
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

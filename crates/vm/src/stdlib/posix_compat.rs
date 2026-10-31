// spell-checker:disable

//! `posix` compatible module for `not(any(unix, windows))`

pub(crate) use module::module_def;

#[pymodule(name = "posix", with(
    super::os::_os,
    #[cfg(any(unix, target_os = "wasi"))]
    super::posix_unix_like::_posix_unix_like,
))]
pub(crate) mod module {
    #[cfg(target_os = "wasi")]
    use crate::{AsObject, function::OptionalArg, ospath::PathConverter};
    use crate::{
        Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::PyStrRef,
        ospath::OsPath,
        stdlib::os::{_os, DirFd, SupportFunc, SymlinkArgs, TargetIsDirectory},
    };

    #[cfg(target_os = "wasi")]
    #[derive(FromArgs)]
    struct ChmodArgs {
        #[pyarg(any)]
        path: PyObjectRef,
        #[pyarg(any)]
        mode: PyObjectRef,
        #[pyarg(named, default, py_default = "None")]
        dir_fd: Option<PyObjectRef>,
        #[pyarg(named, optional, py_default = "(os.name != 'nt')")]
        follow_symlinks: OptionalArg<PyObjectRef>,
    }

    #[cfg(target_os = "wasi")]
    #[pyfunction]
    fn chmod(args: ChmodArgs, vm: &VirtualMachine) -> PyResult<()> {
        // Keep the clinic conversion order even though WASI has no chmod syscall.
        let path = PathConverter::new()
            .function("chmod")
            .argument("path")
            .try_path(args.path, vm)?;
        let mode = args.mode.try_index(vm)?;
        let mode = i32::try_from(mode.as_bigint())
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C int"))?;
        if let Some(dir_fd) = args.dir_fd {
            super::super::os::warn_if_bool_fd(&dir_fd, vm)?;
            let fd = dir_fd.try_index_opt(vm).unwrap_or_else(|| {
                Err(vm.new_type_error(format!(
                    "argument should be integer or None, not {}",
                    dir_fd.class().name()
                )))
            })?;
            let fd = i32::try_from(fd.as_bigint()).map_err(|_| {
                vm.new_overflow_error(if fd.as_bigint().sign() == malachite_bigint::Sign::Minus {
                    "fd is less than minimum"
                } else {
                    "fd is greater than maximum"
                })
            })?;
            if fd != rustpython_host_env::os::AT_FDCWD {
                return Err(vm.new_not_implemented_error("dir_fd unavailable on this platform"));
            }
        }
        let follow_symlinks = match args.follow_symlinks {
            OptionalArg::Present(value) => value.try_to_bool(vm)?,
            OptionalArg::Missing => true,
        };
        if !follow_symlinks {
            return Err(
                vm.new_not_implemented_error("chmod: follow_symlinks unavailable on this platform")
            );
        }
        vm.audit("os.chmod", || (path.filename(vm), mode, -1))?;
        // CPython 3.15 posixmodule.c explicitly ignores the absent WASI syscall
        // after argument conversion and auditing. No supports_* capability is set.
        Ok(())
    }

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

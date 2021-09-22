//! `posix` compatible module for `not(any(unix, windows))`

#[pymodule(name = "posix")]
pub(crate) mod posix {
    use crate::{
        builtins::PyStrRef,
        stdlib::os::{DirFd, PyPathLike, SupportFunc, TargetIsDirectory},
        PyResult, VirtualMachine,
    };
    use std::env;
    #[cfg(unix)]
    use std::os::unix::ffi as ffi_ext;
    #[cfg(target_os = "wasi")]
    use std::os::wasi::ffi as ffi_ext;

    #[pyfunction]
    pub(super) fn access(_path: PyStrRef, _mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        os_unimpl("os.access", vm)
    }

    pub const SYMLINK_DIR_FD: bool = false;

    #[derive(FromArgs)]
    #[allow(unused)]
    pub(super) struct SimlinkArgs {
        #[pyarg(any)]
        src: PyPathLike,
        #[pyarg(any)]
        dst: PyPathLike,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<{ SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(_args: SimlinkArgs, vm: &VirtualMachine) -> PyResult<()> {
        os_unimpl("os.symlink", vm)
    }

    #[cfg(target_os = "wasi")]
    #[pyattr]
    fn environ(vm: &VirtualMachine) -> crate::builtins::PyDictRef {
        use crate::ItemProtocol;
        use ffi_ext::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars_os() {
            environ
                .set_item(
                    vm.ctx.new_bytes(key.into_vec()),
                    vm.ctx.new_bytes(value.into_vec()),
                    vm,
                )
                .unwrap();
        }

        environ
    }

    #[allow(dead_code)]
    fn os_unimpl<T>(func: &str, vm: &VirtualMachine) -> PyResult<T> {
        Err(vm.new_os_error(format!("{} is not supported on this platform", func)))
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        Vec::new()
    }
}

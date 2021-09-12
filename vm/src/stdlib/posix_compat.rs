//! `posix` compatible module for `not(any(unix, windows))`

use crate::{PyObjectRef, VirtualMachine};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = module::make_module(vm);
    super::os::extend_module(vm, &module);
    module
}

#[pymodule(name = "posix")]
pub(crate) mod module {
    use crate::{
        builtins::PyStrRef,
        stdlib::os::{DirFd, PyPathLike, SupportFunc, TargetIsDirectory, _os},
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

    #[derive(FromArgs)]
    #[allow(unused)]
    pub(super) struct SimlinkArgs {
        src: PyPathLike,
        dst: PyPathLike,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<{ _os::SYMLINK_DIR_FD as usize }>,
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

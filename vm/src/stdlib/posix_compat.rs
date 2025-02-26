//! `posix` compatible module for `not(any(unix, windows))`
use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = module::make_module(vm);
    super::os::extend_module(vm, &module);
    module
}

#[pymodule(name = "posix", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::PyStrRef,
        ospath::OsPath,
        stdlib::os::{_os, DirFd, SupportFunc, TargetIsDirectory},
    };
    use std::env;

    #[pyfunction]
    pub(super) fn access(_path: PyStrRef, _mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        os_unimpl("os.access", vm)
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    pub(super) struct SymlinkArgs {
        src: OsPath,
        dst: OsPath,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<{ _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(_args: SymlinkArgs, vm: &VirtualMachine) -> PyResult<()> {
        os_unimpl("os.symlink", vm)
    }

    #[cfg(target_os = "wasi")]
    #[pyattr]
    fn environ(vm: &VirtualMachine) -> crate::builtins::PyDictRef {
        use rustpython_common::os::ffi::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars_os() {
            let key: PyObjectRef = vm.ctx.new_bytes(key.into_vec()).into();
            let value: PyObjectRef = vm.ctx.new_bytes(value.into_vec()).into();
            environ.set_item(&*key, value, vm).unwrap();
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

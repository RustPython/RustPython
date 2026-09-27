#[pymodule(sub)]
pub(crate) mod _posix_unix_like {
    use rustpython_host_env::os::ffi::OsStringExt;

    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::PyDictRef,
        exceptions::OSErrorBuilder,
        ospath::OsPath,
        stdlib::os::{_os, DirFd},
    };

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();
        for (key, value) in crate::host_env::os::vars_os() {
            let key: PyObjectRef = vm.ctx.new_bytes(key.into_vec()).into();
            let value: PyObjectRef = vm.ctx.new_bytes(value.into_vec()).into();
            environ.set_item(&*key, value, vm).unwrap();
        }

        environ
    }

    #[derive(FromArgs)]
    struct RemoveArgs<'a> {
        #[pyarg(any)]
        path: OsPath,
        #[pyarg(flatten)]
        dir_fd: DirFd<'a, { _os::UNLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(args: RemoveArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        rustpython_host_env::posix::unlinkat(args.dir_fd.get_opt(), &args.path)
            .map_err(|err| OSErrorBuilder::with_filename(&err, args.path, vm))
    }
}

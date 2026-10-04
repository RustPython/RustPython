pub(crate) use _md5::module_def;

#[pymodule]
mod _md5 {
    use crate::hashlib::_hashlib::{HashArgs, local_md5};
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn md5(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_md5(args, vm)?.into_pyobject(vm))
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

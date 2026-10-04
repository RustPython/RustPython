pub(crate) use _sha1::module_def;

#[pymodule]
mod _sha1 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha1};
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn sha1(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha1(args, vm)?.into_pyobject(vm))
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

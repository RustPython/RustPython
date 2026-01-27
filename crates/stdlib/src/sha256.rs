#[pymodule]
mod _sha256 {
    use crate::hashlib::_hashlib::{HashArgs, local_sha224, local_sha256};
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha224(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha256(args).into_pyobject(vm))
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

pub(crate) use _sha256::module_def;

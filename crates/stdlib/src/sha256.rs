#[pymodule]
mod _sha256 {
    use crate::hashlib::_hashlib::{HashArgs, openssl_sha224, openssl_sha256};
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha224(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha256(args, vm)?.into_pyobject(vm))
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

pub(crate) use _sha256::module_def;

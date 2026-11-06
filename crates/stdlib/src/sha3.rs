pub(crate) use _sha3::module_def;

#[pymodule]
mod _sha3 {
    use crate::hashlib::_hashlib::{
        HashArgs, openssl_sha3_224, openssl_sha3_256, openssl_sha3_384, openssl_sha3_512,
        openssl_shake_128, openssl_shake_256,
    };
    use crate::vm::{Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule};

    #[pyfunction]
    fn sha3_224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha3_224(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha3_256(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_384(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha3_384(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_512(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_sha3_512(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn shake_128(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_shake_128(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn shake_256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(openssl_shake_256(args, vm)?.into_pyobject(vm))
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

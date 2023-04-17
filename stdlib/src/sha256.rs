pub(crate) use _sha256::make_module;

#[pymodule]
mod _sha256 {
    use crate::hashlib::_hashlib::{local_sha224, local_sha256, HashArgs};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha224(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha256(args).into_pyobject(vm))
    }
}

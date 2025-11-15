pub(crate) use _sha3::make_module;

#[pymodule]
mod _sha3 {
    use crate::hashlib::_hashlib::{
        HashArgs, local_sha3_224, local_sha3_256, local_sha3_384, local_sha3_512, local_shake_128,
        local_shake_256,
    };
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn sha3_224(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha3_224(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha3_256(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_384(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha3_384(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn sha3_512(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_sha3_512(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn shake_128(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_shake_128(args).into_pyobject(vm))
    }

    #[pyfunction]
    fn shake_256(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_shake_256(args).into_pyobject(vm))
    }
}

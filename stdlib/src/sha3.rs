// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _sha3::make_module;

#[pymodule]
mod _sha3 {
    use crate::hashlib::_hashlib::{HashArgs, HashWrapper, HashXofWrapper, PyHasher, PyHasherXof};
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512};

    #[pyfunction(name = "sha3_224")]
    fn sha3_224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha3_224", HashWrapper::new::<Sha3_224>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "sha3_256")]
    fn sha3_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha3_256", HashWrapper::new::<Sha3_256>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "sha3_384")]
    fn sha3_384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha3_384", HashWrapper::new::<Sha3_384>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "sha3_512")]
    fn sha3_512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha3_512", HashWrapper::new::<Sha3_512>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "shake_128")]
    fn shake_128(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(
            PyHasherXof::new("shake_128", HashXofWrapper::new_shake_128(args.string))
                .into_pyobject(vm),
        )
    }

    #[pyfunction(name = "shake_256")]
    fn shake_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(
            PyHasherXof::new("shake_256", HashXofWrapper::new_shake_256(args.string))
                .into_pyobject(vm),
        )
    }
}

// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _sha512::make_module;

#[pymodule]
mod _sha512 {
    use crate::hashlib::_hashlib::{HashArgs, HashWrapper, PyHasher};
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use sha2::{Sha384, Sha512};

    #[pyfunction(name = "sha384")]
    fn sha384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha384", HashWrapper::new::<Sha384>(args.string)).into_pyobject(vm))
    }

    #[pyfunction(name = "sha512")]
    fn sha512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha512", HashWrapper::new::<Sha512>(args.string)).into_pyobject(vm))
    }
}

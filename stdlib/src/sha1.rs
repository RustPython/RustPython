// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _sha1::make_module;

#[pymodule]
mod _sha1 {
    use crate::vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use sha1::Sha1;

    use crate::hashlib::_hashlib::{HashArgs, HashWrapper, PyHasher};
    #[pyfunction(name = "sha1")]
    fn sha1(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(PyHasher::new("sha1", HashWrapper::new::<Sha1>(args.string)).into_pyobject(vm))
    }
}

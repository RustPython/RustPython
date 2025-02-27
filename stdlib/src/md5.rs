pub(crate) use _md5::make_module;

#[pymodule]
mod _md5 {
    use crate::hashlib::_hashlib::{HashArgs, local_md5};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyfunction]
    fn md5(args: HashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_md5(args).into_pyobject(vm))
    }
}

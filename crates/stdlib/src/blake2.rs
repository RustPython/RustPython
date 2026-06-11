// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _blake2::module_def;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{BlakeHashArgs, local_blake2b, local_blake2s};
    use crate::vm::{PyPayload, PyResult, VirtualMachine};

    #[pyattr(name = "_GIL_MINSIZE")]
    const GIL_MINSIZE: u16 = 2048;

    #[pyattr]
    const BLAKE2B_SALT_SIZE: u8 = 16;

    #[pyattr]
    const BLAKE2B_PERSON_SIZE: u8 = 16;

    #[pyattr]
    const BLAKE2B_MAX_KEY_SIZE: u8 = 64;

    #[pyattr]
    const BLAKE2B_MAX_DIGEST_SIZE: u8 = 64;

    #[pyattr]
    const BLAKE2S_SALT_SIZE: u8 = 8;

    #[pyattr]
    const BLAKE2S_PERSON_SIZE: u8 = 8;

    #[pyattr]
    const BLAKE2S_MAX_KEY_SIZE: u8 = 32;

    #[pyattr]
    const BLAKE2S_MAX_DIGEST_SIZE: u8 = 32;

    #[pyfunction]
    fn blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_blake2b(args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult {
        Ok(local_blake2s(args, vm)?.into_pyobject(vm))
    }
}

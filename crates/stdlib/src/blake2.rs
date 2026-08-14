// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _blake2::module_def;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{blake2b_from_args, blake2s_from_args};
    use crate::vm::{
        Py, PyPayload, PyResult, VirtualMachine, builtins::PyModule, function::FuncArgs,
    };

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
    fn blake2b(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(blake2b_from_args("blake2b", args, vm)?.into_pyobject(vm))
    }

    #[pyfunction]
    fn blake2s(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(blake2s_from_args("blake2s", args, vm)?.into_pyobject(vm))
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}

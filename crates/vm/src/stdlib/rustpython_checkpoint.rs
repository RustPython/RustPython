pub(crate) use rustpython_checkpoint::make_module;

#[pymodule]
mod rustpython_checkpoint {
    use crate::{PyResult, VirtualMachine, builtins::PyStrRef};

    #[pyfunction]
    fn checkpoint(path: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        crate::vm::checkpoint::save_checkpoint(vm, path.as_str())?;
        std::process::exit(0);
    }
}

pub(crate) use _suggestions::module_def;

#[pymodule]
mod _suggestions {
    use rustpython_vm::{PyResult, VirtualMachine, builtins::PyList};

    use crate::vm::PyObjectRef;

    #[pyfunction]
    fn _generate_suggestions(
        candidates: PyObjectRef,
        name: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let candidates = candidates
            .downcast::<PyList>()
            .map_err(|_| vm.new_type_error("candidates must be a list"))?;
        let candidates = candidates.borrow_vec();
        Ok(
            match crate::vm::suggestion::calculate_suggestions(candidates.iter(), &name) {
                Some(suggestion) => suggestion.into(),
                None => vm.ctx.none(),
            },
        )
    }
}

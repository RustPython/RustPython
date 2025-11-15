pub(crate) use _suggestions::make_module;

#[pymodule]
mod _suggestions {
    use rustpython_vm::VirtualMachine;

    use crate::vm::PyObjectRef;

    #[pyfunction]
    fn _generate_suggestions(
        candidates: Vec<PyObjectRef>,
        name: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        match crate::vm::suggestion::calculate_suggestions(candidates.iter(), &name) {
            Some(suggestion) => suggestion.into(),
            None => vm.ctx.none(),
        }
    }
}

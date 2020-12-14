pub(crate) use decl::make_module;

#[pymodule(name = "platform")]
mod decl {
    use crate::version;
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn python_implementation(_vm: &VirtualMachine) -> String {
        "RustPython".to_owned()
    }

    #[pyfunction]
    fn python_version(_vm: &VirtualMachine) -> String {
        version::get_version_number()
    }

    #[pyfunction]
    fn python_compiler(_vm: &VirtualMachine) -> String {
        version::get_compiler()
    }

    #[pyfunction]
    fn python_build(_vm: &VirtualMachine) -> (String, String) {
        (version::get_git_identifier(), version::get_git_datetime())
    }

    #[pyfunction]
    fn python_branch(_vm: &VirtualMachine) -> String {
        version::get_git_branch()
    }

    #[pyfunction]
    fn python_revision(_vm: &VirtualMachine) -> String {
        version::get_git_revision()
    }
}

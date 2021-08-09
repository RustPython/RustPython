/// Testing if a string is a keyword.
pub(crate) use decl::make_module;

#[pymodule(name = "keyword")]
mod decl {
    use itertools::Itertools;
    use rustpython_parser::lexer;

    use crate::builtins::PyStr;
    use crate::vm::VirtualMachine;
    use crate::PyObjectRef;

    #[pyfunction]
    fn iskeyword(s: PyObjectRef) -> bool {
        if let Some(s) = s.payload::<PyStr>() {
            lexer::KEYWORDS.contains_key(s.as_str())
        } else {
            false
        }
    }

    #[pyattr]
    fn kwlist(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            lexer::KEYWORDS
                .keys()
                .sorted()
                .map(|k| vm.ctx.new_str(k.to_owned()))
                .collect(),
        )
    }
}

/// Testing if a string is a keyword.
pub(crate) use keyword::make_module;

#[pymodule]
mod keyword {
    use crate::vm::{builtins::PyStr, PyObjectRef, VirtualMachine};
    use itertools::Itertools;
    use rustpython_parser::lexer;

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
                .map(|k| vm.ctx.new_utf8_str(k))
                .collect(),
        )
    }
}

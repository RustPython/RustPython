/// Testing if a string is a keyword.
pub(crate) use decl::make_module;

#[pymodule(name = "keyword")]
mod decl {
    use rustpython_parser::lexer;

    use crate::builtins::pystr::PyStrRef;
    use crate::pyobject::{BorrowValue, PyObjectRef};
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn iskeyword(s: PyStrRef) -> bool {
        lexer::KEYWORDS.contains_key(s.borrow_value())
    }

    #[pyattr]
    fn kwlist(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            lexer::KEYWORDS
                .keys()
                .map(|k| vm.ctx.new_str(k.to_owned()))
                .collect(),
        )
    }
}

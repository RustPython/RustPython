/*
 * python tokenize module.
 */
pub(crate) use decl::make_module;

// TODO: create main function when called with -m
#[pymodule(name = "tokenize")]
mod decl {
    use crate::builtins::pystr::PyStrRef;
    use crate::pyobject::{BorrowValue, PyResult};
    use crate::vm::VirtualMachine;
    use rustpython_parser::lexer;

    #[pyfunction]
    fn tokenize(s: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let source = s.borrow_value();

        // TODO: implement generator when the time has come.
        let lexer1 = lexer::make_tokenizer(source);

        let tokens = lexer1
            .map(|st| vm.ctx.new_str(format!("{:?}", st.unwrap().1)))
            .collect();
        Ok(vm.ctx.new_list(tokens))
    }
}

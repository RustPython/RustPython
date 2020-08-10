/// Testing if a string is a keyword.
pub(crate) use decl::make_module;

#[pymodule(name = "keyword")]
mod decl {
    use rustpython_parser::lexer;

    use crate::obj::objstr::PyStringRef;
    use crate::pyobject::{BorrowValue, PyObjectRef, PyResult};
    use crate::vm::VirtualMachine;

    #[pyfunction]
    fn iskeyword(s: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let keywords = lexer::get_keywords();
        let value = keywords.contains_key(s.borrow_value());
        let value = vm.ctx.new_bool(value);
        Ok(value)
    }

    #[pyattr]
    fn kwlist(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            lexer::get_keywords()
                .keys()
                .map(|k| vm.ctx.new_str(k.to_owned()))
                .collect(),
        )
    }
}

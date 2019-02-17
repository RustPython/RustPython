extern crate rustpython_parser;

use std::error::Error;

use super::compile;
use super::pyobject::{PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn eval(
    vm: &mut VirtualMachine,
    source: &str,
    scope: PyObjectRef,
    source_path: &str,
) -> PyResult {
    match compile::compile(
        source,
        &compile::Mode::Eval,
        source_path.to_string(),
        vm.ctx.code_type(),
    ) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.run_code_obj(bytecode, scope)
        }
        Err(err) => {
            let syntax_error = vm.context().exceptions.syntax_error.clone();
            Err(vm.new_exception(syntax_error, err.description().to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::eval;
    use super::VirtualMachine;

    #[test]
    fn test_print_42() {
        let source = String::from("print('Hello world')\n");
        let mut vm = VirtualMachine::new();
        let vars = vm.context().new_scope(None);
        let _result = eval(&mut vm, &source, vars, "<unittest>");

        // TODO: check result?
        //assert_eq!(
        //    parse_ast,
        // );
    }
}

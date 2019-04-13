extern crate rustpython_parser;

use crate::compile;
use crate::frame::Scope;
use crate::pyobject::PyResult;
use crate::vm::VirtualMachine;

pub fn eval(vm: &VirtualMachine, source: &str, scope: Scope, source_path: &str) -> PyResult {
    match compile::compile(vm, source, &compile::Mode::Eval, source_path.to_string()) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.run_code_obj(bytecode, scope)
        }
        Err(err) => {
            let syntax_error = vm.context().exceptions.syntax_error.clone();
            Err(vm.new_exception(syntax_error, format!("{}", err)))
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
        let vars = vm.ctx.new_scope();
        let _result = eval(&mut vm, &source, vars, "<unittest>");

        // TODO: check result?
        //assert_eq!(
        //    parse_ast,
        // );
    }
}

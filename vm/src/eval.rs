use crate::pyobject::PyResult;
use crate::scope::Scope;
use crate::vm::VirtualMachine;
use rustpython_compiler::compile;

pub fn eval(vm: &VirtualMachine, source: &str, scope: Scope, source_path: &str) -> PyResult {
    match vm.compile(source, compile::Mode::Eval, source_path.to_string()) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.run_code_obj(bytecode, scope)
        }
        Err(err) => Err(vm.new_syntax_error(&err)),
    }
}

#[cfg(test)]
mod tests {
    use super::eval;
    use super::VirtualMachine;

    #[test]
    fn test_print_42() {
        let source = String::from("print('Hello world')");
        let vm: VirtualMachine = Default::default();
        let vars = vm.new_scope_with_builtins();
        let _result = eval(&vm, &source, vars, "<unittest>");

        // TODO: check result?
        //assert_eq!(
        //    parse_ast,
        // );
    }
}

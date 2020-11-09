use crate::compile;
use crate::pyobject::PyResult;
use crate::scope::Scope;
use crate::VirtualMachine;

pub fn eval(vm: &VirtualMachine, source: &str, scope: Scope, source_path: &str) -> PyResult {
    match vm.compile(source, compile::Mode::Eval, source_path.to_owned()) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.run_code_obj(bytecode, scope)
        }
        Err(err) => Err(vm.new_syntax_error(&err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn test_print_42() {
        Interpreter::default().enter(|vm| {
            let source = String::from("print('Hello world')");
            let vars = vm.new_scope_with_builtins();
            let result = eval(&vm, &source, vars, "<unittest>").expect("this should pass");
            assert!(vm.is_none(&result));
        })
    }
}

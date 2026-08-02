use crate::{PyResult, VirtualMachine, compiler, scope::Scope};

pub fn eval(vm: &VirtualMachine, source: &str, scope: Scope, source_path: &str) -> PyResult {
    match vm.compile(source, compiler::Mode::Eval, source_path) {
        Ok(bytecode) => {
            debug!("Code object: {bytecode:?}");
            vm.run_code_obj(bytecode, scope)
        }
        Err(err) => Err(err.into_pyexception(vm, Some(source))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn print_42() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let source = String::from("print('Hello world')");
            let vars = vm.new_scope_with_builtins();
            let result = eval(vm, &source, vars, "<unittest>").expect("this should pass");
            assert!(vm.is_none(&result));
        })
    }
}

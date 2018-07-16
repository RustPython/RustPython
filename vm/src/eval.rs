extern crate rustpython_parser;

use super::compile;
use super::pyobject::{Executor, PyObjectRef, PyResult};
use super::vm::VirtualMachine;

pub fn eval(vm: &mut VirtualMachine, source: &String, locals: PyObjectRef) -> PyResult {
    match compile::compile(source, compile::Mode::Eval) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.evaluate(bytecode, locals)
        }
        Err(msg) => {
            panic!("Parsing went horribly wrong: {}", msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Executor;
    use super::VirtualMachine;
    use super::eval;

    #[test]
    fn test_print_42() {
        let source = String::from("print('Hello world')\n");
        let mut vm = VirtualMachine::new();
        let vars = vm.new_dict();
        let result = eval(&mut vm, &source, vars);

        // TODO: check result?
        //assert_eq!(
        //    parse_ast,
        // );
    }
}

extern crate rustpython_parser;

use super::compile;
use super::pyobject::PyResult;
use super::vm::VirtualMachine;

pub fn eval(vm: &mut VirtualMachine, source: &String) -> PyResult {
    match compile::compile(source, compile::Mode::Eval) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            vm.evaluate(bytecode)
        }
        Err(msg) => {
            panic!("Parsing went horribly wrong: {}", msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualMachine;
    use super::eval;

    #[test]
    fn test_print_42() {
        let source = String::from("print('Hello world')\n");
        let mut vm = VirtualMachine::new();
        let result = eval(&mut vm, &source);

        // TODO: check result?
        //assert_eq!(
        //    parse_ast,
        // );
    }
}

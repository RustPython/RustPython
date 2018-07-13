extern crate rustpython_parser;

use self::rustpython_parser::parser::parse_source;
use super::compile;
use super::pyobject::PyObjectRef;
use super::vm::VirtualMachine;

pub fn eval(vm: &mut VirtualMachine, source: &String) -> Result<PyObjectRef, PyObjectRef> {
    match parse_source(source) {
        Ok(program) => {
            debug!("Got ast: {:?}", program);
            let bytecode = compile::compile(program);
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

extern crate rustpython_parser;
extern crate rustpython_vm;

use rustpython_vm::compile;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::VirtualMachine;

fn main() {
    let mut vm = VirtualMachine::new();
    let program = "print('Hello RustPython!')";
    run_command(&mut vm, program.to_string());
}

fn _run_string(vm: &mut VirtualMachine, source: &str, source_path: Option<String>) -> PyResult {
    let code_obj = compile::compile(vm, &source.to_string(), compile::Mode::Exec, source_path)?;
    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins)); // Keep track of local variables
    vm.run_code_obj(code_obj, vars)
}

fn run_command(vm: &mut VirtualMachine, mut source: String) -> PyResult {
    // This works around https://github.com/RustPython/RustPython/issues/17
    source.push_str("\n");
    _run_string(vm, &source, None)
}

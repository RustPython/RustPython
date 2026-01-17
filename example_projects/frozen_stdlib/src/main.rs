// spell-checker:ignore aheui
//! Setting up a project with a frozen stdlib can be done *either* by using `rustpython::InterpreterBuilder` or `rustpython_vm::Interpreter::builder`.
//! See each function for example.
//!
//! See also: `aheui-rust.md` for freezing your own package.

use rustpython::InterpreterBuilderExt;
use rustpython_vm::{PyResult, VirtualMachine};

fn run(keyword: &str, vm: &VirtualMachine) -> PyResult<()> {
    let json = vm.import("json", 0)?;
    let json_loads = json.get_attr("loads", vm)?;
    let template = r#"{"key": "value"}"#;
    let json_string = template.replace("value", keyword);
    let dict = json_loads.call((vm.ctx.new_str(json_string),), vm)?;
    vm.print((dict,))?;
    Ok(())
}

fn interpreter_with_config() {
    let interpreter = rustpython::InterpreterBuilder::new()
        .init_stdlib()
        .interpreter();
    // Use interpreter.enter to reuse the same interpreter later
    interpreter.run(|vm| run("rustpython::InterpreterBuilder", vm));
}

fn interpreter_with_vm() {
    let interpreter = rustpython_vm::Interpreter::builder(Default::default())
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build();
    // Use interpreter.enter to reuse the same interpreter later
    interpreter.run(|vm| run("rustpython_vm::Interpreter::builder", vm));
}

fn main() {
    interpreter_with_config();
    interpreter_with_vm();
}

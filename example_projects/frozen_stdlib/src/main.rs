// spell-checker:ignore aheui
//! Setting up a project with a frozen stdlib can be done *either* by using `rustpython::InterpreterBuilder` or `rustpython_vm::Interpreter::builder`.
//! See each function for example.
//!
//! See also: `aheui-rust.md` for freezing your own package.

use rustpython::InterpreterBuilderExt;
use rustpython_vm::embedding::{Result, Vm};

fn run(keyword: &str, vm: Vm<'_>) -> Result<()> {
    let json = vm.import("json")?;
    let json_loads = json.get_attr("loads")?;
    let template = r#"{"key": "value"}"#;
    let json_string = template.replace("value", keyword);
    let dict = json_loads.call(&[vm.new_str(&json_string)?])?;
    vm.import("builtins")?.get_attr("print")?.call(&[dict])?;
    Ok(())
}

fn interpreter_with_config() {
    let interpreter = rustpython::InterpreterBuilder::new()
        .init_stdlib()
        .interpreter();
    // Use interpreter.enter to reuse the same interpreter later
    interpreter
        .run(|vm| run("rustpython::InterpreterBuilder", vm))
        .expect("no native workers remain");
}

fn interpreter_with_vm() {
    let interpreter = rustpython_vm::Interpreter::builder(Default::default())
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build();
    // Use interpreter.enter to reuse the same interpreter later
    interpreter
        .run(|vm| run("rustpython_vm::Interpreter::builder", vm))
        .expect("no native workers remain");
}

fn main() {
    interpreter_with_config();
    interpreter_with_vm();
}

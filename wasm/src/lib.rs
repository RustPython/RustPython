mod wasm_builtins;

extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use rustpython_vm::VirtualMachine;
use rustpython_vm::compile;
use rustpython_vm::pyobject::AttributeProtocol;
use wasm_bindgen::prelude::*;
use web_sys::console;

#[wasm_bindgen]
pub fn run_code(source: &str) -> () {
    //add hash in here
    console::log_1(&"Running RustPython".into());
    console::log_1(&"Running code:".into());
    console::log_1(&source.to_string().into());

    let mut vm = VirtualMachine::new();
    // We are monkey-patching the builtin print to use console.log
    // TODO: moneky-patch sys.stdout instead, after print actually uses sys.stdout
    vm.builtins.set_attr("print", vm.context().new_rustfunc(wasm_builtins::builtin_print));

    let code_obj = compile::compile(&mut vm, &source.to_string(), compile::Mode::Exec, None);

    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins));
    match vm.run_code_obj(code_obj.unwrap(), vars) {
        Ok(_value) => console::log_1(&"Execution successful".into()),
        Err(_) => console::log_1(&"Execution failed".into()),
    }
}

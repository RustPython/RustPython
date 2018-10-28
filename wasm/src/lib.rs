extern crate rustpython_vm;
extern crate wasm_bindgen;
use wasm_bindgen::prelude::*;
use rustpython_vm::VirtualMachine;
use rustpython_vm::compile;

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen]
pub fn run_code(source: &str) -> () {
    //add hash in here
    log("Running RustPython");
    log(&source.to_string());
    let mut vm = VirtualMachine::new();
    let code_obj = compile::compile(&mut vm, &source.to_string(), compile::Mode::Exec, None);
    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins));
    match vm.run_code_obj(code_obj.unwrap(), vars) {
        Ok(_value) => log("Execution successful"),
        Err(_) => log("Execution failed")
    }
}
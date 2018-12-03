//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
//!
extern crate wasm_bindgen;
extern crate web_sys;

use rustpython_vm::obj::objstr;
use rustpython_vm::VirtualMachine;
use rustpython_vm::pyobject::{ PyFuncArgs, PyResult };
use web_sys::console;

pub fn builtin_print(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else {
            console::log_1(&" ".into())
        }
        let v = vm.to_str(&a)?;
        let s = objstr::get_value(&v);
        console::log_1(&format!("{}", s).into())
    }
    Ok(vm.get_none())
}

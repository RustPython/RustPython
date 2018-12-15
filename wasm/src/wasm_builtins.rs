//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
//!
extern crate js_sys;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::Array;
use rustpython_vm::obj::objstr;
use rustpython_vm::pyobject::{PyFuncArgs, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::JsCast;
use web_sys::{console, window, HtmlTextAreaElement};

// The HTML id of the textarea element that act as our STDOUT
const CONSOLE_ELEMENT_ID: &str = "console";

pub fn print_to_html(text: &str) {
    let document = window().unwrap().document().unwrap();
    let element = document
        .get_element_by_id(CONSOLE_ELEMENT_ID)
        .expect("Can't find the console textarea");
    let textarea = element.dyn_ref::<HtmlTextAreaElement>().unwrap();
    let value = textarea.value();
    textarea.set_value(&format!("{}{}", value, text));
}

pub fn builtin_print_html(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else {
            print_to_html(&" ")
        }
        let v = vm.to_str(&a)?;
        let s = objstr::get_value(&v);
        print_to_html(&format!("{}\n", s))
    }
    Ok(vm.get_none())
}

pub fn builtin_print_console(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let arr = Array::new();
    for a in args.args {
        let v = vm.to_str(&a)?;
        let s = objstr::get_value(&v);
        arr.push(&s.into());
    }
    console::log(&arr);
    Ok(vm.get_none())
}

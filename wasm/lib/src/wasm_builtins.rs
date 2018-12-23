//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
//!
extern crate js_sys;
extern crate wasm_bindgen;
extern crate web_sys;

use crate::js_to_py;
use js_sys::Array;
use rustpython_vm::pyobject::{PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{console, window, HtmlTextAreaElement};

// The HTML id of the textarea element that act as our STDOUT

pub fn print_to_html(text: &str, selector: &str) -> Result<(), JsValue> {
    let document = window().unwrap().document().unwrap();
    let element = document
        .query_selector(selector)?
        .ok_or_else(|| js_sys::TypeError::new("Couldn't get element"))?;
    let textarea = element
        .dyn_ref::<HtmlTextAreaElement>()
        .ok_or_else(|| js_sys::TypeError::new("Element must be a textarea"))?;
    let value = textarea.value();
    textarea.set_value(&format!("{}{}", value, text));
    Ok(())
}

pub fn format_print_args(vm: &mut VirtualMachine, args: PyFuncArgs) -> Result<String, PyObjectRef> {
    let mut output = String::new();
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else {
            output.push_str(" ");
        }
        output.push_str(&vm.to_pystr(&a)?);
        output.push('\n');
    }
    Ok(output)
}

pub fn builtin_print_html(vm: &mut VirtualMachine, args: PyFuncArgs, selector: &str) -> PyResult {
    let output = format_print_args(vm, args)?;
    print_to_html(&output, selector).map_err(|err| js_to_py(vm, err))?;
    Ok(vm.get_none())
}

pub fn builtin_print_console(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let arr = Array::new();
    for arg in args.args {
        arr.push(&vm.to_pystr(&arg)?.into());
    }
    console::log(&arr);
    Ok(vm.get_none())
}

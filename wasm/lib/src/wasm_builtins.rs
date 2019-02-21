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
use rustpython_vm::obj::{objstr, objtype};
use rustpython_vm::pyobject::{IdProtocol, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
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
    // Handle 'sep' kwarg:
    let sep_arg = args
        .get_optional_kwarg("sep")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = sep_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "sep must be None or a string, not {}",
                objtype::get_type_name(&obj.typ())
            )));
        }
    }
    let sep_str = sep_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // Handle 'end' kwarg:
    let end_arg = args
        .get_optional_kwarg("end")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = end_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "end must be None or a string, not {}",
                objtype::get_type_name(&obj.typ())
            )));
        }
    }
    let end_str = end_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // No need to handle 'flush' kwarg, irrelevant when writing to String

    let mut output = String::new();
    let mut first = true;
    for a in args.args {
        if first {
            first = false;
        } else if let Some(ref sep_str) = sep_str {
            output.push_str(sep_str);
        } else {
            output.push(' ');
        }
        output.push_str(&vm.to_pystr(&a)?);
    }

    if let Some(end_str) = end_str {
        output.push_str(end_str.as_ref())
    } else {
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

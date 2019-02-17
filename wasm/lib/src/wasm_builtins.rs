//! Builtin function specific to WASM build.
//!
//! This is required because some feature like I/O works differently in the browser comparing to
//! desktop.
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html
//!
extern crate js_sys;
extern crate wasm_bindgen;
extern crate web_sys;

use crate::{convert, vm_class::AccessibleVM};
use js_sys::Array;
use rustpython_vm::obj::{objstr, objtype};
use rustpython_vm::pyobject::{IdProtocol, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{console, HtmlTextAreaElement};

fn window() -> web_sys::Window {
    web_sys::window().expect("Window to be available")
}

// The HTML id of the textarea element that act as our STDOUT

pub fn print_to_html(text: &str, selector: &str) -> Result<(), JsValue> {
    let document = window().document().expect("Document to be available");
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
    print_to_html(&output, selector).map_err(|err| convert::js_to_py(vm, err))?;
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

enum FetchResponseFormat {
    Json,
    Text,
}

impl FetchResponseFormat {
    fn from_str(vm: &mut VirtualMachine, s: &str) -> Result<Self, PyObjectRef> {
        match s {
            "json" => Ok(FetchResponseFormat::Json),
            "text" => Ok(FetchResponseFormat::Text),
            _ => Err(vm.new_type_error("Unkown fetch response_format".into())),
        }
    }
    fn get_response(&self, response: &web_sys::Response) -> js_sys::Promise {
        let prom = match self {
            FetchResponseFormat::Json => response.json(),
            FetchResponseFormat::Text => response.text(),
        };
        prom.expect("response method not to throw error")
    }
}

fn builtin_fetch(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (url, Some(vm.ctx.str_type())),
            (handler, Some(vm.ctx.function_type()))
        ],
        optional = [(response_format, Some(vm.ctx.str_type()))]
    );
    let response_format = match response_format {
        Some(s) => FetchResponseFormat::from_str(vm, &objstr::get_value(s))?,
        None => FetchResponseFormat::Text,
    };
    let window = window();
    let response_prom = window.fetch_with_str(&objstr::get_value(url));
    let handler = handler.clone();
    let acc_vm = AccessibleVM::from_vm(vm);
    response_prom.then(&Closure::wrap(Box::new(move |val: JsValue| {
        let response = val
            .dyn_into::<web_sys::Response>()
            .expect("val to be of type Response");
        web_sys::console::log_1(&"hey".into());
        let prom = response_format.get_response(&response);
        let acc_vm = acc_vm.clone();
        let handler = handler.clone();
        prom.then(&Closure::wrap(Box::new(move |val: JsValue| {
            let acc_vm = acc_vm.clone();
            let handler = handler.clone();
            let vm = acc_vm
                .upgrade()
                .expect("[Achievement unlucked] How did you get here?");
            let val = convert::js_to_py(vm, val);
            let mut args = PyFuncArgs::default();
            args.args.push(val);
            let _ = vm.invoke(handler, args);
        })));
    })));
    Ok(vm.get_none())
}

pub fn setup_wasm_builtins(vm: &mut VirtualMachine, scope: &PyObjectRef) {
    let ctx = vm.context();
    ctx.set_attr(scope, "print", ctx.new_rustfunc(builtin_print_console));
    ctx.set_attr(scope, "fetch", ctx.new_rustfunc(builtin_fetch));
}

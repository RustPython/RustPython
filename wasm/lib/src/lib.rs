mod convert;
mod vm_class;
mod wasm_builtins;

extern crate js_sys;
#[macro_use]
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::{Object, Reflect, TypeError};
use rustpython_vm::compile;
use rustpython_vm::pyobject::{PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use std::error::Error;
use wasm_bindgen::prelude::*;

pub use vm_class::*;

extern crate console_error_panic_hook;

#[wasm_bindgen(start)]
pub fn setup_console_error() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
}

// Hack to comment out wasm-bindgen's generated typescript definitons
#[wasm_bindgen(typescript_custom_section)]
const TS_CMT_START: &'static str = "/*";

fn base_scope(vm: &mut VirtualMachine) -> PyObjectRef {
    let builtins = vm.get_builtin_scope();
    vm.context().new_scope(Some(builtins))
}

fn eval(vm: &mut VirtualMachine, source: &str, vars: PyObjectRef) -> PyResult {
    // HACK: if the code doesn't end with newline it crashes.
    let mut source = source.to_string();
    if !source.ends_with('\n') {
        source.push('\n');
    }

    let code_obj = compile::compile(
        &source,
        &compile::Mode::Exec,
        "<string>".to_string(),
        vm.ctx.code_type(),
    )
    .map_err(|err| {
        let syntax_error = vm.context().exceptions.syntax_error.clone();
        vm.new_exception(syntax_error, err.description().to_string())
    })?;

    vm.run_code_obj(code_obj, vars)
}

#[wasm_bindgen(js_name = pyEval)]
/// Evaluate Python code
///
/// ```js
/// pyEval(code, options?);
/// ```
///
/// `code`: `string`: The Python code to run
///
/// `options`:
///
/// -   `vars?`: `{ [key: string]: any }`: Variables passed to the VM that can be
///     accessed in Python with the variable `js_vars`. Functions do work, and
///     receive the Python kwargs as the `this` argument.
/// -   `stdout?`: `(out: string) => void`: A function to replace the native print
///     function, by default `console.log`.
pub fn eval_py(source: &str, options: Option<Object>) -> Result<JsValue, JsValue> {
    let options = options.unwrap_or_else(Object::new);
    let js_vars = {
        let prop = Reflect::get(&options, &"vars".into())?;
        if prop.is_undefined() {
            None
        } else if prop.is_object() {
            Some(Object::from(prop))
        } else {
            return Err(TypeError::new("vars must be an object").into());
        }
    };
    let stdout = {
        let prop = Reflect::get(&options, &"stdout".into())?;
        if prop.is_undefined() {
            None
        } else {
            Some(prop)
        }
    };
    let mut vm = VirtualMachine::new();

    let print_fn: Box<Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult> = match stdout {
        Some(val) => {
            if let Some(selector) = val.as_string() {
                Box::new(
                    move |vm: &mut VirtualMachine, args: PyFuncArgs| -> PyResult {
                        wasm_builtins::builtin_print_html(vm, args, &selector)
                    },
                )
            } else if val.is_function() {
                let func = js_sys::Function::from(val);
                Box::new(
                    move |vm: &mut VirtualMachine, args: PyFuncArgs| -> PyResult {
                        func.call1(
                            &JsValue::UNDEFINED,
                            &wasm_builtins::format_print_args(vm, args)?.into(),
                        )
                        .map_err(|err| convert::js_to_py(vm, err))?;
                        Ok(vm.get_none())
                    },
                )
            } else {
                return Err(TypeError::new("stdout must be a function or a css selector").into());
            }
        }
        None => Box::new(wasm_builtins::builtin_print_console),
    };

    vm.ctx.set_attr(
        &vm.builtins,
        "print",
        vm.ctx.new_rustfunc_from_box(print_fn),
    );

    let vars = base_scope(&mut vm);

    let injections = vm.new_dict();

    if let Some(js_vars) = js_vars.clone() {
        for pair in convert::object_entries(&js_vars) {
            let (key, val) = pair?;
            let py_val = convert::js_to_py(&mut vm, val);
            vm.ctx.set_item(
                &injections,
                &String::from(js_sys::JsString::from(key)),
                py_val,
            );
        }
    }

    vm.ctx.set_attr(&vars, "js_vars", injections);

    let result = eval(&mut vm, source, vars);
    convert::pyresult_to_jsresult(&mut vm, result)
}

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_DEFS: &'static str = r#"
*/
export interface PyEvalOptions {
    stdout: (out: string) => void;
    vars: { [key: string]: any };
}

export function pyEval(code: string, options?: PyEvalOptions): any;
"#;

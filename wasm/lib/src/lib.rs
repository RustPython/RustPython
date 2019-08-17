pub mod browser_module;
pub mod convert;
pub mod js_module;
pub mod vm_class;
pub mod wasm_builtins;

extern crate futures;
extern crate js_sys;
#[macro_use]
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate wasm_bindgen_futures;
extern crate web_sys;

use js_sys::{Object, Reflect, TypeError};
use std::panic;
use wasm_bindgen::prelude::*;

pub use crate::vm_class::*;

const PY_EVAL_VM_ID: &str = "__py_eval_vm";

fn panic_hook(info: &panic::PanicInfo) {
    // If something errors, just ignore it; we don't want to panic in the panic hook
    use js_sys::WebAssembly::RuntimeError;
    let window = match web_sys::window() {
        Some(win) => win,
        None => return,
    };
    let msg = &info.to_string();
    let _ = Reflect::set(&window, &"__RUSTPYTHON_ERROR_MSG".into(), &msg.into());
    let error = RuntimeError::new(&msg);
    let _ = Reflect::set(&window, &"__RUSTPYTHON_ERROR".into(), &error);
    let stack = match Reflect::get(&error, &"stack".into()) {
        Ok(stack) => stack,
        Err(_) => return,
    };
    let _ = Reflect::set(&window, &"__RUSTPYTHON_ERROR_STACK".into(), &stack);
}

#[wasm_bindgen(start)]
pub fn setup_console_error() {
    std::panic::set_hook(Box::new(panic_hook));
}

// Hack to comment out wasm-bindgen's generated typescript definitons
#[wasm_bindgen(typescript_custom_section)]
const TS_CMT_START: &'static str = "/*";

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

    let vm = VMStore::init(PY_EVAL_VM_ID.into(), Some(true));

    vm.set_stdout(stdout.unwrap_or(JsValue::UNDEFINED))?;

    if let Some(js_vars) = js_vars {
        vm.add_to_scope("js_vars".into(), js_vars.into())?;
    }

    vm.exec(source)
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

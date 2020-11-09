pub mod browser_module;
pub mod convert;
pub mod js_module;
pub mod vm_class;
pub mod wasm_builtins;

#[macro_use]
extern crate rustpython_vm;

use js_sys::{Object, Reflect, TypeError};
use rustpython_vm::compile::Mode;
use std::panic;
use wasm_bindgen::prelude::*;

pub use crate::convert::PyError;
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

fn run_py(source: &str, options: Option<Object>, mode: Mode) -> Result<JsValue, JsValue> {
    let vm = VMStore::init(PY_EVAL_VM_ID.into(), Some(true));
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

    vm.set_stdout(Reflect::get(&options, &"stdout".into())?)?;

    if let Some(js_vars) = js_vars {
        vm.add_to_scope("js_vars".into(), js_vars.into())?;
    }
    vm.run(source, mode, None)
}

/// Evaluate Python code
///
/// ```js
/// var result = pyEval(code, options?);
/// ```
///
/// `code`: `string`: The Python code to run in eval mode
///
/// `options`:
///
/// -   `vars?`: `{ [key: string]: any }`: Variables passed to the VM that can be
///     accessed in Python with the variable `js_vars`. Functions do work, and
///     receive the Python kwargs as the `this` argument.
/// -   `stdout?`: `"console" | ((out: string) => void) | null`: A function to replace the
///     native print native print function, and it will be `console.log` when giving
///     `undefined` or "console", and it will be a dumb function when giving null.
#[wasm_bindgen(js_name = pyEval)]
pub fn eval_py(source: &str, options: Option<Object>) -> Result<JsValue, JsValue> {
    run_py(source, options, Mode::Eval)
}

/// Evaluate Python code
///
/// ```js
/// pyExec(code, options?);
/// ```
///
/// `code`: `string`: The Python code to run in exec mode
///
/// `options`: The options are the same as eval mode
#[wasm_bindgen(js_name = pyExec)]
pub fn exec_py(source: &str, options: Option<Object>) -> Result<(), JsValue> {
    run_py(source, options, Mode::Exec).map(drop)
}

/// Evaluate Python code
///
/// ```js
/// var result = pyExecSingle(code, options?);
/// ```
///
/// `code`: `string`: The Python code to run in exec single mode
///
/// `options`: The options are the same as eval mode
#[wasm_bindgen(js_name = pyExecSingle)]
pub fn exec_single_py(source: &str, options: Option<Object>) -> Result<JsValue, JsValue> {
    run_py(source, options, Mode::Single)
}

pub mod convert;
pub mod vm_class;
pub mod wasm_builtins;

extern crate futures;
extern crate js_sys;
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::{Object, Reflect, TypeError};
use wasm_bindgen::prelude::*;

pub use crate::vm_class::*;

const PY_EVAL_VM_ID: &str = "__py_eval_vm";

extern crate console_error_panic_hook;

#[wasm_bindgen(start)]
pub fn setup_console_error() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
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
pub fn eval_py(source: String, options: Option<Object>) -> Result<JsValue, JsValue> {
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

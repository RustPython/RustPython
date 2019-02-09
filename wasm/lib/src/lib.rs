mod wasm_builtins;

extern crate js_sys;
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::{Array, Object, Reflect, TypeError};
use rustpython_vm::compile;
use rustpython_vm::pyobject::{self, PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{prelude::*, JsCast};

// Hack to comment out wasm-bindgen's typescript definitions
#[wasm_bindgen(typescript_custom_section)]
const TS_CMT_START: &'static str = "/*";

fn py_str_err(vm: &mut VirtualMachine, py_err: &PyObjectRef) -> String {
    vm.to_pystr(&py_err)
        .unwrap_or_else(|_| "Error, and error getting error message".into())
}

fn py_to_js(vm: &mut VirtualMachine, py_obj: PyObjectRef) -> JsValue {
    let dumps = rustpython_vm::import::import(
        vm,
        std::path::PathBuf::default(),
        "json",
        &Some("dumps".into()),
    )
    .expect("Couldn't get json.dumps function");
    match vm.invoke(dumps, pyobject::PyFuncArgs::new(vec![py_obj], vec![])) {
        Ok(value) => {
            let json = vm.to_pystr(&value).unwrap();
            js_sys::JSON::parse(&json).unwrap_or(JsValue::UNDEFINED)
        }
        Err(_) => JsValue::UNDEFINED,
    }
}

fn js_to_py(vm: &mut VirtualMachine, js_val: JsValue) -> PyObjectRef {
    if js_val.is_object() {
        if Array::is_array(&js_val) {
            let js_arr: Array = js_val.into();
            let elems = js_arr
                .values()
                .into_iter()
                .map(|val| js_to_py(vm, val.expect("Iteration over array failed")))
                .collect();
            vm.ctx.new_list(elems)
        } else {
            let dict = vm.new_dict();
            for pair in Object::entries(&Object::from(js_val)).values() {
                let pair = pair.expect("Iteration over object failed");
                let key = Reflect::get(&pair, &"0".into()).unwrap();
                let val = Reflect::get(&pair, &"1".into()).unwrap();
                let py_val = js_to_py(vm, val);
                vm.ctx
                    .set_item(&dict, &String::from(js_sys::JsString::from(key)), py_val);
            }
            dict
        }
    } else if js_val.is_function() {
        let func = js_sys::Function::from(js_val);
        vm.ctx.new_rustfunc(
            move |vm: &mut VirtualMachine, args: PyFuncArgs| -> PyResult {
                let func = func.clone();
                let this = Object::new();
                for (k, v) in args.kwargs {
                    Reflect::set(&this, &k.into(), &py_to_js(vm, v))
                        .expect("Couldn't set this property");
                }
                let js_args = Array::new();
                for v in args.args {
                    js_args.push(&py_to_js(vm, v));
                }
                func.apply(&this, &js_args)
                    .map(|val| js_to_py(vm, val))
                    .map_err(|err| js_to_py(vm, err))
            },
        )
    } else if let Some(err) = js_val.dyn_ref::<js_sys::Error>() {
        let exc_type = match String::from(err.name()).as_str() {
            "TypeError" => &vm.ctx.exceptions.type_error,
            "ReferenceError" => &vm.ctx.exceptions.name_error,
            "SyntaxError" => &vm.ctx.exceptions.syntax_error,
            _ => &vm.ctx.exceptions.exception_type,
        }
        .clone();
        vm.new_exception(exc_type, err.message().into())
    } else if js_val.is_undefined() {
        // Because `JSON.stringify(undefined)` returns undefined
        vm.get_none()
    } else {
        let loads = rustpython_vm::import::import(
            vm,
            std::path::PathBuf::default(),
            "json",
            &Some("loads".into()),
        )
        .expect("Couldn't get json.loads function");

        let json = match js_sys::JSON::stringify(&js_val) {
            Ok(json) => String::from(json),
            Err(_) => return vm.get_none(),
        };
        let py_json = vm.new_str(json);

        vm.invoke(loads, pyobject::PyFuncArgs::new(vec![py_json], vec![]))
            // can safely unwrap because we know it's valid JSON
            .unwrap()
    }
}

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

    let code_obj = compile::compile(vm, &source, &compile::Mode::Exec, "<string>".to_string())?;

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
    let options = options.unwrap_or_else(|| Object::new());
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
                        .map_err(|err| js_to_py(vm, err))?;
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

    let mut vars = base_scope(&mut vm);

    let injections = vm.new_dict();

    if let Some(js_vars) = js_vars.clone() {
        for pair in Object::entries(&js_vars).values() {
            let pair = pair?;
            let key = Reflect::get(&pair, &"0".into()).unwrap();
            let val = Reflect::get(&pair, &"1".into()).unwrap();
            let py_val = js_to_py(&mut vm, val);
            vm.ctx.set_item(
                &injections,
                &String::from(js_sys::JsString::from(key)),
                py_val,
            );
        }
    }

    vm.ctx.set_item(&mut vars, "js_vars", injections);

    eval(&mut vm, source, vars)
        .map(|value| py_to_js(&mut vm, value))
        .map_err(|err| py_str_err(&mut vm, &err).into())
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

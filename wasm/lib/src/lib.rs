mod wasm_builtins;

extern crate js_sys;
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use js_sys::{Object, Reflect, TypeError};
use rustpython_vm::compile;
use rustpython_vm::pyobject::{self, PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::prelude::*;

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
    let json = match js_sys::JSON::stringify(&js_val) {
        Ok(json) => String::from(json),
        Err(_) => return vm.get_none(),
    };

    let loads = rustpython_vm::import::import(
        vm,
        std::path::PathBuf::default(),
        "json",
        &Some("loads".into()),
    )
    .expect("Couldn't get json.loads function");

    let py_json = vm.new_str(json);

    vm.invoke(loads, pyobject::PyFuncArgs::new(vec![py_json], vec![]))
        // can safely unwrap because we know it's valid JSON
        .unwrap()
}

fn eval<F>(vm: &mut VirtualMachine, source: &str, setup_scope: F) -> PyResult
where
    F: Fn(&mut VirtualMachine, &PyObjectRef),
{
    // HACK: if the code doesn't end with newline it crashes.
    let mut source = source.to_string();
    if !source.ends_with('\n') {
        source.push('\n');
    }

    let code_obj = compile::compile(vm, &source, compile::Mode::Exec, None)?;

    let builtins = vm.get_builtin_scope();
    let mut vars = vm.context().new_scope(Some(builtins));

    setup_scope(vm, &mut vars);

    vm.run_code_obj(code_obj, vars)
}

#[wasm_bindgen(js_name = pyEval)]
pub fn eval_py(source: &str, options: Option<Object>) -> Result<JsValue, JsValue> {
    let options = options.unwrap_or_else(|| Object::new());
    let js_vars = {
        let prop = Reflect::get(&options, &"vars".into())?;
        let prop = Object::from(prop);
        if prop.is_undefined() {
            None
        } else if prop.is_object() {
            Some(prop)
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

    let print_fn: Box<(Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult)> = match stdout {
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

    let res = eval(&mut vm, source, |vm, vars| {
        let injections = vm.new_dict();

        if let Some(js_vars) = js_vars.clone() {
            for pair in js_sys::try_iter(&Object::entries(&js_vars))
                .unwrap()
                .unwrap()
            {
                let pair = pair.unwrap();
                let key = Reflect::get(&pair, &"0".into()).unwrap();
                let val = Reflect::get(&pair, &"1".into()).unwrap();
                let py_val = js_to_py(vm, val);
                vm.ctx.set_item(
                    &injections,
                    &String::from(js_sys::JsString::from(key)),
                    py_val,
                );
            }
        }

        vm.ctx.set_item(vars, "js_vars", injections);
    });

    res.map(|value| py_to_js(&mut vm, value))
        .map_err(|err| py_str_err(&mut vm, &err).into())
}

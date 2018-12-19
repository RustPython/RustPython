mod wasm_builtins;

extern crate js_sys;
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use rustpython_vm::compile;
use rustpython_vm::pyobject::{self, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::prelude::*;
use web_sys::console;

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

#[wasm_bindgen]
pub fn eval_py(source: &str, js_injections: Option<js_sys::Object>) -> Result<JsValue, JsValue> {
    if let Some(js_injections) = js_injections.clone() {
        if !js_injections.is_object() {
            return Err(js_sys::TypeError::new("The second argument must be an object").into());
        }
    }

    let mut vm = VirtualMachine::new();

    vm.ctx.set_attr(
        &vm.builtins,
        "print",
        vm.context()
            .new_rustfunc(wasm_builtins::builtin_print_console),
    );

    let res = eval(&mut vm, source, |vm, vars| {
        let injections = if let Some(js_injections) = js_injections.clone() {
            js_to_py(vm, js_injections.into())
        } else {
            vm.new_dict()
        };

        vm.ctx.set_item(vars, "js_vars", injections);
    });

    res.map(|value| py_to_js(&mut vm, value))
        .map_err(|err| py_str_err(&mut vm, &err).into())
}

#[wasm_bindgen]
pub fn run_from_textbox(source: &str) -> Result<JsValue, JsValue> {
    //add hash in here
    console::log_1(&"Running RustPython".into());
    console::log_1(&"Running code:".into());
    console::log_1(&source.to_string().into());

    let mut vm = VirtualMachine::new();

    // We are monkey-patching the builtin print to use console.log
    // TODO: monkey-patch sys.stdout instead, after print actually uses sys.stdout
    vm.ctx.set_attr(
        &vm.builtins,
        "print",
        vm.context().new_rustfunc(wasm_builtins::builtin_print_html),
    );

    match eval(&mut vm, source, |_, _| {}) {
        Ok(value) => {
            console::log_1(&"Execution successful".into());
            match value.borrow().kind {
                pyobject::PyObjectKind::None => {}
                _ => {
                    if let Ok(text) = vm.to_pystr(&value) {
                        wasm_builtins::print_to_html(&text);
                    }
                }
            }
            Ok(JsValue::UNDEFINED)
        }
        Err(err) => {
            console::log_1(&"Execution failed".into());
            Err(py_str_err(&mut vm, &err).into())
        }
    }
}

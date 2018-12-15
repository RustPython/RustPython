mod wasm_builtins;

extern crate js_sys;
extern crate num_bigint;
extern crate rustpython_vm;
extern crate wasm_bindgen;
extern crate web_sys;

use num_bigint::BigInt;
use rustpython_vm::compile;
use rustpython_vm::pyobject::{self, IdProtocol, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::prelude::*;
use web_sys::console;

fn py_str_err(vm: &mut VirtualMachine, py_err: &PyObjectRef) -> String {
    vm.to_pystr(&py_err)
        .unwrap_or_else(|_| "Error, and error getting error message".into())
}

fn py_to_js(vm: &mut VirtualMachine, py_obj: &PyObjectRef) -> JsValue {
    use pyobject::PyObjectKind;
    let py_obj = py_obj.borrow();
    match py_obj.kind {
        PyObjectKind::String { ref value } => value.into(),
        PyObjectKind::Integer { ref value } => {
            if let Some(ref typ) = py_obj.typ {
                if typ.is(&vm.ctx.bool_type()) {
                    let out_bool = value == &BigInt::new(num_bigint::Sign::Plus, vec![1]);
                    return out_bool.into();
                }
            }
            let int = vm.ctx.new_int(value.clone());
            rustpython_vm::obj::objfloat::make_float(vm, &int)
                .unwrap()
                .into()
        }
        PyObjectKind::Float { ref value } => JsValue::from_f64(*value),
        PyObjectKind::Bytes { ref value } => {
            let arr = js_sys::Uint8Array::new(&JsValue::from(value.len() as u32));
            for (i, byte) in value.iter().enumerate() {
                console::log_1(&JsValue::from(i as u32));
                js_sys::Reflect::set(&arr, &JsValue::from(i as u32), &JsValue::from(*byte))
                    .unwrap();
            }
            arr.into()
        }
        PyObjectKind::Sequence { ref elements } => {
            let arr = js_sys::Array::new();
            for val in elements {
                arr.push(&py_to_js(vm, val));
            }
            arr.into()
        }
        PyObjectKind::Dict { ref elements } => {
            let obj = js_sys::Object::new();
            for (key, (_, val)) in elements {
                js_sys::Reflect::set(&obj, &key.into(), &py_to_js(vm, val))
                    .expect("couldn't set property of object");
            }
            obj.into()
        }
        PyObjectKind::None => JsValue::UNDEFINED,
        _ => JsValue::UNDEFINED,
    }
}

fn eval(vm: &mut VirtualMachine, source: &str) -> PyResult {
    let code_obj = compile::compile(vm, &source.to_string(), compile::Mode::Exec, None)?;

    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins));
    vm.run_code_obj(code_obj, vars)
}

#[wasm_bindgen]
pub fn eval_py(source: &str) -> Result<JsValue, JsValue> {
    let mut vm = VirtualMachine::new();

    vm.ctx.set_attr(
        &vm.builtins,
        "print",
        vm.context().new_rustfunc(wasm_builtins::builtin_log),
    );

    eval(&mut vm, source)
        .map(|value| py_to_js(&mut vm, &value))
        .map_err(|err| py_str_err(&mut vm, &err).into())
}

#[wasm_bindgen]
pub fn run_code(source: &str) -> Result<JsValue, JsValue> {
    //add hash in here
    console::log_1(&"Running RustPython".into());
    console::log_1(&"Running code:".into());
    console::log_1(&source.to_string().into());

    let mut vm = VirtualMachine::new();
    // We are monkey-patching the builtin print to use console.log
    // TODO: moneky-patch sys.stdout instead, after print actually uses sys.stdout
    vm.ctx.set_attr(
        &vm.builtins,
        "print",
        vm.context().new_rustfunc(wasm_builtins::builtin_print),
    );

    match eval(&mut vm, source) {
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

use js_sys::{Array, Object, Reflect};
use rustpython_vm::pyobject::{self, PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{prelude::*, JsCast};

pub fn py_str_err(vm: &mut VirtualMachine, py_err: &PyObjectRef) -> String {
    vm.to_pystr(&py_err)
        .unwrap_or_else(|_| "Error, and error getting error message".into())
}

pub fn py_to_js(vm: &mut VirtualMachine, py_obj: PyObjectRef) -> JsValue {
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

pub fn js_to_py(vm: &mut VirtualMachine, js_val: JsValue) -> PyObjectRef {
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

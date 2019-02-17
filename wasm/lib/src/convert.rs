use js_sys::{Array, Object, Reflect};
use rustpython_vm::pyobject::{self, PyFuncArgs, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use vm_class::{AccessibleVM, WASMVirtualMachine};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};

pub fn py_str_err(vm: &mut VirtualMachine, py_err: &PyObjectRef) -> String {
    vm.to_pystr(&py_err)
        .unwrap_or_else(|_| "Error, and error getting error message".into())
}

pub fn py_to_js(
    vm: &mut VirtualMachine,
    py_obj: PyObjectRef,
    wasm_vm: Option<WASMVirtualMachine>,
) -> JsValue {
    if let Some(wasm_vm) = wasm_vm {
        if rustpython_vm::obj::objtype::isinstance(&py_obj, &vm.ctx.function_type()) {
            let closure =
                move |args: Option<Array>, kwargs: Option<Object>| -> Result<JsValue, JsValue> {
                    let py_obj = py_obj.clone();
                    wasm_vm.assert_valid()?;
                    let acc_vm = AccessibleVM::from(wasm_vm.clone());
                    let vm = acc_vm
                        .upgrade()
                        .expect("acc. VM to be invalid when WASM vm is valid");
                    let mut py_func_args = rustpython_vm::pyobject::PyFuncArgs::default();
                    if let Some(ref args) = args {
                        for arg in args.values() {
                            py_func_args
                                .args
                                .push(js_to_py(vm, arg?, Some(wasm_vm.clone())));
                        }
                    }
                    if let Some(ref kwargs) = kwargs {
                        for pair in object_entries(kwargs) {
                            let (key, val) = pair?;
                            py_func_args.kwargs.push((
                                js_sys::JsString::from(key).into(),
                                js_to_py(vm, val, Some(wasm_vm.clone())),
                            ));
                        }
                    }
                    let result = vm.invoke(py_obj.clone(), py_func_args);
                    pyresult_to_jsresult(vm, result, Some(wasm_vm.clone()))
                };
            let closure = Closure::wrap(Box::new(closure)
                as Box<dyn Fn(Option<Array>, Option<Object>) -> Result<JsValue, JsValue>>);
            let func = closure.as_ref().clone();

            // TODO: Come up with a way of managing closure handles
            closure.forget();

            return func;
        }
    }
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

pub fn object_entries(obj: &Object) -> impl Iterator<Item = Result<(JsValue, JsValue), JsValue>> {
    Object::entries(obj).values().into_iter().map(|pair| {
        pair.map(|pair| {
            let key = Reflect::get(&pair, &"0".into()).unwrap();
            let val = Reflect::get(&pair, &"1".into()).unwrap();
            (key, val)
        })
    })
}

pub fn pyresult_to_jsresult(
    vm: &mut VirtualMachine,
    result: PyResult,
    wasm_vm: Option<WASMVirtualMachine>,
) -> Result<JsValue, JsValue> {
    result
        .map(|value| py_to_js(vm, value, wasm_vm))
        .map_err(|err| py_str_err(vm, &err).into())
}

pub fn js_to_py(
    vm: &mut VirtualMachine,
    js_val: JsValue,
    // Accept a WASM VM because if js_val is a function it has to be able to convert
    // its arguments to JS, and those arguments might include a closure
    wasm_vm: Option<WASMVirtualMachine>,
) -> PyObjectRef {
    if js_val.is_object() {
        if Array::is_array(&js_val) {
            let js_arr: Array = js_val.into();
            let elems = js_arr
                .values()
                .into_iter()
                .map(|val| {
                    js_to_py(
                        vm,
                        val.expect("Iteration over array failed"),
                        wasm_vm.clone(),
                    )
                })
                .collect();
            vm.ctx.new_list(elems)
        } else {
            let dict = vm.new_dict();
            for pair in Object::entries(&Object::from(js_val)).values() {
                let pair = pair.expect("Iteration over object failed");
                let key = Reflect::get(&pair, &"0".into()).unwrap();
                let val = Reflect::get(&pair, &"1".into()).unwrap();
                let py_val = js_to_py(vm, val, wasm_vm.clone());
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
                    Reflect::set(&this, &k.into(), &py_to_js(vm, v, wasm_vm.clone()))
                        .expect("Couldn't set this property");
                }
                let js_args = Array::new();
                for v in args.args {
                    js_args.push(&py_to_js(vm, v, wasm_vm.clone()));
                }
                func.apply(&this, &js_args)
                    .map(|val| js_to_py(vm, val, wasm_vm.clone()))
                    .map_err(|err| js_to_py(vm, err, wasm_vm.clone()))
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

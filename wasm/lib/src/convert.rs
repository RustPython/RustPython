use crate::vm_class::{AccessibleVM, WASMVirtualMachine};
use js_sys::{Array, ArrayBuffer, Object, Reflect, Uint8Array};
use rustpython_vm::obj::{objbytes, objtype};
use rustpython_vm::pyobject::{self, PyFuncArgs, PyObjectRef, PyResult, DictProtocol};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};

pub fn py_str_err(vm: &mut VirtualMachine, py_err: &PyObjectRef) -> String {
    vm.to_pystr(&py_err)
        .unwrap_or_else(|_| "Error, and error getting error message".into())
}

pub fn js_py_typeerror(vm: &mut VirtualMachine, js_err: JsValue) -> PyObjectRef {
    let msg = js_err.unchecked_into::<js_sys::Error>().to_string();
    vm.new_type_error(msg.into())
}

pub fn py_to_js(vm: &mut VirtualMachine, py_obj: PyObjectRef) -> JsValue {
    if let Some(ref wasm_id) = vm.wasm_id {
        if objtype::isinstance(&py_obj, &vm.ctx.function_type()) {
            let wasm_vm = WASMVirtualMachine {
                id: wasm_id.clone(),
            };
            let mut py_obj = Some(py_obj);
            let closure =
                move |args: Option<Array>, kwargs: Option<Object>| -> Result<JsValue, JsValue> {
                    let py_obj = match wasm_vm.assert_valid() {
                        Ok(_) => py_obj.clone().expect("py_obj to be valid if VM is valid"),
                        Err(err) => {
                            py_obj = None;
                            return Err(err);
                        }
                    };
                    let acc_vm = AccessibleVM::from(wasm_vm.clone());
                    let vm = &mut acc_vm
                        .upgrade()
                        .expect("acc. VM to be invalid when WASM vm is valid");
                    let mut py_func_args = rustpython_vm::pyobject::PyFuncArgs::default();
                    if let Some(ref args) = args {
                        for arg in args.values() {
                            py_func_args.args.push(js_to_py(vm, arg?));
                        }
                    }
                    if let Some(ref kwargs) = kwargs {
                        for pair in object_entries(kwargs) {
                            let (key, val) = pair?;
                            py_func_args
                                .kwargs
                                .push((js_sys::JsString::from(key).into(), js_to_py(vm, val)));
                        }
                    }
                    let result = vm.invoke(py_obj.clone(), py_func_args);
                    pyresult_to_jsresult(vm, result)
                };
            let closure = Closure::wrap(Box::new(closure)
                as Box<dyn FnMut(Option<Array>, Option<Object>) -> Result<JsValue, JsValue>>);
            let func = closure.as_ref().clone();

            // TODO: Come up with a way of managing closure handles
            closure.forget();

            return func;
        }
    }
    if objtype::isinstance(&py_obj, &vm.ctx.bytes_type())
        || objtype::isinstance(&py_obj, &vm.ctx.bytearray_type())
    {
        let bytes = objbytes::get_value(&py_obj);
        let arr = Uint8Array::new_with_length(bytes.len() as u32);
        for (i, byte) in bytes.iter().enumerate() {
            Reflect::set(&arr, &(i as u32).into(), &(*byte).into())
                .expect("setting Uint8Array value failed");
        }
        arr.into()
    } else {
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

pub fn pyresult_to_jsresult(vm: &mut VirtualMachine, result: PyResult) -> Result<JsValue, JsValue> {
    result
        .map(|value| py_to_js(vm, value))
        .map_err(|err| py_str_err(vm, &err).into())
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
        } else if ArrayBuffer::is_view(&js_val) || js_val.is_instance_of::<ArrayBuffer>() {
            // unchecked_ref because if it's not an ArrayByffer it could either be a TypedArray
            // or a DataView, but they all have a `buffer` property
            let u8_array = js_sys::Uint8Array::new(
                &js_val
                    .dyn_ref::<ArrayBuffer>()
                    .cloned()
                    .unwrap_or_else(|| js_val.unchecked_ref::<Uint8Array>().buffer()),
            );
            let mut vec = Vec::with_capacity(u8_array.length() as usize);
            // TODO: use Uint8Array::copy_to once updating js_sys doesn't break everything
            u8_array.for_each(&mut |byte, _, _| vec.push(byte));
            vm.ctx.new_bytes(vec)
        } else {
            let dict = vm.new_dict();
            for pair in object_entries(&Object::from(js_val)) {
                let (key, val) = pair.expect("iteration over object to not fail");
                let py_val = js_to_py(vm, val);
                dict.set_item(&vm.ctx, &String::from(js_sys::JsString::from(key)), py_val);
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
                        .expect("property to be settable");
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
        .expect("json.loads function to be available");

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

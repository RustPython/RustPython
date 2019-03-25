use js_sys::{Array, ArrayBuffer, Object, Promise, Reflect, Uint8Array};
use num_traits::cast::ToPrimitive;
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};

use rustpython_vm::function::PyFuncArgs;
use rustpython_vm::obj::{objbytes, objint, objsequence, objtype};
use rustpython_vm::pyobject::{DictProtocol, PyObjectRef, PyResult, PyValue};
use rustpython_vm::VirtualMachine;

use crate::browser_module;
use crate::vm_class::{AccessibleVM, WASMVirtualMachine};

pub fn py_err_to_js_err(vm: &VirtualMachine, py_err: &PyObjectRef) -> JsValue {
    macro_rules! map_exceptions {
        ($py_exc:ident, $msg:expr, { $($py_exc_ty:expr => $js_err_new:expr),*$(,)? }) => {
            $(if objtype::isinstance($py_exc, $py_exc_ty) {
                JsValue::from($js_err_new($msg))
            } else)* {
                JsValue::from(js_sys::Error::new($msg))
            }
        };
    }
    let msg = match vm
        .get_attribute(py_err.clone(), "msg")
        .ok()
        .and_then(|msg| vm.to_pystr(&msg).ok())
    {
        Some(msg) => msg,
        None => return js_sys::Error::new("error getting error").into(),
    };
    let js_err = map_exceptions!(py_err,& msg, {
        // TypeError is sort of a catch-all for "this value isn't what I thought it was like"
        &vm.ctx.exceptions.type_error => js_sys::TypeError::new,
        &vm.ctx.exceptions.value_error => js_sys::TypeError::new,
        &vm.ctx.exceptions.index_error => js_sys::TypeError::new,
        &vm.ctx.exceptions.key_error => js_sys::TypeError::new,
        &vm.ctx.exceptions.attribute_error => js_sys::TypeError::new,
        &vm.ctx.exceptions.name_error => js_sys::ReferenceError::new,
        &vm.ctx.exceptions.syntax_error => js_sys::SyntaxError::new,
    });
    if let Ok(tb) = vm.get_attribute(py_err.clone(), "__traceback__") {
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let elements = objsequence::get_elements(&tb).to_vec();
            if let Some(top) = elements.get(0) {
                if objtype::isinstance(&top, &vm.ctx.tuple_type()) {
                    let element = objsequence::get_elements(&top);

                    if let Some(lineno) = objint::to_int(vm, &element[1], 10)
                        .ok()
                        .and_then(|lineno| lineno.to_u32())
                    {
                        let _ = Reflect::set(&js_err, &"row".into(), &lineno.into());
                    }
                }
            }
        }
    }
    js_err
}

pub fn js_py_typeerror(vm: &VirtualMachine, js_err: JsValue) -> PyObjectRef {
    let msg = js_err.unchecked_into::<js_sys::Error>().to_string();
    vm.new_type_error(msg.into())
}

pub fn py_to_js(vm: &VirtualMachine, py_obj: PyObjectRef) -> JsValue {
    if let Some(ref wasm_id) = vm.wasm_id {
        if objtype::isinstance(&py_obj, &vm.ctx.function_type()) {
            let wasm_vm = WASMVirtualMachine {
                id: wasm_id.clone(),
            };
            let weak_py_obj = wasm_vm.push_held_rc(py_obj).unwrap();

            let closure =
                move |args: Option<Array>, kwargs: Option<Object>| -> Result<JsValue, JsValue> {
                    let py_obj = match wasm_vm.assert_valid() {
                        Ok(_) => weak_py_obj
                            .upgrade()
                            .expect("weak_py_obj to be valid if VM is valid"),
                        Err(err) => {
                            return Err(err);
                        }
                    };
                    let acc_vm = AccessibleVM::from(wasm_vm.clone());
                    let stored_vm = acc_vm
                        .upgrade()
                        .expect("acc. VM to be invalid when WASM vm is valid");
                    let vm = &stored_vm.vm;
                    let mut py_func_args = PyFuncArgs::default();
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

            // stores pretty much nothing, it's fine to leak this because if it gets dropped
            // the error message is worse
            closure.forget();

            return func;
        }
    }
    // the browser module might not be injected
    if vm.try_class("browser", "Promise").is_ok() {
        if let Some(py_prom) = py_obj.payload::<browser_module::PyPromise>() {
            return py_prom.value().into();
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
        match vm.serialize(&py_obj) {
            Ok(json) => js_sys::JSON::parse(&json).unwrap_or(JsValue::UNDEFINED),
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

pub fn pyresult_to_jsresult(vm: &VirtualMachine, result: PyResult) -> Result<JsValue, JsValue> {
    result
        .map(|value| py_to_js(vm, value))
        .map_err(|err| py_err_to_js_err(vm, &err))
}

pub fn js_to_py(vm: &VirtualMachine, js_val: JsValue) -> PyObjectRef {
    if js_val.is_object() {
        if let Some(promise) = js_val.dyn_ref::<Promise>() {
            // the browser module might not be injected
            if vm.try_class("browser", "Promise").is_ok() {
                return browser_module::PyPromise::new(promise.clone())
                    .into_ref(vm)
                    .into_object();
            }
        }
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
            let dict = vm.ctx.new_dict();
            for pair in object_entries(&Object::from(js_val)) {
                let (key, val) = pair.expect("iteration over object to not fail");
                let py_val = js_to_py(vm, val);
                dict.set_item(&vm.ctx, &String::from(js_sys::JsString::from(key)), py_val);
            }
            dict.into_object()
        }
    } else if js_val.is_function() {
        let func = js_sys::Function::from(js_val);
        vm.ctx
            .new_rustfunc(move |vm: &VirtualMachine, args: PyFuncArgs| -> PyResult {
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
            })
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
        let json = match js_sys::JSON::stringify(&js_val) {
            Ok(json) => String::from(json),
            Err(_) => return vm.get_none(),
        };
        vm.deserialize(&json).unwrap_or_else(|_| vm.get_none())
    }
}

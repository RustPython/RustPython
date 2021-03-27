use js_sys::{Array, ArrayBuffer, Object, Promise, Reflect, SyntaxError, Uint8Array};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};

use rustpython_parser::error::ParseErrorType;
use rustpython_vm::byteslike::PyBytesLike;
use rustpython_vm::compile::{CompileError, CompileErrorType};
use rustpython_vm::exceptions::PyBaseExceptionRef;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::pyobject::{
    ItemProtocol, PyObjectRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use rustpython_vm::VirtualMachine;
use rustpython_vm::{exceptions, py_serde};

use crate::js_module;
use crate::vm_class::{stored_vm_from_wasm, WASMVirtualMachine};

#[wasm_bindgen(inline_js = r"
export class PyError extends Error {
    constructor(info) {
        const msg = info.args[0];
        if (typeof msg === 'string') super(msg);
        else super();
        this.info = info;
    }
    get name() { return this.info.exc_type; }
    get traceback() { return this.info.traceback; }
    toString() { return this.info.rendered; }
}
")]
extern "C" {
    pub type PyError;
    #[wasm_bindgen(constructor)]
    fn new(info: JsValue) -> PyError;
}

pub fn py_err_to_js_err(vm: &VirtualMachine, py_err: &PyBaseExceptionRef) -> JsValue {
    let jserr = vm.try_class("_js", "JSError").ok();
    let js_arg = if jserr.map_or(false, |jserr| py_err.isinstance(&jserr)) {
        py_err.get_arg(0)
    } else {
        None
    };
    let js_arg = js_arg
        .as_ref()
        .and_then(|x| x.payload::<js_module::PyJsValue>());
    match js_arg {
        Some(val) => val.value.clone(),
        None => {
            let res =
                serde_wasm_bindgen::to_value(&exceptions::SerializeException::new(vm, py_err));
            match res {
                Ok(err_info) => PyError::new(err_info).into(),
                Err(e) => e.into(),
            }
        }
    }
}

pub fn js_py_typeerror(vm: &VirtualMachine, js_err: JsValue) -> PyBaseExceptionRef {
    let msg = js_err.unchecked_into::<js_sys::Error>().to_string();
    vm.new_type_error(msg.into())
}

pub fn js_err_to_py_err(vm: &VirtualMachine, js_err: &JsValue) -> PyBaseExceptionRef {
    match js_err.dyn_ref::<js_sys::Error>() {
        Some(err) => {
            let exc_type = match String::from(err.name()).as_str() {
                "TypeError" => &vm.ctx.exceptions.type_error,
                "ReferenceError" => &vm.ctx.exceptions.name_error,
                "SyntaxError" => &vm.ctx.exceptions.syntax_error,
                _ => &vm.ctx.exceptions.exception_type,
            }
            .clone();
            vm.new_exception_msg(exc_type, err.message().into())
        }
        None => vm.new_exception_msg(
            vm.ctx.exceptions.exception_type.clone(),
            format!("{:?}", js_err),
        ),
    }
}

pub fn py_to_js(vm: &VirtualMachine, py_obj: PyObjectRef) -> JsValue {
    if let Some(ref wasm_id) = vm.wasm_id {
        if py_obj.isinstance(&vm.ctx.types.function_type) {
            let wasm_vm = WASMVirtualMachine {
                id: wasm_id.clone(),
            };
            let weak_py_obj = wasm_vm.push_held_rc(py_obj).unwrap();

            let closure = move |args: Option<Box<[JsValue]>>,
                                kwargs: Option<Object>|
                  -> Result<JsValue, JsValue> {
                let py_obj = match wasm_vm.assert_valid() {
                    Ok(_) => weak_py_obj
                        .upgrade()
                        .expect("weak_py_obj to be valid if VM is valid"),
                    Err(err) => {
                        return Err(err);
                    }
                };
                stored_vm_from_wasm(&wasm_vm).interp.enter(move |vm| {
                    let args = match args {
                        Some(args) => Vec::from(args)
                            .into_iter()
                            .map(|arg| js_to_py(vm, arg))
                            .collect::<Vec<_>>(),
                        None => Vec::new(),
                    };
                    let mut py_func_args = FuncArgs::from(args);
                    if let Some(ref kwargs) = kwargs {
                        for pair in object_entries(kwargs) {
                            let (key, val) = pair?;
                            py_func_args
                                .kwargs
                                .insert(js_sys::JsString::from(key).into(), js_to_py(vm, val));
                        }
                    }
                    let result = vm.invoke(&py_obj, py_func_args);
                    pyresult_to_jsresult(vm, result)
                })
            };
            let closure = Closure::wrap(Box::new(closure)
                as Box<
                    dyn FnMut(Option<Box<[JsValue]>>, Option<Object>) -> Result<JsValue, JsValue>,
                >);
            let func = closure.as_ref().clone();

            // stores pretty much nothing, it's fine to leak this because if it gets dropped
            // the error message is worse
            closure.forget();

            return func;
        }
    }
    // the browser module might not be injected
    if vm.try_class("_js", "Promise").is_ok() {
        if let Some(py_prom) = py_obj.payload::<js_module::PyPromise>() {
            return py_prom.as_js(vm).into();
        }
    }

    if let Ok(bytes) = PyBytesLike::try_from_object(vm, py_obj.clone()) {
        bytes.with_ref(|bytes| unsafe {
            // `Uint8Array::view` is an `unsafe fn` because it provides
            // a direct view into the WASM linear memory; if you were to allocate
            // something with Rust that view would probably become invalid. It's safe
            // because we then copy the array using `Uint8Array::slice`.
            let view = Uint8Array::view(bytes);
            view.slice(0, bytes.len() as u32).into()
        })
    } else {
        py_serde::serialize(vm, &py_obj, &serde_wasm_bindgen::Serializer::new())
            .unwrap_or(JsValue::UNDEFINED)
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
                return js_module::PyPromise::new(promise.clone())
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
            let mut vec = vec![0; u8_array.length() as usize];
            u8_array.copy_to(&mut vec);
            vm.ctx.new_bytes(vec)
        } else {
            let dict = vm.ctx.new_dict();
            for pair in object_entries(&Object::from(js_val)) {
                let (key, val) = pair.expect("iteration over object to not fail");
                let py_val = js_to_py(vm, val);
                dict.set_item(
                    String::from(js_sys::JsString::from(key)).as_str(),
                    py_val,
                    vm,
                )
                .unwrap();
            }
            dict.into_object()
        }
    } else if js_val.is_function() {
        let func = js_sys::Function::from(js_val);
        vm.ctx.new_method(
            func.name(),
            move |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                let this = Object::new();
                for (k, v) in args.kwargs {
                    Reflect::set(&this, &k.into(), &py_to_js(vm, v))
                        .expect("property to be settable");
                }
                let js_args = args
                    .args
                    .into_iter()
                    .map(|v| py_to_js(vm, v))
                    .collect::<Array>();
                func.apply(&this, &js_args)
                    .map(|val| js_to_py(vm, val))
                    .map_err(|err| js_err_to_py_err(vm, &err))
            },
        )
    } else if let Some(err) = js_val.dyn_ref::<js_sys::Error>() {
        js_err_to_py_err(vm, err).into_object()
    } else if js_val.is_undefined() {
        // Because `JSON.stringify(undefined)` returns undefined
        vm.ctx.none()
    } else {
        py_serde::deserialize(vm, serde_wasm_bindgen::Deserializer::from(js_val))
            .unwrap_or_else(|_| vm.ctx.none())
    }
}

pub fn syntax_err(err: CompileError) -> SyntaxError {
    let js_err = SyntaxError::new(&format!("Error parsing Python code: {}", err));
    let _ = Reflect::set(&js_err, &"row".into(), &(err.location.row() as u32).into());
    let _ = Reflect::set(
        &js_err,
        &"col".into(),
        &(err.location.column() as u32).into(),
    );
    let can_continue = matches!(&err.error, CompileErrorType::Parse(ParseErrorType::Eof));
    let _ = Reflect::set(&js_err, &"canContinue".into(), &can_continue.into());
    js_err
}

pub trait PyResultExt<T> {
    fn into_js(self, vm: &VirtualMachine) -> Result<T, JsValue>;
}
impl<T> PyResultExt<T> for PyResult<T> {
    fn into_js(self, vm: &VirtualMachine) -> Result<T, JsValue> {
        self.map_err(|err| py_err_to_js_err(vm, &err))
    }
}

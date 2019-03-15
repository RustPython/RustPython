use crate::{convert, vm_class::AccessibleVM, wasm_builtins::window};
use futures::Future;
use js_sys::Promise;
use num_traits::cast::ToPrimitive;
use rustpython_vm::obj::{objint, objstr};
use rustpython_vm::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectRef, PyResult, PyValue,
    TypeProtocol,
};
use rustpython_vm::{import::import_module, VirtualMachine};
use std::path::PathBuf;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, JsFuture};

enum FetchResponseFormat {
    Json,
    Text,
    ArrayBuffer,
}

impl FetchResponseFormat {
    fn from_str(vm: &mut VirtualMachine, s: &str) -> Result<Self, PyObjectRef> {
        match s {
            "json" => Ok(FetchResponseFormat::Json),
            "text" => Ok(FetchResponseFormat::Text),
            "array_buffer" => Ok(FetchResponseFormat::ArrayBuffer),
            _ => Err(vm.new_type_error("Unkown fetch response_format".into())),
        }
    }
    fn get_response(&self, response: &web_sys::Response) -> Result<Promise, JsValue> {
        match self {
            FetchResponseFormat::Json => response.json(),
            FetchResponseFormat::Text => response.text(),
            FetchResponseFormat::ArrayBuffer => response.array_buffer(),
        }
    }
}

fn browser_fetch(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(url, Some(vm.ctx.str_type()))]);

    let promise_type = import_promise_type(vm)?;

    let response_format =
        args.get_optional_kwarg_with_type("response_format", vm.ctx.str_type(), vm)?;
    let method = args.get_optional_kwarg_with_type("method", vm.ctx.str_type(), vm)?;
    let headers = args.get_optional_kwarg_with_type("headers", vm.ctx.dict_type(), vm)?;
    let body = args.get_optional_kwarg("body");
    let content_type = args.get_optional_kwarg_with_type("content_type", vm.ctx.str_type(), vm)?;

    let response_format = match response_format {
        Some(s) => FetchResponseFormat::from_str(vm, &objstr::get_value(&s))?,
        None => FetchResponseFormat::Text,
    };

    let mut opts = web_sys::RequestInit::new();

    match method {
        Some(s) => opts.method(&objstr::get_value(&s)),
        None => opts.method("GET"),
    };

    if let Some(body) = body {
        opts.body(Some(&convert::py_to_js(vm, body)));
    }

    let request = web_sys::Request::new_with_str_and_init(&objstr::get_value(url), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    if let Some(headers) = headers {
        let h = request.headers();
        for (key, value) in rustpython_vm::obj::objdict::get_key_value_pairs(&headers) {
            let ref key = vm.to_str(&key)?.value;
            let ref value = vm.to_str(&value)?.value;
            h.set(key, value)
                .map_err(|err| convert::js_py_typeerror(vm, err))?;
        }
    }

    if let Some(content_type) = content_type {
        request
            .headers()
            .set("Content-Type", &objstr::get_value(&content_type))
            .map_err(|err| convert::js_py_typeerror(vm, err))?;
    }

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = JsFuture::from(request_prom)
        .and_then(move |val| {
            let response = val
                .dyn_into::<web_sys::Response>()
                .expect("val to be of type Response");
            response_format.get_response(&response)
        })
        .and_then(JsFuture::from);

    Ok(PyPromise::new_obj(promise_type, future_to_promise(future)))
}

fn browser_request_animation_frame(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(func, Some(vm.ctx.function_type()))]);

    use std::{cell::RefCell, rc::Rc};

    // this basic setup for request_animation_frame taken from:
    // https://rustwasm.github.io/wasm-bindgen/examples/request-animation-frame.html

    let f = Rc::new(RefCell::new(None));
    let g = f.clone();

    let func = func.clone();

    let acc_vm = AccessibleVM::from_vm(vm);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move |time: f64| {
        let vm = &mut acc_vm
            .upgrade()
            .expect("that the vm is valid from inside of request_animation_frame");
        let func = func.clone();
        let args = vec![vm.ctx.new_float(time)];
        let _ = vm.invoke(func, args);

        let closure = f.borrow_mut().take();
        drop(closure);
    }) as Box<Fn(f64)>));

    let id = window()
        .request_animation_frame(&js_sys::Function::from(
            g.borrow().as_ref().unwrap().as_ref().clone(),
        ))
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.ctx.new_int(id))
}

fn browser_cancel_animation_frame(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(id, Some(vm.ctx.int_type()))]);

    let id = objint::get_value(id).to_i32().ok_or_else(|| {
        vm.new_exception(
            vm.ctx.exceptions.value_error.clone(),
            "Integer too large to convert to i32 for animationFrame id".into(),
        )
    })?;

    window()
        .cancel_animation_frame(id)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.get_none())
}

#[derive(Debug)]
pub struct PyPromise {
    value: Promise,
}

impl PyValue for PyPromise {
    fn class(_vm: &mut VirtualMachine) -> PyObjectRef {
        // TODO
        unimplemented!()
    }
}

impl PyPromise {
    pub fn new_obj(promise_type: PyObjectRef, value: Promise) -> PyObjectRef {
        PyObject::new(PyPromise { value }, promise_type)
    }
}

pub fn get_promise_value(obj: &PyObjectRef) -> Promise {
    if let Some(promise) = obj.payload::<PyPromise>() {
        return promise.value.clone();
    }
    panic!("Inner error getting promise")
}

pub fn import_promise_type(vm: &mut VirtualMachine) -> PyResult {
    match import_module(vm, PathBuf::default(), BROWSER_NAME)?.get_attr("Promise".into()) {
        Some(promise) => Ok(promise),
        None => Err(vm.new_not_implemented_error("No Promise".to_string())),
    }
}

fn promise_then(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let promise_type = import_promise_type(vm)?;
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(promise_type.clone())),
            (on_fulfill, Some(vm.ctx.function_type()))
        ],
        optional = [(on_reject, Some(vm.ctx.function_type()))]
    );

    let on_fulfill = on_fulfill.clone();
    let on_reject = on_reject.cloned();

    let acc_vm = AccessibleVM::from_vm(vm);

    let promise = get_promise_value(zelf);

    let ret_future = JsFuture::from(promise).then(move |res| {
        let vm = &mut acc_vm
            .upgrade()
            .expect("that the vm is valid when the promise resolves");
        let ret = match res {
            Ok(val) => {
                let val = convert::js_to_py(vm, val);
                vm.invoke(on_fulfill, PyFuncArgs::new(vec![val], vec![]))
            }
            Err(err) => {
                if let Some(on_reject) = on_reject {
                    let err = convert::js_to_py(vm, err);
                    vm.invoke(on_reject, PyFuncArgs::new(vec![err], vec![]))
                } else {
                    return Err(err);
                }
            }
        };
        convert::pyresult_to_jsresult(vm, ret)
    });

    let ret_promise = future_to_promise(ret_future);

    Ok(PyPromise::new_obj(promise_type, ret_promise))
}

fn promise_catch(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let promise_type = import_promise_type(vm)?;
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(promise_type.clone())),
            (on_reject, Some(vm.ctx.function_type()))
        ]
    );

    let on_reject = on_reject.clone();

    let acc_vm = AccessibleVM::from_vm(vm);

    let promise = get_promise_value(zelf);

    let ret_future = JsFuture::from(promise).then(move |res| match res {
        Ok(val) => Ok(val),
        Err(err) => {
            let vm = &mut acc_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            let err = convert::js_to_py(vm, err);
            let res = vm.invoke(on_reject, PyFuncArgs::new(vec![err], vec![]));
            convert::pyresult_to_jsresult(vm, res)
        }
    });

    let ret_promise = future_to_promise(ret_future);

    Ok(PyPromise::new_obj(promise_type, ret_promise))
}

fn browser_alert(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(message, Some(vm.ctx.str_type()))]);

    window()
        .alert_with_message(&objstr::get_value(message))
        .expect("alert() not to fail");

    Ok(vm.get_none())
}

fn browser_confirm(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(message, Some(vm.ctx.str_type()))]);

    let result = window()
        .confirm_with_message(&objstr::get_value(message))
        .expect("confirm() not to fail");

    Ok(vm.new_bool(result))
}

fn browser_prompt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(message, Some(vm.ctx.str_type()))],
        optional = [(default, Some(vm.ctx.str_type()))]
    );

    let result = if let Some(default) = default {
        window().prompt_with_message_and_default(
            &objstr::get_value(message),
            &objstr::get_value(default),
        )
    } else {
        window().prompt_with_message(&objstr::get_value(message))
    };

    let result = match result.expect("prompt() not to fail") {
        Some(result) => vm.new_str(result),
        None => vm.get_none(),
    };

    Ok(result)
}

const BROWSER_NAME: &str = "browser";

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let promise = py_class!(ctx, "Promise", ctx.object(), {
        "then" => ctx.new_rustfunc(promise_then),
        "catch" => ctx.new_rustfunc(promise_catch)
    });

    py_module!(ctx, BROWSER_NAME, {
        "fetch" => ctx.new_rustfunc(browser_fetch),
        "request_animation_frame" => ctx.new_rustfunc(browser_request_animation_frame),
        "cancel_animation_frame" => ctx.new_rustfunc(browser_cancel_animation_frame),
        "Promise" => promise,
        "alert" => ctx.new_rustfunc(browser_alert),
        "confirm" => ctx.new_rustfunc(browser_confirm),
        "prompt" => ctx.new_rustfunc(browser_prompt),
    })
}

pub fn setup_browser_module(vm: &mut VirtualMachine) {
    vm.stdlib_inits
        .insert(BROWSER_NAME.to_string(), Box::new(make_module));
}

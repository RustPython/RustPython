use crate::{convert, vm_class::AccessibleVM, wasm_builtins::window};
use futures::Future;
use js_sys::Promise;
use rustpython_vm::obj::{objint, objstr};
use rustpython_vm::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use rustpython_vm::{import::import, VirtualMachine};
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
            let key = objstr::get_value(&vm.to_str(&key)?);
            let value = objstr::get_value(&vm.to_str(&value)?);
            h.set(&key, &value)
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

    Ok(PyPromise::new(promise_type, future_to_promise(future)))
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
        let args = PyFuncArgs {
            args: vec![vm.ctx.new_float(time)],
            kwargs: vec![],
        };
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

    // questionable, but it's probably fine
    let id = objint::get_value(id)
        .to_string()
        .parse()
        .expect("bigint.to_string() to be parsable as i32");

    window()
        .cancel_animation_frame(id)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.get_none())
}

pub struct PyPromise {
    value: Promise,
}

impl PyPromise {
    pub fn new(promise_type: PyObjectRef, value: Promise) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(PyPromise { value }),
            },
            promise_type,
        )
    }
}

pub fn get_promise_value(obj: &PyObjectRef) -> Promise {
    if let PyObjectPayload::AnyRustValue { value } = &obj.payload {
        if let Some(promise) = value.downcast_ref::<PyPromise>() {
            return promise.value.clone();
        }
    }
    panic!("Inner error getting promise")
}

pub fn import_promise_type(vm: &mut VirtualMachine) -> PyResult {
    import(
        vm,
        PathBuf::default(),
        BROWSER_NAME,
        &Some("Promise".into()),
    )
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
        ret.map(|val| convert::py_to_js(vm, val))
            .map_err(|err| convert::py_to_js(vm, err))
    });

    let ret_promise = future_to_promise(ret_future);

    Ok(PyPromise::new(promise_type, ret_promise))
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
            vm.invoke(on_reject, PyFuncArgs::new(vec![err], vec![]))
                .map(|val| convert::py_to_js(vm, val))
                .map_err(|err| convert::py_to_js(vm, err))
        }
    });

    let ret_promise = future_to_promise(ret_future);

    Ok(PyPromise::new(promise_type, ret_promise))
}

const BROWSER_NAME: &str = "browser";

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let promise = {
        let promise = ctx.new_class("Promise", ctx.object());
        ctx.set_attr(&promise, "then", ctx.new_rustfunc(promise_then));
        ctx.set_attr(&promise, "catch", ctx.new_rustfunc(promise_catch));
        promise
    };

    py_module!(ctx, BROWSER_NAME, {
        "fetch" => ctx.new_rustfunc(browser_fetch),
        "request_animation_frame" => ctx.new_rustfunc(browser_request_animation_frame),
        "cancel_animation_frame" => ctx.new_rustfunc(browser_cancel_animation_frame),
        "Promise" => promise,
    })
}

pub fn setup_browser_module(vm: &mut VirtualMachine) {
    vm.stdlib_inits
        .insert(BROWSER_NAME.to_string(), Box::new(mk_module));
}

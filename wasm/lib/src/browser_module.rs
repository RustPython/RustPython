use futures::Future;
use js_sys::Promise;
use num_traits::cast::ToPrimitive;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{future_to_promise, JsFuture};

use rustpython_vm::function::{OptionalArg, PyFuncArgs};
use rustpython_vm::import::import_file;
use rustpython_vm::obj::{
    objdict::PyDictRef, objint::PyIntRef, objstr::PyStringRef, objtype::PyClassRef,
};
use rustpython_vm::pyobject::{
    PyCallable, PyClassImpl, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
};
use rustpython_vm::VirtualMachine;

use crate::{convert, vm_class::weak_vm, wasm_builtins::window};

enum FetchResponseFormat {
    Json,
    Text,
    ArrayBuffer,
}

impl FetchResponseFormat {
    fn from_str(vm: &VirtualMachine, s: &str) -> PyResult<Self> {
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

#[derive(FromArgs)]
struct FetchArgs {
    #[pyarg(keyword_only, default = "None")]
    response_format: Option<PyStringRef>,
    #[pyarg(keyword_only, default = "None")]
    method: Option<PyStringRef>,
    #[pyarg(keyword_only, default = "None")]
    headers: Option<PyDictRef>,
    #[pyarg(keyword_only, default = "None")]
    body: Option<PyObjectRef>,
    #[pyarg(keyword_only, default = "None")]
    content_type: Option<PyStringRef>,
}

fn browser_fetch(url: PyStringRef, args: FetchArgs, vm: &VirtualMachine) -> PyResult {
    let FetchArgs {
        response_format,
        method,
        headers,
        body,
        content_type,
    } = args;

    let response_format = match response_format {
        Some(s) => FetchResponseFormat::from_str(vm, s.as_str())?,
        None => FetchResponseFormat::Text,
    };

    let mut opts = web_sys::RequestInit::new();

    match method {
        Some(s) => opts.method(s.as_str()),
        None => opts.method("GET"),
    };

    if let Some(body) = body {
        opts.body(Some(&convert::py_to_js(vm, body)));
    }

    let request = web_sys::Request::new_with_str_and_init(url.as_str(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    if let Some(headers) = headers {
        let h = request.headers();
        for (key, value) in headers {
            let key = vm.to_str(&key)?;
            let value = vm.to_str(&value)?;
            h.set(key.as_str(), value.as_str())
                .map_err(|err| convert::js_py_typeerror(vm, err))?;
        }
    }

    if let Some(content_type) = content_type {
        request
            .headers()
            .set("Content-Type", content_type.as_str())
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

    Ok(PyPromise::from_future(future).into_ref(vm).into_object())
}

fn browser_request_animation_frame(func: PyCallable, vm: &VirtualMachine) -> PyResult {
    use std::{cell::RefCell, rc::Rc};

    // this basic setup for request_animation_frame taken from:
    // https://rustwasm.github.io/wasm-bindgen/examples/request-animation-frame.html

    let f = Rc::new(RefCell::new(None));
    let g = f.clone();

    let weak_vm = weak_vm(vm);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move |time: f64| {
        let stored_vm = weak_vm
            .upgrade()
            .expect("that the vm is valid from inside of request_animation_frame");
        let vm = &stored_vm.vm;
        let func = func.clone();
        let args = vec![vm.ctx.new_float(time)];
        let _ = vm.invoke(&func.into_object(), args);

        let closure = f.borrow_mut().take();
        drop(closure);
    }) as Box<dyn Fn(f64)>));

    let id = window()
        .request_animation_frame(&js_sys::Function::from(
            g.borrow().as_ref().unwrap().as_ref().clone(),
        ))
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.ctx.new_int(id))
}

fn browser_cancel_animation_frame(id: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let id = id.as_bigint().to_i32().ok_or_else(|| {
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

#[pyclass(name = "Promise")]
#[derive(Debug)]
pub struct PyPromise {
    value: Promise,
}
pub type PyPromiseRef = PyRef<PyPromise>;

impl PyValue for PyPromise {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("browser", "Promise")
    }
}

#[pyimpl]
impl PyPromise {
    pub fn new(value: Promise) -> PyPromise {
        PyPromise { value }
    }
    pub fn from_future<F>(future: F) -> PyPromise
    where
        F: Future<Item = JsValue, Error = JsValue> + 'static,
    {
        PyPromise::new(future_to_promise(future))
    }
    pub fn value(&self) -> Promise {
        self.value.clone()
    }

    #[pymethod]
    fn then(
        &self,
        on_fulfill: PyCallable,
        on_reject: OptionalArg<PyCallable>,
        vm: &VirtualMachine,
    ) -> PyPromiseRef {
        let weak_vm = weak_vm(vm);

        let ret_future = JsFuture::from(self.value.clone()).then(move |res| {
            let stored_vm = &weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            let vm = &stored_vm.vm;
            let ret = match res {
                Ok(val) => {
                    let args = if val.is_null() {
                        vec![]
                    } else {
                        vec![convert::js_to_py(vm, val)]
                    };
                    vm.invoke(&on_fulfill.into_object(), PyFuncArgs::new(args, vec![]))
                }
                Err(err) => {
                    if let OptionalArg::Present(on_reject) = on_reject {
                        let err = convert::js_to_py(vm, err);
                        vm.invoke(&on_reject.into_object(), PyFuncArgs::new(vec![err], vec![]))
                    } else {
                        return Err(err);
                    }
                }
            };
            convert::pyresult_to_jsresult(vm, ret)
        });

        PyPromise::from_future(ret_future).into_ref(vm)
    }

    #[pymethod]
    fn catch(&self, on_reject: PyCallable, vm: &VirtualMachine) -> PyPromiseRef {
        let weak_vm = weak_vm(vm);

        let ret_future = JsFuture::from(self.value.clone()).then(move |res| {
            res.or_else(|err| {
                let stored_vm = weak_vm
                    .upgrade()
                    .expect("that the vm is valid when the promise resolves");
                let vm = &stored_vm.vm;
                let err = convert::js_to_py(vm, err);
                let res = vm.invoke(&on_reject.into_object(), PyFuncArgs::new(vec![err], vec![]));
                convert::pyresult_to_jsresult(vm, res)
            })
        });

        PyPromise::from_future(ret_future).into_ref(vm)
    }
}

#[pyclass]
#[derive(Debug)]
struct Document {
    doc: web_sys::Document,
}

impl PyValue for Document {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("browser", "Document")
    }
}

#[pyimpl]
impl Document {
    #[pymethod]
    fn query(&self, query: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let elem = self
            .doc
            .query_selector(query.as_str())
            .map_err(|err| convert::js_py_typeerror(vm, err))?;
        let elem = match elem {
            Some(elem) => Element { elem }.into_ref(vm).into_object(),
            None => vm.get_none(),
        };
        Ok(elem)
    }
}

#[pyclass]
#[derive(Debug)]
struct Element {
    elem: web_sys::Element,
}

impl PyValue for Element {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("browser", "Element")
    }
}

#[pyimpl]
impl Element {
    #[pymethod]
    fn get_attr(
        &self,
        attr: PyStringRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        match self.elem.get_attribute(attr.as_str()) {
            Some(s) => vm.new_str(s),
            None => default.into_option().unwrap_or_else(|| vm.get_none()),
        }
    }

    #[pymethod]
    fn set_attr(&self, attr: PyStringRef, value: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        self.elem
            .set_attribute(attr.as_str(), value.as_str())
            .map_err(|err| convert::js_to_py(vm, err))
    }
}

fn browser_load_module(module: PyStringRef, path: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let weak_vm = weak_vm(vm);

    let mut opts = web_sys::RequestInit::new();
    opts.method("GET");

    let request = web_sys::Request::new_with_str_and_init(path.as_str(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = JsFuture::from(request_prom)
        .and_then(move |val| {
            let response = val
                .dyn_into::<web_sys::Response>()
                .expect("val to be of type Response");
            response.text()
        })
        .and_then(JsFuture::from)
        .and_then(move |text| {
            let stored_vm = &weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            let vm = &stored_vm.vm;
            let resp_text = text.as_string().unwrap();
            let res = import_file(vm, module.as_str(), "WEB".to_string(), resp_text);
            match res {
                Ok(_) => Ok(JsValue::null()),
                Err(err) => Err(convert::py_err_to_js_err(vm, &err)),
            }
        });

    Ok(PyPromise::from_future(future).into_ref(vm).into_object())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let promise = PyPromise::make_class(ctx);

    let document_class = Document::make_class(ctx);

    let document = PyObject::new(
        Document {
            doc: window().document().expect("Document missing from window"),
        },
        document_class.clone(),
        None,
    );

    let element = Element::make_class(ctx);

    py_module!(vm, "browser", {
        "fetch" => ctx.new_rustfunc(browser_fetch),
        "request_animation_frame" => ctx.new_rustfunc(browser_request_animation_frame),
        "cancel_animation_frame" => ctx.new_rustfunc(browser_cancel_animation_frame),
        "Promise" => promise,
        "Document" => document_class,
        "document" => document,
        "Element" => element,
        "load_module" => ctx.new_rustfunc(browser_load_module),
    })
}

pub fn setup_browser_module(vm: &VirtualMachine) {
    vm.stdlib_inits
        .borrow_mut()
        .insert("_browser".to_string(), Box::new(make_module));
    vm.frozen.borrow_mut().extend(py_compile_bytecode!(
        file = "src/browser.py",
        module_name = "browser",
    ));
}

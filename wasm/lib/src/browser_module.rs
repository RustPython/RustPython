use js_sys::Promise;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use rustpython_vm::builtins::{PyDictRef, PyStrRef, PyTypeRef};
use rustpython_vm::function::OptionalArg;
use rustpython_vm::import::import_file;
use rustpython_vm::pyobject::{
    BorrowValue, IntoPyObject, PyCallable, PyClassImpl, PyObject, PyObjectRef, PyResult, PyValue,
    StaticType,
};
use rustpython_vm::VirtualMachine;

use crate::{convert, js_module::PyPromise, vm_class::weak_vm, wasm_builtins::window};

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
    #[pyarg(named, default)]
    response_format: Option<PyStrRef>,
    #[pyarg(named, default)]
    method: Option<PyStrRef>,
    #[pyarg(named, default)]
    headers: Option<PyDictRef>,
    #[pyarg(named, default)]
    body: Option<PyObjectRef>,
    #[pyarg(named, default)]
    content_type: Option<PyStrRef>,
}

fn browser_fetch(url: PyStrRef, args: FetchArgs, vm: &VirtualMachine) -> PyResult {
    let FetchArgs {
        response_format,
        method,
        headers,
        body,
        content_type,
    } = args;

    let response_format = match response_format {
        Some(s) => FetchResponseFormat::from_str(vm, s.borrow_value())?,
        None => FetchResponseFormat::Text,
    };

    let mut opts = web_sys::RequestInit::new();

    match method {
        Some(s) => opts.method(s.borrow_value()),
        None => opts.method("GET"),
    };

    if let Some(body) = body {
        opts.body(Some(&convert::py_to_js(vm, body)));
    }

    let request = web_sys::Request::new_with_str_and_init(url.borrow_value(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    if let Some(headers) = headers {
        let h = request.headers();
        for (key, value) in headers {
            let key = vm.to_str(&key)?;
            let value = vm.to_str(&value)?;
            h.set(key.borrow_value(), value.borrow_value())
                .map_err(|err| convert::js_py_typeerror(vm, err))?;
        }
    }

    if let Some(content_type) = content_type {
        request
            .headers()
            .set("Content-Type", content_type.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))?;
    }

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = async move {
        let val = JsFuture::from(request_prom).await?;
        let response = val
            .dyn_into::<web_sys::Response>()
            .expect("val to be of type Response");
        JsFuture::from(response_format.get_response(&response)?).await
    };

    Ok(PyPromise::from_future(future).into_object(vm))
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
        stored_vm.interp.enter(|vm| {
            let func = func.clone();
            let args = vec![vm.ctx.new_float(time)];
            let _ = vm.invoke(&func.into_object(), args);

            let closure = f.borrow_mut().take();
            drop(closure);
        })
    }) as Box<dyn Fn(f64)>));

    let id = window()
        .request_animation_frame(&js_sys::Function::from(
            g.borrow().as_ref().unwrap().as_ref().clone(),
        ))
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(vm.ctx.new_int(id))
}

fn browser_cancel_animation_frame(id: i32, vm: &VirtualMachine) -> PyResult<()> {
    window()
        .cancel_animation_frame(id)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    Ok(())
}

#[pyclass(module = "browser", name)]
#[derive(Debug)]
struct Document {
    doc: web_sys::Document,
}

impl PyValue for Document {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl Document {
    #[pymethod]
    fn query(&self, query: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let elem = self
            .doc
            .query_selector(query.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))?
            .map(|elem| Element { elem })
            .into_pyobject(vm);
        Ok(elem)
    }
}

#[pyclass(module = "browser", name)]
#[derive(Debug)]
struct Element {
    elem: web_sys::Element,
}

impl PyValue for Element {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl Element {
    #[pymethod]
    fn get_attr(
        &self,
        attr: PyStrRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        match self.elem.get_attribute(attr.borrow_value()) {
            Some(s) => vm.ctx.new_str(s),
            None => default.unwrap_or_none(vm),
        }
    }

    #[pymethod]
    fn set_attr(&self, attr: PyStrRef, value: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        self.elem
            .set_attribute(attr.borrow_value(), value.borrow_value())
            .map_err(|err| convert::js_py_typeerror(vm, err))
    }
}

fn browser_load_module(module: PyStrRef, path: PyStrRef, vm: &VirtualMachine) -> PyResult {
    let weak_vm = weak_vm(vm);

    let mut opts = web_sys::RequestInit::new();
    opts.method("GET");

    let request = web_sys::Request::new_with_str_and_init(path.borrow_value(), &opts)
        .map_err(|err| convert::js_py_typeerror(vm, err))?;

    let window = window();
    let request_prom = window.fetch_with_request(&request);

    let future = async move {
        let val = JsFuture::from(request_prom).await?;
        let response = val
            .dyn_into::<web_sys::Response>()
            .expect("val to be of type Response");
        let text = JsFuture::from(response.text()?).await?;
        let stored_vm = &weak_vm
            .upgrade()
            .expect("that the vm is valid when the promise resolves");
        stored_vm.interp.enter(move |vm| {
            let resp_text = text.as_string().unwrap();
            let res = import_file(vm, module.borrow_value(), "WEB".to_owned(), resp_text);
            match res {
                Ok(_) => Ok(JsValue::null()),
                Err(err) => Err(convert::py_err_to_js_err(vm, &err)),
            }
        })
    };

    Ok(PyPromise::from_future(future).into_object(vm))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let document_class = Document::make_class(ctx);

    let document = PyObject::new(
        Document {
            doc: window().document().expect("Document missing from window"),
        },
        document_class.clone(),
        None,
    );

    let element = Element::make_class(ctx);

    py_module!(vm, "_browser", {
        "fetch" => named_function!(ctx, browser, fetch),
        "request_animation_frame" => named_function!(ctx, browser, request_animation_frame),
        "cancel_animation_frame" => named_function!(ctx, browser, cancel_animation_frame),
        "Document" => document_class,
        "document" => document,
        "Element" => element,
        "load_module" => named_function!(ctx, browser, load_module),
    })
}

pub fn setup_browser_module(vm: &mut VirtualMachine) {
    vm.add_native_module("_browser".to_owned(), Box::new(make_module));
    vm.add_frozen(py_freeze!(dir = "Lib"));
}

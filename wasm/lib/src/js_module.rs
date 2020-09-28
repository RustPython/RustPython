use crate::convert;
use crate::vm_class::{stored_vm_from_wasm, WASMVirtualMachine};
use crate::weak_vm;
use js_sys::{Array, Object, Promise, Reflect};
use std::{cell, fmt, future};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, JsFuture};

use rustpython_vm::builtins::{PyFloatRef, PyStrRef, PyTypeRef};
use rustpython_vm::common::rc::PyRc;
use rustpython_vm::exceptions::PyBaseExceptionRef;
use rustpython_vm::function::{Args, OptionalArg};
use rustpython_vm::pyobject::{
    BorrowValue, IntoPyObject, PyCallable, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue,
    StaticType, TryFromObject,
};
use rustpython_vm::types::create_simple_type;
use rustpython_vm::VirtualMachine;

#[wasm_bindgen(inline_js = "
export function has_prop(target, prop) { return prop in Object(target); }
export function get_prop(target, prop) { return target[prop]; }
export function set_prop(target, prop, value) { target[prop] = value; }
export function type_of(a) { return typeof a; }
export function instance_of(lhs, rhs) { return lhs instanceof rhs; }
export function call_func(func, args) { return func(...args); }
export function call_method(obj, method, args) { return obj[method](...args) }
export function wrap_closure(closure) {
    return function pyfunction(...args) {
        closure(this, args)
    }
}
")]
extern "C" {
    #[wasm_bindgen(catch)]
    fn has_prop(target: &JsValue, prop: &JsValue) -> Result<bool, JsValue>;
    #[wasm_bindgen(catch)]
    fn get_prop(target: &JsValue, prop: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn set_prop(target: &JsValue, prop: &JsValue, value: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen]
    fn type_of(a: &JsValue) -> String;
    #[wasm_bindgen(catch)]
    fn instance_of(lhs: &JsValue, rhs: &JsValue) -> Result<bool, JsValue>;
    #[wasm_bindgen(catch)]
    fn call_func(func: &JsValue, args: &Array) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn call_method(obj: &JsValue, method: &JsValue, args: &Array) -> Result<JsValue, JsValue>;
    #[wasm_bindgen]
    fn wrap_closure(closure: &JsValue) -> JsValue;
}

#[pyclass(module = "_js", name = "JSValue")]
#[derive(Debug)]
pub struct PyJsValue {
    pub(crate) value: JsValue,
}
type PyJsValueRef = PyRef<PyJsValue>;

impl PyValue for PyJsValue {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl AsRef<JsValue> for PyJsValue {
    fn as_ref(&self) -> &JsValue {
        &self.value
    }
}

enum JsProperty {
    Str(PyStrRef),
    Js(PyJsValueRef),
}

impl TryFromObject for JsProperty {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyStrRef::try_from_object(vm, obj.clone())
            .map(JsProperty::Str)
            .or_else(|_| PyJsValueRef::try_from_object(vm, obj).map(JsProperty::Js))
    }
}

impl JsProperty {
    fn into_jsvalue(self) -> JsValue {
        match self {
            JsProperty::Str(s) => s.borrow_value().into(),
            JsProperty::Js(value) => value.value.clone(),
        }
    }
}

#[pyimpl]
impl PyJsValue {
    #[inline]
    pub fn new(value: impl Into<JsValue>) -> PyJsValue {
        PyJsValue {
            value: value.into(),
        }
    }

    #[pymethod]
    fn null(&self) -> PyJsValue {
        PyJsValue::new(JsValue::NULL)
    }

    #[pymethod]
    fn undefined(&self) -> PyJsValue {
        PyJsValue::new(JsValue::UNDEFINED)
    }

    #[pymethod]
    fn new_from_str(&self, s: PyStrRef) -> PyJsValue {
        PyJsValue::new(s.borrow_value())
    }

    #[pymethod]
    fn new_from_float(&self, n: PyFloatRef) -> PyJsValue {
        PyJsValue::new(n.to_f64())
    }

    #[pymethod]
    fn new_closure(&self, obj: PyObjectRef, vm: &VirtualMachine) -> JsClosure {
        JsClosure::new(obj, vm)
    }

    #[pymethod]
    fn new_object(&self, opts: NewObjectOptions, vm: &VirtualMachine) -> PyResult<PyJsValue> {
        let value = if let Some(proto) = opts.prototype {
            if let Some(proto) = proto.value.dyn_ref::<Object>() {
                Object::create(proto)
            } else if proto.value.is_null() {
                Object::create(proto.value.unchecked_ref())
            } else {
                return Err(vm.new_value_error("prototype must be an Object or null".to_owned()));
            }
        } else {
            Object::new()
        };
        Ok(PyJsValue::new(value))
    }

    #[pymethod]
    fn has_prop(&self, name: JsProperty, vm: &VirtualMachine) -> PyResult<bool> {
        has_prop(&self.value, &name.into_jsvalue()).map_err(|err| new_js_error(vm, err))
    }

    #[pymethod]
    fn get_prop(&self, name: JsProperty, vm: &VirtualMachine) -> PyResult<PyJsValue> {
        let name = &name.into_jsvalue();
        if has_prop(&self.value, name).map_err(|err| new_js_error(vm, err))? {
            get_prop(&self.value, name)
                .map(PyJsValue::new)
                .map_err(|err| new_js_error(vm, err))
        } else {
            Err(vm.new_attribute_error(format!("No attribute {:?} on JS value", name)))
        }
    }

    #[pymethod]
    fn set_prop(&self, name: JsProperty, value: PyJsValueRef, vm: &VirtualMachine) -> PyResult<()> {
        set_prop(&self.value, &name.into_jsvalue(), &value.value)
            .map_err(|err| new_js_error(vm, err))
    }

    #[pymethod]
    fn call(
        &self,
        args: Args<PyJsValueRef>,
        opts: CallOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyJsValue> {
        let func = self
            .value
            .dyn_ref::<js_sys::Function>()
            .ok_or_else(|| vm.new_type_error("JS value is not callable".to_owned()))?;
        let js_args = args.iter().map(|x| x.as_ref()).collect::<Array>();
        let res = match opts.this {
            Some(this) => Reflect::apply(func, &this.value, &js_args),
            None => call_func(func, &js_args),
        };
        res.map(PyJsValue::new).map_err(|err| new_js_error(vm, err))
    }

    #[pymethod]
    fn call_method(
        &self,
        name: JsProperty,
        args: Args<PyJsValueRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyJsValue> {
        let js_args = args.iter().map(|x| x.as_ref()).collect::<Array>();
        call_method(&self.value, &name.into_jsvalue(), &js_args)
            .map(PyJsValue::new)
            .map_err(|err| new_js_error(vm, err))
    }

    #[pymethod]
    fn construct(
        &self,
        args: Args<PyJsValueRef>,
        opts: NewObjectOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyJsValue> {
        let ctor = self
            .value
            .dyn_ref::<js_sys::Function>()
            .ok_or_else(|| vm.new_type_error("JS value is not callable".to_owned()))?;
        let proto = opts
            .prototype
            .as_ref()
            .and_then(|proto| proto.value.dyn_ref::<js_sys::Function>());
        let js_args = args.iter().map(|x| x.as_ref()).collect::<Array>();
        let constructed_result = if let Some(proto) = proto {
            Reflect::construct_with_new_target(ctor, &js_args, &proto)
        } else {
            Reflect::construct(ctor, &js_args)
        };

        constructed_result
            .map(PyJsValue::new)
            .map_err(|err| new_js_error(vm, err))
    }

    #[pymethod]
    fn as_str(&self) -> Option<String> {
        self.value.as_string()
    }

    #[pymethod]
    fn as_float(&self) -> Option<f64> {
        self.value.as_f64()
    }

    #[pymethod]
    fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    #[pymethod(name = "typeof")]
    fn type_of(&self) -> String {
        type_of(&self.value)
    }

    /// Checks that `typeof self == "object" && self !== null`. Use instead
    /// of `value.typeof() == "object"`
    #[pymethod]
    fn is_object(&self) -> bool {
        self.value.is_object()
    }

    #[pymethod]
    fn instanceof(&self, rhs: PyJsValueRef, vm: &VirtualMachine) -> PyResult<bool> {
        instance_of(&self.value, &rhs.value).map_err(|err| new_js_error(vm, err))
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> String {
        format!("{:?}", self.value)
    }
}

#[derive(FromArgs)]
struct CallOptions {
    #[pyarg(named, default)]
    this: Option<PyJsValueRef>,
}

#[derive(FromArgs)]
struct NewObjectOptions {
    #[pyarg(named, default)]
    prototype: Option<PyJsValueRef>,
}

type ClosureType = Closure<dyn Fn(JsValue, Box<[JsValue]>) -> Result<JsValue, JsValue>>;

#[pyclass(module = "_js", name = "JSClosure")]
struct JsClosure {
    closure: cell::RefCell<Option<(ClosureType, PyJsValueRef)>>,
    destroyed: cell::Cell<bool>,
    detached: cell::Cell<bool>,
}

impl fmt::Debug for JsClosure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("JsClosure")
    }
}

impl PyValue for JsClosure {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl JsClosure {
    fn new(obj: PyObjectRef, vm: &VirtualMachine) -> Self {
        let wasm_vm = WASMVirtualMachine {
            id: vm.wasm_id.clone().unwrap(),
        };
        let weak_py_obj = wasm_vm.push_held_rc(obj).unwrap();
        let f = move |this: JsValue, args: Box<[JsValue]>| {
            let py_obj = match wasm_vm.assert_valid() {
                Ok(_) => weak_py_obj
                    .upgrade()
                    .expect("weak_py_obj to be valid if VM is valid"),
                Err(err) => {
                    return Err(err);
                }
            };
            stored_vm_from_wasm(&wasm_vm).interp.enter(move |vm| {
                let mut pyargs = vec![PyJsValue::new(this).into_object(vm)];
                pyargs.extend(
                    Vec::from(args)
                        .into_iter()
                        .map(|arg| PyJsValue::new(arg).into_object(vm)),
                );
                let res = vm.invoke(&py_obj, pyargs);
                convert::pyresult_to_jsresult(vm, res)
            })
        };
        let closure = Closure::wrap(Box::new(f) as _);
        let wrapped = PyJsValue::new(wrap_closure(closure.as_ref())).into_ref(vm);
        JsClosure {
            closure: Some((closure, wrapped)).into(),
            destroyed: false.into(),
            detached: false.into(),
        }
    }

    #[pyproperty]
    fn value(&self) -> Option<PyJsValueRef> {
        self.closure
            .borrow()
            .as_ref()
            .map(|(_, jsval)| jsval.clone())
    }
    #[pyproperty]
    fn destroyed(&self) -> bool {
        self.destroyed.get()
    }
    #[pyproperty]
    fn detached(&self) -> bool {
        self.detached.get()
    }

    #[pymethod]
    fn destroy(&self, vm: &VirtualMachine) -> PyResult<()> {
        let (closure, _) = self.closure.replace(None).ok_or_else(|| {
            vm.new_value_error(
                "can't destroy closure has already been destroyed or detached".to_owned(),
            )
        })?;
        drop(closure);
        self.destroyed.set(true);
        Ok(())
    }
    #[pymethod]
    fn detach(&self, vm: &VirtualMachine) -> PyResult<PyJsValueRef> {
        let (closure, jsval) = self.closure.replace(None).ok_or_else(|| {
            vm.new_value_error(
                "can't detach closure has already been detached or destroyed".to_owned(),
            )
        })?;
        closure.forget();
        self.detached.set(true);
        Ok(jsval)
    }
}

#[pyclass(module = "browser", name = "Promise")]
#[derive(Debug)]
pub struct PyPromise {
    value: Promise,
}
pub type PyPromiseRef = PyRef<PyPromise>;

impl PyValue for PyPromise {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl]
impl PyPromise {
    pub fn new(value: Promise) -> PyPromise {
        PyPromise { value }
    }
    pub fn from_future<F>(future: F) -> PyPromise
    where
        F: future::Future<Output = Result<JsValue, JsValue>> + 'static,
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
        let prom = JsFuture::from(self.value.clone());

        let ret_future = async move {
            let stored_vm = &weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            let res = prom.await;
            match res {
                Ok(val) => stored_vm.interp.enter(move |vm| {
                    let args = if val.is_null() {
                        vec![]
                    } else {
                        vec![convert::js_to_py(vm, val)]
                    };
                    let res = vm.invoke(&on_fulfill.into_object(), args);
                    convert::pyresult_to_jsresult(vm, res)
                }),
                Err(err) => {
                    if let OptionalArg::Present(on_reject) = on_reject {
                        stored_vm.interp.enter(move |vm| {
                            let err = convert::js_to_py(vm, err);
                            let res = vm.invoke(&on_reject.into_object(), (err,));
                            convert::pyresult_to_jsresult(vm, res)
                        })
                    } else {
                        Err(err)
                    }
                }
            }
        };

        PyPromise::from_future(ret_future).into_ref(vm)
    }

    #[pymethod]
    fn catch(&self, on_reject: PyCallable, vm: &VirtualMachine) -> PyPromiseRef {
        let weak_vm = weak_vm(vm);
        let prom = JsFuture::from(self.value.clone());

        let ret_future = async move {
            let err = match prom.await {
                Ok(x) => return Ok(x),
                Err(e) => e,
            };
            let stored_vm = weak_vm
                .upgrade()
                .expect("that the vm is valid when the promise resolves");
            stored_vm.interp.enter(move |vm| {
                let err = convert::js_to_py(vm, err);
                let res = vm.invoke(&on_reject.into_object(), (err,));
                convert::pyresult_to_jsresult(vm, res)
            })
        };

        PyPromise::from_future(ret_future).into_ref(vm)
    }
}

fn new_js_error(vm: &VirtualMachine, err: JsValue) -> PyBaseExceptionRef {
    vm.new_exception(
        vm.class("_js", "JSError"),
        vec![PyJsValue::new(err).into_pyobject(vm)],
    )
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let js_error = create_simple_type("JSError", &ctx.exceptions.exception_type);
    extend_class!(ctx, &js_error, {
        "value" => ctx.new_readonly_getset("value", |exc: PyBaseExceptionRef| exc.get_arg(0)),
    });

    py_module!(vm, "_js", {
        "JSError" => js_error,
        "JSValue" => PyJsValue::make_class(ctx),
        "JSClosure" => JsClosure::make_class(ctx),
        "Promise" => PyPromise::make_class(ctx),
    })
}

pub fn setup_js_module(vm: &mut VirtualMachine) {
    let state = PyRc::get_mut(&mut vm.state).unwrap();
    state
        .stdlib_inits
        .insert("_js".to_owned(), Box::new(make_module));
}

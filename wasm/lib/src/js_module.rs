pub(crate) use _js::{PyJsValue, PyPromise};
use rustpython_vm::{
    builtins::{PyBaseExceptionRef, PyType},
    PyObjectRef, VirtualMachine,
};

#[pymodule]
mod _js {
    use crate::{
        convert,
        vm_class::{stored_vm_from_wasm, WASMVirtualMachine},
        weak_vm,
    };
    use js_sys::{Array, Object, Promise, Reflect};
    use rustpython_vm::{
        builtins::{PyBaseExceptionRef, PyFloat, PyStrRef, PyTypeRef},
        function::{ArgCallable, IntoPyObject, OptionalArg, OptionalOption, PosArgs},
        protocol::PyIterReturn,
        types::{IterNext, IterNextIterable},
        PyObjectRef, PyObjectView, PyObjectWrap, PyRef, PyResult, PyValue, TryFromObject,
        VirtualMachine,
    };
    use std::{cell, fmt, future};
    use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
    use wasm_bindgen_futures::{future_to_promise, JsFuture};

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

    #[pyattr]
    #[pyclass(module = "_js", name = "JSValue")]
    #[derive(Debug, PyValue)]
    pub struct PyJsValue {
        pub(crate) value: JsValue,
    }
    type PyJsValueRef = PyRef<PyJsValue>;

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
                JsProperty::Str(s) => s.as_str().into(),
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
            PyJsValue::new(s.as_str())
        }

        #[pymethod]
        fn new_from_float(&self, n: PyRef<PyFloat>) -> PyJsValue {
            PyJsValue::new(n.to_f64())
        }

        #[pymethod]
        fn new_closure(&self, obj: PyObjectRef, vm: &VirtualMachine) -> JsClosure {
            JsClosure::new(obj, false, vm)
        }

        #[pymethod]
        fn new_closure_once(&self, obj: PyObjectRef, vm: &VirtualMachine) -> JsClosure {
            JsClosure::new(obj, true, vm)
        }

        #[pymethod]
        fn new_object(&self, opts: NewObjectOptions, vm: &VirtualMachine) -> PyResult<PyJsValue> {
            let value = if let Some(proto) = opts.prototype {
                if let Some(proto) = proto.value.dyn_ref::<Object>() {
                    Object::create(proto)
                } else if proto.value.is_null() {
                    Object::create(proto.value.unchecked_ref())
                } else {
                    return Err(
                        vm.new_value_error("prototype must be an Object or null".to_owned())
                    );
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
        fn set_prop(
            &self,
            name: JsProperty,
            value: PyJsValueRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            set_prop(&self.value, &name.into_jsvalue(), &value.value)
                .map_err(|err| new_js_error(vm, err))
        }

        #[pymethod]
        fn call(
            &self,
            args: PosArgs<PyJsValueRef>,
            opts: CallOptions,
            vm: &VirtualMachine,
        ) -> PyResult<PyJsValue> {
            let func = self
                .value
                .dyn_ref::<js_sys::Function>()
                .ok_or_else(|| vm.new_type_error("JS value is not callable".to_owned()))?;
            let js_args = args.iter().map(|x| -> &PyJsValue { x }).collect::<Array>();
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
            args: PosArgs<PyJsValueRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyJsValue> {
            let js_args = args.iter().map(|x| -> &PyJsValue { x }).collect::<Array>();
            call_method(&self.value, &name.into_jsvalue(), &js_args)
                .map(PyJsValue::new)
                .map_err(|err| new_js_error(vm, err))
        }

        #[pymethod]
        fn construct(
            &self,
            args: PosArgs<PyJsValueRef>,
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
            let js_args = args.iter().map(|x| -> &PyJsValue { x }).collect::<Array>();
            let constructed_result = if let Some(proto) = proto {
                Reflect::construct_with_new_target(ctor, &js_args, proto)
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

        #[pymethod(magic)]
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

    type ClosureType = Closure<dyn FnMut(JsValue, Box<[JsValue]>) -> Result<JsValue, JsValue>>;

    #[pyattr]
    #[pyclass(module = "_js", name = "JSClosure")]
    #[derive(PyValue)]
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

    #[pyimpl]
    impl JsClosure {
        fn new(obj: PyObjectRef, once: bool, vm: &VirtualMachine) -> Self {
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
            let closure: ClosureType = if once {
                Closure::wrap(Box::new(f))
            } else {
                Closure::once(Box::new(f))
            };
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

    #[pyattr]
    #[pyclass(module = "_js", name = "Promise")]
    #[derive(Debug, Clone, PyValue)]
    pub struct PyPromise {
        value: PromiseKind,
    }

    #[derive(Debug, Clone)]
    enum PromiseKind {
        Js(Promise),
        PyProm { then: PyObjectRef },
        PyResolved(PyObjectRef),
        PyRejected(PyBaseExceptionRef),
    }

    #[pyimpl]
    impl PyPromise {
        pub fn new(value: Promise) -> PyPromise {
            PyPromise {
                value: PromiseKind::Js(value),
            }
        }
        pub fn from_future<F>(future: F) -> PyPromise
        where
            F: future::Future<Output = Result<JsValue, JsValue>> + 'static,
        {
            PyPromise::new(future_to_promise(future))
        }
        pub fn as_js(&self, vm: &VirtualMachine) -> Promise {
            match &self.value {
                PromiseKind::Js(prom) => prom.clone(),
                PromiseKind::PyProm { then } => Promise::new(&mut |js_resolve, js_reject| {
                    let resolve = move |res: PyObjectRef, vm: &VirtualMachine| {
                        let _ = js_resolve.call1(&JsValue::UNDEFINED, &convert::py_to_js(vm, res));
                    };
                    let reject = move |err: PyBaseExceptionRef, vm: &VirtualMachine| {
                        let _ = js_reject
                            .call1(&JsValue::UNDEFINED, &convert::py_err_to_js_err(vm, &err));
                    };
                    let _ = vm.invoke(
                        then,
                        (
                            vm.ctx.new_function("resolve", resolve),
                            vm.ctx.new_function("reject", reject),
                        ),
                    );
                }),
                PromiseKind::PyResolved(obj) => {
                    Promise::resolve(&convert::py_to_js(vm, obj.clone()))
                }
                PromiseKind::PyRejected(err) => {
                    Promise::reject(&convert::py_err_to_js_err(vm, err))
                }
            }
        }

        fn cast(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
            let then = vm.get_attribute_opt(obj.clone(), "then")?;
            let value = if let Some(then) = then.filter(|obj| vm.is_callable(obj)) {
                PromiseKind::PyProm { then }
            } else {
                PromiseKind::PyResolved(obj)
            };
            Ok(Self { value })
        }

        fn cast_result(res: PyResult, vm: &VirtualMachine) -> PyResult<Self> {
            match res {
                Ok(res) => Self::cast(res, vm),
                Err(e) => Ok(Self {
                    value: PromiseKind::PyRejected(e),
                }),
            }
        }

        #[pyclassmethod]
        fn resolve(cls: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::cast(obj, vm)?.into_ref_with_type(vm, cls)
        }

        #[pyclassmethod]
        fn reject(
            cls: PyTypeRef,
            err: PyBaseExceptionRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            Self {
                value: PromiseKind::PyRejected(err),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn then(
            &self,
            on_fulfill: OptionalOption<ArgCallable>,
            on_reject: OptionalOption<ArgCallable>,
            vm: &VirtualMachine,
        ) -> PyResult<PyPromise> {
            let (on_fulfill, on_reject) = (on_fulfill.flatten(), on_reject.flatten());
            if on_fulfill.is_none() && on_reject.is_none() {
                return Ok(self.clone());
            }
            match &self.value {
                PromiseKind::Js(prom) => {
                    let weak_vm = weak_vm(vm);
                    let prom = JsFuture::from(prom.clone());

                    let ret_future = async move {
                        let stored_vm = &weak_vm
                            .upgrade()
                            .expect("that the vm is valid when the promise resolves");
                        let res = prom.await;
                        match res {
                            Ok(val) => match on_fulfill {
                                Some(on_fulfill) => stored_vm.interp.enter(move |vm| {
                                    let val = convert::js_to_py(vm, val);
                                    let res = on_fulfill.invoke((val,), vm);
                                    convert::pyresult_to_jsresult(vm, res)
                                }),
                                None => Ok(val),
                            },
                            Err(err) => match on_reject {
                                Some(on_reject) => stored_vm.interp.enter(move |vm| {
                                    let err = new_js_error(vm, err);
                                    let res = on_reject.invoke((err,), vm);
                                    convert::pyresult_to_jsresult(vm, res)
                                }),
                                None => Err(err),
                            },
                        }
                    };

                    Ok(PyPromise::from_future(ret_future))
                }
                PromiseKind::PyProm { then } => Self::cast_result(
                    vm.invoke(
                        then,
                        (
                            on_fulfill.map(|c| c.into_object()),
                            on_reject.map(|c| c.into_object()),
                        ),
                    ),
                    vm,
                ),
                PromiseKind::PyResolved(res) => match on_fulfill {
                    Some(resolve) => Self::cast_result(resolve.invoke((res.clone(),), vm), vm),
                    None => Ok(self.clone()),
                },
                PromiseKind::PyRejected(err) => match on_reject {
                    Some(reject) => Self::cast_result(reject.invoke((err.clone(),), vm), vm),
                    None => Ok(self.clone()),
                },
            }
        }

        #[pymethod]
        fn catch(
            &self,
            on_reject: OptionalOption<ArgCallable>,
            vm: &VirtualMachine,
        ) -> PyResult<PyPromise> {
            self.then(OptionalArg::Present(None), on_reject, vm)
        }

        #[pymethod(name = "__await__")]
        fn r#await(zelf: PyRef<Self>) -> AwaitPromise {
            AwaitPromise {
                obj: Some(zelf.into()).into(),
            }
        }
    }

    #[pyclass(noattr, module = "_js", name = "AwaitPromise")]
    #[derive(PyValue)]
    struct AwaitPromise {
        obj: cell::Cell<Option<PyObjectRef>>,
    }

    impl fmt::Debug for AwaitPromise {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("AwaitPromise").finish()
        }
    }

    #[pyimpl(with(IterNext))]
    impl AwaitPromise {
        #[pymethod]
        fn send(&self, val: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            match self.obj.take() {
                Some(prom) => {
                    if val.is_some() {
                        Err(vm.new_type_error(
                            "can't send non-None value to an awaitpromise".to_owned(),
                        ))
                    } else {
                        Ok(PyIterReturn::Return(prom))
                    }
                }
                None => Ok(PyIterReturn::StopIteration(val)),
            }
        }

        #[pymethod]
        fn throw(
            &self,
            exc_type: PyObjectRef,
            exc_val: OptionalArg,
            exc_tb: OptionalArg,
            vm: &VirtualMachine,
        ) -> PyResult {
            let err = vm.normalize_exception(
                exc_type,
                exc_val.unwrap_or_none(vm),
                exc_tb.unwrap_or_none(vm),
            )?;
            Err(err)
        }
    }

    impl IterNextIterable for AwaitPromise {}
    impl IterNext for AwaitPromise {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            zelf.send(None, vm)
        }
    }

    fn new_js_error(vm: &VirtualMachine, err: JsValue) -> PyBaseExceptionRef {
        vm.new_exception(
            vm.class("_js", "JSError"),
            vec![PyJsValue::new(err).into_pyobject(vm)],
        )
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = _js::make_module(vm);

    let ctx = &vm.ctx;
    let js_error = PyType::new_simple_ref("JSError", &vm.ctx.exceptions.exception_type).unwrap();
    extend_class!(ctx, &js_error, {
        "value" => ctx.new_readonly_getset("value", js_error.clone(), |exc: PyBaseExceptionRef| exc.get_arg(0)),
    });

    extend_module!(vm, module, {
        "JSError" => js_error,
    });

    module
}

pub fn setup_js_module(vm: &mut VirtualMachine) {
    vm.add_native_module("_js".to_owned(), Box::new(make_module));
}

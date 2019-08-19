use js_sys::{Array, Object, Reflect};
use rustpython_vm::function::Args;
use rustpython_vm::obj::{objfloat::PyFloatRef, objstr::PyStringRef, objtype::PyClassRef};
use rustpython_vm::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use rustpython_vm::types::create_type;
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen(inline_js = "
export function has_prop(target, prop) { return prop in Object(target); }
export function get_prop(target, prop) { return target[prop]; }
export function set_prop(target, prop, value) { target[prop] = value; }
export function type_of(a) { return typeof a; }
export function instance_of(lhs, rhs) { return lhs instanceof rhs; }
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
}

#[pyclass(name = "JsValue")]
#[derive(Debug)]
pub struct PyJsValue {
    value: JsValue,
}
type PyJsValueRef = PyRef<PyJsValue>;

impl PyValue for PyJsValue {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_js", "JsValue")
    }
}

enum JsProperty {
    Str(PyStringRef),
    Js(PyJsValueRef),
}

impl TryFromObject for JsProperty {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyStringRef::try_from_object(vm, obj.clone())
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
    fn null(&self, _vm: &VirtualMachine) -> PyJsValue {
        PyJsValue::new(JsValue::NULL)
    }

    #[pymethod]
    fn undefined(&self, _vm: &VirtualMachine) -> PyJsValue {
        PyJsValue::new(JsValue::UNDEFINED)
    }

    #[pymethod]
    fn new_from_str(&self, s: PyStringRef, _vm: &VirtualMachine) -> PyJsValue {
        PyJsValue::new(s.as_str())
    }

    #[pymethod]
    fn new_from_float(&self, n: PyFloatRef, _vm: &VirtualMachine) -> PyJsValue {
        PyJsValue::new(n.to_f64())
    }

    #[pymethod]
    fn new_object(&self, opts: NewObjectOptions, vm: &VirtualMachine) -> PyResult<PyJsValue> {
        let value = if let Some(proto) = opts.prototype {
            if let Some(proto) = proto.value.dyn_ref::<Object>() {
                Object::create(proto)
            } else if proto.value.is_null() {
                Object::create(proto.value.unchecked_ref())
            } else {
                return Err(vm.new_value_error("prototype must be an Object or null".to_string()));
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
            .ok_or_else(|| vm.new_type_error("JS value is not callable".to_string()))?;
        let this = opts
            .this
            .map(|this| this.value.clone())
            .unwrap_or(JsValue::UNDEFINED);
        let js_args = Array::new();
        for arg in args {
            js_args.push(&arg.value);
        }
        Reflect::apply(func, &this, &js_args)
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
            .ok_or_else(|| vm.new_type_error("JS value is not callable".to_string()))?;
        let proto = opts
            .prototype
            .as_ref()
            .and_then(|proto| proto.value.dyn_ref::<js_sys::Function>());
        let js_args = Array::new();
        for arg in args {
            js_args.push(&arg.value);
        }
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
    fn as_str(&self, _vm: &VirtualMachine) -> Option<String> {
        self.value.as_string()
    }

    #[pymethod]
    fn as_float(&self, _vm: &VirtualMachine) -> Option<f64> {
        self.value.as_f64()
    }

    #[pymethod]
    fn as_bool(&self, _vm: &VirtualMachine) -> Option<bool> {
        self.value.as_bool()
    }

    #[pymethod(name = "typeof")]
    fn type_of(&self, _vm: &VirtualMachine) -> String {
        type_of(&self.value)
    }

    #[pymethod]
    /// Checks that `typeof self == "object" && self !== null`. Use instead
    /// of `value.typeof() == "object"`
    fn is_object(&self, _vm: &VirtualMachine) -> bool {
        self.value.is_object()
    }

    #[pymethod]
    fn instanceof(&self, rhs: PyJsValueRef, vm: &VirtualMachine) -> PyResult<bool> {
        instance_of(&self.value, &rhs.value).map_err(|err| new_js_error(vm, err))
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        format!("{:?}", self.value)
    }
}

#[derive(FromArgs)]
struct CallOptions {
    #[pyarg(keyword_only, default = "None")]
    this: Option<PyJsValueRef>,
}

#[derive(FromArgs)]
struct NewObjectOptions {
    #[pyarg(keyword_only, default = "None")]
    prototype: Option<PyJsValueRef>,
}

fn new_js_error(vm: &VirtualMachine, err: JsValue) -> PyObjectRef {
    let exc = vm.new_exception(vm.class("_js", "JsError"), format!("{:?}", err));
    vm.set_attr(&exc, "js_value", PyJsValue::new(err).into_ref(vm))
        .unwrap();
    exc
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_js", {
        "JsError" => create_type("JsError", &ctx.type_type(), &ctx.exceptions.exception_type),
        "JsValue" => PyJsValue::make_class(ctx),
    })
}

pub fn setup_js_module(vm: &VirtualMachine) {
    vm.stdlib_inits
        .borrow_mut()
        .insert("_js".to_string(), Box::new(make_module));
}

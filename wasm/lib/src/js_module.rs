use crate::convert;
use js_sys::{Array, Object, Reflect};
use rustpython_vm::function::Args;
use rustpython_vm::obj::{objfloat::PyFloatRef, objstr::PyStringRef, objtype::PyClassRef};
use rustpython_vm::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use rustpython_vm::VirtualMachine;
use wasm_bindgen::{prelude::*, JsCast};

// I don't know why there is no other option for this
#[wasm_bindgen(inline_js = "export function type_of(a) { return typeof a; }")]
extern "C" {
    #[wasm_bindgen]
    fn type_of(a: JsValue) -> String;
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
    fn to_jsvalue(self) -> JsValue {
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

    #[pyclassmethod]
    fn null(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyJsValueRef> {
        PyJsValue::new(JsValue::NULL).into_ref_with_type(vm, cls)
    }

    #[pyclassmethod]
    fn undefined(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyJsValueRef> {
        PyJsValue::new(JsValue::UNDEFINED).into_ref_with_type(vm, cls)
    }

    #[pyclassmethod]
    fn fromstr(cls: PyClassRef, s: PyStringRef, vm: &VirtualMachine) -> PyResult<PyJsValueRef> {
        PyJsValue::new(s.as_str()).into_ref_with_type(vm, cls)
    }

    #[pyclassmethod]
    fn fromfloat(cls: PyClassRef, n: PyFloatRef, vm: &VirtualMachine) -> PyResult<PyJsValueRef> {
        PyJsValue::new(n.to_f64()).into_ref_with_type(vm, cls)
    }

    #[pyclassmethod]
    fn new_object(
        cls: PyClassRef,
        opts: NewObjectOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyJsValueRef> {
        let value = if let Some(proto) = opts.prototype {
            if let Some(proto) = proto.value.dyn_ref::<Object>() {
                Object::create(proto)
            } else if proto.value.is_null() {
                Object::create(proto.value.unchecked_ref())
            } else {
                return Err(vm.new_value_error(format!("prototype must be an Object or null")));
            }
        } else {
            Object::new()
        };
        PyJsValue::new(value).into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn get_prop(&self, name: JsProperty, vm: &VirtualMachine) -> PyResult<PyJsValue> {
        let name = &name.to_jsvalue();
        if Reflect::has(&self.value, name).map_err(|err| convert::js_to_py(vm, err))? {
            Reflect::get(&self.value, name)
                .map(PyJsValue::new)
                .map_err(|err| convert::js_to_py(vm, err))
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
    ) -> PyResult<PyJsValue> {
        Reflect::set(&self.value, &name.to_jsvalue(), &value.value)
            .map(PyJsValue::new)
            .map_err(|err| convert::js_to_py(vm, err))
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

    #[pymethod]
    /// Checks that `typeof self == "object" && self !== null`. Use instead
    /// of `value.typeof() == "object"`
    fn is_object(&self, _vm: &VirtualMachine) -> bool {
        self.value.is_object()
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
            .map_err(|err| convert::js_to_py(vm, err))
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
            .and_then(|proto| proto.value.dyn_ref::<js_sys::Function>().clone());
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
            .map_err(|err| convert::js_to_py(vm, err))
    }


    #[pymethod(name = "typeof")]
    fn type_of(&self, _vm: &VirtualMachine) -> String {
        type_of(self.value.clone())
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "_js", {
      "JsValue" => PyJsValue::make_class(&vm.ctx),
    })
}

pub fn setup_js_module(vm: &VirtualMachine) {
    vm.stdlib_inits
        .borrow_mut()
        .insert("_js".to_string(), Box::new(make_module));
}

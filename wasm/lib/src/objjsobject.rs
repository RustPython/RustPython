use js_sys::{Function, JsString, Object, Reflect};
use wasm_bindgen::JsValue;

use rustpython_vm::function::{Args, OptionalArg};
use rustpython_vm::obj::objstr::PyStringRef;
use rustpython_vm::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use rustpython_vm::VirtualMachine;

use crate::convert;

fn get_prop(object: Object, name: &str, vm: &VirtualMachine) -> Option<PyObjectRef> {
    let name: &JsString = &name.into();
    if Reflect::has(&object, name).expect("Reflect.has failed") {
        Some(convert::js_to_py_with_this(
            vm,
            Reflect::get(&object, name).expect("Reflect.get failed"),
            Some(object.into()),
        ))
    } else {
        None
    }
}

fn set_prop(value: &Object, name: &str, val: PyObjectRef, vm: &VirtualMachine) {
    Reflect::set(value, &name.into(), &convert::py_to_js(vm, val)).expect("Reflect failed");
}

#[pyclass(name = "JSObject")]
#[derive(Debug)]
pub struct PyJsObject {
    object: Object,
}
pub type PyJsObjectRef = PyRef<PyJsObject>;

impl PyValue for PyJsObject {}

#[pyimpl]
impl PyJsObject {
    pub fn new(object: Object) -> PyJsObject {
        PyJsObject { object }
    }

    pub fn object(&self) -> &Object {
        &self.object
    }

    #[pyproperty(name = "_props")]
    fn props(&self, _vm: &VirtualMachine) -> PyJsProps {
        PyJsProps {
            object: self.object().clone(),
        }
    }

    #[pymethod(name = "__getattr__")]
    fn getattr(&self, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        get_prop(self.object().clone(), attr_name.as_str(), vm).ok_or_else(|| {
            vm.new_attribute_error(format!("JS value has no property {:?}", attr_name.as_str()))
        })
    }

    #[pymethod(name = "__setattr__")]
    fn setattr(&self, attr_name: PyStringRef, val: PyObjectRef, vm: &VirtualMachine) {
        set_prop(self.object(), attr_name.as_str(), val, vm);
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        format!("{:?}", self.object())
    }
}

#[pyclass(name = "JSFunction")]
#[derive(Debug)]
pub struct PyJsFunction {
    func: Function,
    this: Option<JsValue>,
}

impl PyValue for PyJsFunction {}

#[pyimpl]
impl PyJsFunction {
    pub fn new(func: Function, this: Option<JsValue>) -> PyJsFunction {
        PyJsFunction { func, this }
    }
    pub fn to_function(&self) -> Function {
        if let Some(this) = &self.this {
            self.func.bind(&this)
        } else {
            self.func.clone()
        }
    }

    #[pymethod(name = "__call__")]
    fn call(&self, args: Args, vm: &VirtualMachine) -> PyResult {
        let undef = JsValue::UNDEFINED;
        let this = match self.this {
            Some(ref this) => this,
            None => &undef,
        };
        let args = convert::iter_to_array(args.into_iter().map(|elem| convert::py_to_js(vm, elem)));
        let result = self.func.apply(this, &args);
        result
            .map(|val| convert::js_to_py(vm, val))
            .map_err(|err| convert::js_to_py(vm, err))
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        // for better formatting
        let value: &JsValue = &self.func;
        format!("{:?}", value)
    }
}

#[pyclass(name = "JSProps")]
#[derive(Debug)]
struct PyJsProps {
    object: Object,
}

impl PyValue for PyJsProps {}

#[pyimpl]
impl PyJsProps {
    #[pymethod]
    fn get(
        &self,
        item_name: PyStringRef,
        default: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        get_prop(self.object.clone(), item_name.as_str(), vm)
            .or(default.into_option())
            .unwrap_or_else(|| vm.get_none())
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, item_name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        get_prop(self.object.clone(), item_name.as_str(), vm)
            .ok_or_else(|| vm.new_key_error(format!("{:?}", item_name.as_str())))
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(&self, item_name: PyStringRef, val: PyObjectRef, vm: &VirtualMachine) {
        set_prop(&self.object, item_name.as_str(), val, vm);
    }
}

pub fn init(ctx: &PyContext) {
    ctx.add_class::<PyJsObject>();
    ctx.add_class::<PyJsFunction>();
    ctx.add_class::<PyJsProps>();
}

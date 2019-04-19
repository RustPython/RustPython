use js_sys::{Function, JsString, Reflect};
use wasm_bindgen::JsValue;

use rustpython_vm::function::{Args, OptionalArg};
use rustpython_vm::obj::objstr::PyStringRef;
use rustpython_vm::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use rustpython_vm::VirtualMachine;

use crate::convert;

fn get_prop(value: JsValue, name: &str, vm: &VirtualMachine) -> Option<PyObjectRef> {
    let name: &JsString = &name.into();
    if Reflect::has(&value, name).expect("Reflect.has failed") {
        Some(convert::js_to_py_with_this(
            vm,
            Reflect::get(&value, name).expect("Reflect.get failed"),
            Some(value),
        ))
    } else {
        None
    }
}

fn set_prop(value: &JsValue, name: &str, val: PyObjectRef, vm: &VirtualMachine) {
    Reflect::set(value, &name.into(), &convert::py_to_js(vm, val)).expect("Reflect failed");
}

#[pyclass(name = "JsValue")]
#[derive(Debug)]
pub struct PyJsValue {
    value: JsValue,
}
pub type PyJsValueRef = PyRef<PyJsValue>;

impl PyValue for PyJsValue {}

#[pyimpl]
impl PyJsValue {
    pub fn new(value: JsValue) -> PyJsValue {
        PyJsValue { value }
    }

    pub fn value(&self) -> &JsValue {
        &self.value
    }

    #[pyproperty(name = "_props")]
    fn props(&self, _vm: &VirtualMachine) -> PyJsProps {
        PyJsProps {
            value: self.value().clone(),
        }
    }

    #[pymethod(name = "__getattr__")]
    fn getattr(&self, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        get_prop(self.value().clone(), attr_name.as_str(), vm).ok_or_else(|| {
            vm.new_attribute_error(format!("JS value has no property {:?}", attr_name.as_str()))
        })
    }

    #[pymethod(name = "__setattr__")]
    fn setattr(&self, attr_name: PyStringRef, val: PyObjectRef, vm: &VirtualMachine) {
        set_prop(self.value(), attr_name.as_str(), val, vm);
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        format!("{:?}", self.value())
    }
}

#[pyclass(name = "JsFunction")]
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

#[pyclass(name = "JsProps")]
#[derive(Debug)]
struct PyJsProps {
    value: JsValue,
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
        get_prop(self.value.clone(), item_name.as_str(), vm)
            .or(default.into_option())
            .unwrap_or_else(|| vm.get_none())
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, item_name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        get_prop(self.value.clone(), item_name.as_str(), vm)
            .ok_or_else(|| vm.new_key_error(format!("{:?}", item_name.as_str())))
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(&self, item_name: PyStringRef, val: PyObjectRef, vm: &VirtualMachine) {
        set_prop(&self.value, item_name.as_str(), val, vm);
    }
}

pub fn init(ctx: &PyContext) {
    ctx.add_class::<PyJsValue>();
    ctx.add_class::<PyJsFunction>();
    ctx.add_class::<PyJsProps>();
}

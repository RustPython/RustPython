use super::objstr::PyStringRef;
use super::objtype::{self, PyClassRef};
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyMappingProxy {
    class: PyClassRef,
}

pub type PyMappingProxyRef = PyRef<PyMappingProxy>;

impl PyValue for PyMappingProxy {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.mappingproxy_type.clone()
    }
}

#[pyimpl]
impl PyMappingProxy {
    pub fn new(class: PyClassRef) -> PyMappingProxy {
        PyMappingProxy { class }
    }

    #[pymethod]
    fn get(&self, key: PyStringRef, default: OptionalArg, vm: &VirtualMachine) -> PyObjectRef {
        let default = default.into_option();
        objtype::class_get_attr(&self.class, key.as_str())
            .or(default)
            .unwrap_or_else(|| vm.get_none())
    }

    #[pymethod(name = "__getitem__")]
    pub fn getitem(&self, key: PyStringRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = objtype::class_get_attr(&self.class, key.as_str()) {
            return Ok(value);
        }
        Err(vm.new_key_error(key.into_object()))
    }

    #[pymethod(name = "__contains__")]
    pub fn contains(&self, attr: PyStringRef, _vm: &VirtualMachine) -> bool {
        objtype::class_has_attr(&self.class, attr.as_str())
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.types.mappingproxy_type)
}

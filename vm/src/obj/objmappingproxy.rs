use super::objstr::PyStringRef;
use super::objtype::{self, PyClassRef};
use crate::pyobject::{PyClassImpl, PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PyMappingProxy {
    class: PyClassRef,
}

pub type PyMappingProxyRef = PyRef<PyMappingProxy>;

impl PyValue for PyMappingProxy {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.mappingproxy_type.clone()
    }
}

#[pyimpl]
impl PyMappingProxy {
    pub fn new(class: PyClassRef) -> PyMappingProxy {
        PyMappingProxy { class }
    }

    #[pymethod(name = "__getitem__")]
    pub fn getitem(&self, key: PyStringRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = objtype::class_get_attr(&self.class, key.as_str()) {
            return Ok(value);
        }
        Err(vm.new_key_error(format!("Key not found: {}", key)))
    }

    #[pymethod(name = "__contains__")]
    pub fn contains(&self, attr: PyStringRef, _vm: &VirtualMachine) -> bool {
        objtype::class_has_attr(&self.class, attr.as_str())
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.mappingproxy_type)
}

use super::objdict::PyDict;
use super::objiter;
use super::objstr::PyStringRef;
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject,
};
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "mappingproxy")]
#[derive(Debug)]
pub struct PyMappingProxy {
    mapping: MappingProxyInner,
}

#[derive(Debug)]
enum MappingProxyInner {
    Class(PyClassRef),
    Dict(PyObjectRef),
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
        PyMappingProxy {
            mapping: MappingProxyInner::Class(class),
        }
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, mapping: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyMappingProxy {
            mapping: MappingProxyInner::Dict(mapping),
        }
        .into_ref_with_type(vm, cls)
    }

    fn get_inner(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        let opt = match &self.mapping {
            MappingProxyInner::Class(class) => {
                let key = PyStringRef::try_from_object(vm, key)?;
                class.get_attr(key.borrow_value())
            }
            MappingProxyInner::Dict(obj) => obj.get_item(key, vm).ok(),
        };
        Ok(opt)
    }

    #[pymethod]
    fn get(&self, key: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let default = default.into_option();
        Ok(self
            .get_inner(key, vm)?
            .or(default)
            .unwrap_or_else(|| vm.get_none()))
    }

    #[pymethod(name = "__getitem__")]
    pub fn getitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.get_inner(key.clone(), vm)?
            .ok_or_else(|| vm.new_key_error(key))
    }

    #[pymethod(name = "__contains__")]
    pub fn contains(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Class(class) => {
                let key = PyStringRef::try_from_object(vm, key)?;
                Ok(vm.ctx.new_bool(class.has_attr(key.borrow_value())))
            }
            MappingProxyInner::Dict(obj) => vm._membership(obj.clone(), key),
        }
    }

    #[pymethod(name = "__iter__")]
    pub fn iter(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                // TODO: something that's much more efficient than this
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        objiter::get_iter(vm, &obj)
    }
    #[pymethod]
    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "items", vec![])
    }
    #[pymethod]
    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "keys", vec![])
    }
    #[pymethod]
    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "values", vec![])
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.types.mappingproxy_type)
}

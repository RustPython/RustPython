use super::objiter;
use super::objstr::PyStringRef;
use super::objtype::{self, PyClassRef};
use crate::function::OptionalArg;
use crate::pyobject::{
    ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
use crate::vm::VirtualMachine;

#[pyclass]
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

    #[pyslot(new)]
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
                objtype::class_get_attr(&class, key.as_str())
            }
            MappingProxyInner::Dict(obj) => obj.get_item(&key, vm).ok(),
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
                Ok(vm.new_bool(objtype::class_has_attr(&class, key.as_str())))
            }
            MappingProxyInner::Dict(obj) => vm._membership(obj.clone(), key),
        }
    }

    #[pymethod(name = "__iter__")]
    pub fn iter(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => objiter::get_iter(vm, d),
            MappingProxyInner::Class(_c) => Err(vm.new_type_error("Can't get iter".to_string())),
        }
    }
    #[pymethod]
    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => vm.call_method(d, "items", vec![]),
            MappingProxyInner::Class(_c) => Err(vm.new_type_error("Can't get iter".to_string())),
        }
    }
    #[pymethod]
    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => vm.call_method(d, "keys", vec![]),
            MappingProxyInner::Class(_c) => Err(vm.new_type_error("Can't get iter".to_string())),
        }
    }
    #[pymethod]
    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => vm.call_method(d, "values", vec![]),
            MappingProxyInner::Class(_c) => Err(vm.new_type_error("Can't get iter".to_string())),
        }
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.types.mappingproxy_type)
}

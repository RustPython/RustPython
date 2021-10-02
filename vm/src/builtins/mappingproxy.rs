use super::{PyDict, PyStrRef, PyTypeRef};
use crate::{
    function::OptionalArg,
    slots::{Iterable, SlotConstructor},
    IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, VirtualMachine,
};

#[pyclass(module = false, name = "mappingproxy")]
#[derive(Debug)]
pub struct PyMappingProxy {
    mapping: MappingProxyInner,
}

#[derive(Debug)]
enum MappingProxyInner {
    Class(PyTypeRef),
    Dict(PyObjectRef),
}

impl PyValue for PyMappingProxy {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.mappingproxy_type
    }
}

impl PyMappingProxy {
    pub fn new(class: PyTypeRef) -> Self {
        Self {
            mapping: MappingProxyInner::Class(class),
        }
    }
}

impl SlotConstructor for PyMappingProxy {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, mapping: Self::Args, vm: &VirtualMachine) -> PyResult {
        Self {
            mapping: MappingProxyInner::Dict(mapping),
        }
        .into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(Iterable, SlotConstructor))]
impl PyMappingProxy {
    fn get_inner(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        let opt = match &self.mapping {
            MappingProxyInner::Class(class) => {
                let key = PyStrRef::try_from_object(vm, key)?;
                class.attributes.read().get(key.as_str()).cloned()
            }
            MappingProxyInner::Dict(obj) => obj.get_item(key, vm).ok(),
        };
        Ok(opt)
    }

    #[pymethod]
    fn get(
        &self,
        key: PyObjectRef,
        default: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let default = default.into_option();
        let value = self.get_inner(key, vm)?.or(default);
        Ok(value)
    }

    #[pymethod(magic)]
    pub fn getitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.get_inner(key.clone(), vm)?
            .ok_or_else(|| vm.new_key_error(key))
    }

    #[pymethod(magic)]
    pub fn contains(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Class(class) => {
                let key = PyStrRef::try_from_object(vm, key)?;
                Ok(vm
                    .ctx
                    .new_bool(class.attributes.read().contains_key(key.as_str())))
            }
            MappingProxyInner::Dict(obj) => vm._membership(obj.clone(), key),
        }
    }

    #[pymethod]
    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "items", ())
    }
    #[pymethod]
    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "keys", ())
    }
    #[pymethod]
    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        vm.call_method(&obj, "values", ())
    }
    #[pymethod]
    pub fn copy(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => vm.call_method(d, "copy", ()),
            MappingProxyInner::Class(c) => {
                Ok(PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm))
            }
        }
    }
}
impl Iterable for PyMappingProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = match &zelf.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                // TODO: something that's much more efficient than this
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.into_pyobject(vm)
            }
        };
        let iter = obj.get_iter(vm)?;
        Ok(iter.into_object())
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.types.mappingproxy_type)
}

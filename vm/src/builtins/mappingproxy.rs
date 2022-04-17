use std::borrow::Cow;

use super::{PyDict, PyGenericAlias, PyList, PyStr, PyStrRef, PyTuple, PyTypeRef};
use crate::{
    convert::ToPyObject,
    function::OptionalArg,
    protocol::{PyMapping, PyMappingMethods, PySequence, PySequenceMethods},
    pyclass::PyClassImpl,
    types::{AsMapping, AsSequence, Constructor, Iterable},
    AsObject, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    VirtualMachine,
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

impl Constructor for PyMappingProxy {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, mapping: Self::Args, vm: &VirtualMachine) -> PyResult {
        if !PyMapping::from(mapping.as_ref()).check(vm)
            || mapping.payload_if_subclass::<PyList>(vm).is_some()
            || mapping.payload_if_subclass::<PyTuple>(vm).is_some()
        {
            Err(vm.new_type_error(format!(
                "mappingproxy() argument must be a mapping, not {}",
                mapping.class()
            )))
        } else {
            Self {
                mapping: MappingProxyInner::Dict(mapping),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }
}

#[pyimpl(with(AsMapping, Iterable, Constructor, AsSequence))]
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

    fn _contains(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        match &self.mapping {
            MappingProxyInner::Class(class) => {
                // let key = PyStrRef::try_from_object(vm, key)?;
                let key = key
                    .payload::<PyStr>()
                    .ok_or_else(|| vm.new_downcast_type_error(PyStr::class(vm), key))?;
                Ok(class.attributes.read().contains_key(key.as_str()))
            }
            MappingProxyInner::Dict(obj) => PySequence::from(obj.as_ref()).contains(key, vm),
        }
    }

    #[pymethod(magic)]
    pub fn contains(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self._contains(&key, vm)
    }

    #[pymethod]
    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        };
        vm.call_method(&obj, "items", ())
    }
    #[pymethod]
    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        };
        vm.call_method(&obj, "keys", ())
    }
    #[pymethod]
    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        };
        vm.call_method(&obj, "values", ())
    }
    #[pymethod]
    pub fn copy(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Dict(d) => vm.call_method(d, "copy", ()),
            MappingProxyInner::Class(c) => {
                Ok(PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm))
            }
        }
    }
    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let obj = match &self.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        };
        Ok(format!("mappingproxy({})", obj.repr(vm)?))
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl PyMappingProxy {
    const MAPPING_METHODS: PyMappingMethods = PyMappingMethods {
        length: None,
        subscript: Some(|mapping, needle, vm| {
            Self::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
        }),
        ass_subscript: None,
    };
}

impl AsMapping for PyMappingProxy {
    fn as_mapping(_zelf: &crate::PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        Self::MAPPING_METHODS
    }
}

impl AsSequence for PyMappingProxy {
    fn as_sequence(
        _zelf: &crate::PyObjectView<Self>,
        _vm: &VirtualMachine,
    ) -> Cow<'static, PySequenceMethods> {
        Cow::Borrowed(&Self::SEQUENCE_METHODS)
    }
}

impl PyMappingProxy {
    const SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
        contains: Some(|seq, target, vm| Self::sequence_downcast(seq)._contains(target, vm)),
        ..*PySequenceMethods::not_implemented()
    };
}

impl Iterable for PyMappingProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = match &zelf.mapping {
            MappingProxyInner::Dict(d) => d.clone(),
            MappingProxyInner::Class(c) => {
                // TODO: something that's much more efficient than this
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        };
        let iter = obj.get_iter(vm)?;
        Ok(iter.into())
    }
}

pub fn init(context: &PyContext) {
    PyMappingProxy::extend_class(context, &context.types.mappingproxy_type)
}

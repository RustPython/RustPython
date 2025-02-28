use super::{PyDict, PyDictRef, PyGenericAlias, PyList, PyTuple, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    class::PyClassImpl,
    convert::ToPyObject,
    function::{ArgMapping, OptionalArg, PyComparisonValue},
    object::{Traverse, TraverseFn},
    protocol::{PyMapping, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, Iterable, PyComparisonOp,
        Representable,
    },
};
use once_cell::sync::Lazy;

#[pyclass(module = false, name = "mappingproxy", traverse)]
#[derive(Debug)]
pub struct PyMappingProxy {
    mapping: MappingProxyInner,
}

#[derive(Debug)]
enum MappingProxyInner {
    Class(PyTypeRef),
    Mapping(ArgMapping),
}

unsafe impl Traverse for MappingProxyInner {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        match self {
            MappingProxyInner::Class(r) => r.traverse(tracer_fn),
            MappingProxyInner::Mapping(arg) => arg.traverse(tracer_fn),
        }
    }
}

impl PyPayload for PyMappingProxy {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.mappingproxy_type
    }
}

impl From<PyTypeRef> for PyMappingProxy {
    fn from(dict: PyTypeRef) -> Self {
        Self {
            mapping: MappingProxyInner::Class(dict),
        }
    }
}

impl From<PyDictRef> for PyMappingProxy {
    fn from(dict: PyDictRef) -> Self {
        Self {
            mapping: MappingProxyInner::Mapping(ArgMapping::from_dict_exact(dict)),
        }
    }
}

impl Constructor for PyMappingProxy {
    type Args = PyObjectRef;

    fn py_new(cls: PyTypeRef, mapping: Self::Args, vm: &VirtualMachine) -> PyResult {
        if let Some(methods) = PyMapping::find_methods(&mapping) {
            if mapping.payload_if_subclass::<PyList>(vm).is_none()
                && mapping.payload_if_subclass::<PyTuple>(vm).is_none()
            {
                return Self {
                    mapping: MappingProxyInner::Mapping(ArgMapping::with_methods(
                        mapping,
                        unsafe { methods.borrow_static() },
                    )),
                }
                .into_ref_with_type(vm, cls)
                .map(Into::into);
            }
        }
        Err(vm.new_type_error(format!(
            "mappingproxy() argument must be a mapping, not {}",
            mapping.class()
        )))
    }
}

#[pyclass(with(
    AsMapping,
    Iterable,
    Constructor,
    AsSequence,
    Comparable,
    AsNumber,
    Representable
))]
impl PyMappingProxy {
    fn get_inner(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
        match &self.mapping {
            MappingProxyInner::Class(class) => Ok(key
                .as_interned_str(vm)
                .and_then(|key| class.attributes.read().get(key).cloned())),
            MappingProxyInner::Mapping(mapping) => mapping.mapping().subscript(&*key, vm).map(Some),
        }
    }

    #[pymethod]
    fn get(
        &self,
        key: PyObjectRef,
        default: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let obj = self.to_object(vm)?;
        Ok(Some(vm.call_method(
            &obj,
            "get",
            (key, default.unwrap_or_none(vm)),
        )?))
    }

    #[pymethod(magic)]
    pub fn getitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.get_inner(key.clone(), vm)?
            .ok_or_else(|| vm.new_key_error(key))
    }

    fn _contains(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        match &self.mapping {
            MappingProxyInner::Class(class) => Ok(key
                .as_interned_str(vm)
                .is_some_and(|key| class.attributes.read().contains_key(key))),
            MappingProxyInner::Mapping(mapping) => mapping.to_sequence().contains(key, vm),
        }
    }

    #[pymethod(magic)]
    pub fn contains(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self._contains(&key, vm)
    }

    fn to_object(&self, vm: &VirtualMachine) -> PyResult {
        Ok(match &self.mapping {
            MappingProxyInner::Mapping(d) => d.as_ref().to_owned(),
            MappingProxyInner::Class(c) => {
                PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm)
            }
        })
    }

    #[pymethod]
    pub fn items(&self, vm: &VirtualMachine) -> PyResult {
        let obj = self.to_object(vm)?;
        vm.call_method(&obj, identifier!(vm, items).as_str(), ())
    }
    #[pymethod]
    pub fn keys(&self, vm: &VirtualMachine) -> PyResult {
        let obj = self.to_object(vm)?;
        vm.call_method(&obj, identifier!(vm, keys).as_str(), ())
    }
    #[pymethod]
    pub fn values(&self, vm: &VirtualMachine) -> PyResult {
        let obj = self.to_object(vm)?;
        vm.call_method(&obj, identifier!(vm, values).as_str(), ())
    }
    #[pymethod]
    pub fn copy(&self, vm: &VirtualMachine) -> PyResult {
        match &self.mapping {
            MappingProxyInner::Mapping(d) => vm.call_method(d, identifier!(vm, copy).as_str(), ()),
            MappingProxyInner::Class(c) => {
                Ok(PyDict::from_attributes(c.attributes.read().clone(), vm)?.to_pyobject(vm))
            }
        }
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        let obj = self.to_object(vm)?;
        obj.length(vm)
    }

    #[pymethod(magic)]
    fn reversed(&self, vm: &VirtualMachine) -> PyResult {
        vm.call_method(
            self.to_object(vm)?.as_object(),
            identifier!(vm, __reversed__).as_str(),
            (),
        )
    }

    #[pymethod(magic)]
    fn ior(&self, _args: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error(format!(
            "\"'|=' is not supported by {}; use '|' instead\"",
            Self::class(&vm.ctx)
        )))
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(&self, args: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._or(self.copy(vm)?.as_ref(), args.as_ref())
    }
}

impl Comparable for PyMappingProxy {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let obj = zelf.to_object(vm)?;
        Ok(PyComparisonValue::Implemented(
            obj.rich_compare_bool(other, op, vm)?,
        ))
    }
}

impl AsMapping for PyMappingProxy {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, vm| PyMappingProxy::mapping_downcast(mapping).len(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyMappingProxy::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyMappingProxy {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            contains: atomic_func!(
                |seq, target, vm| PyMappingProxy::sequence_downcast(seq)._contains(target, vm)
            ),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyMappingProxy {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyMappingProxy>() {
                    a.or(b.to_pyobject(vm), vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            inplace_or: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyMappingProxy>() {
                    a.ior(b.to_pyobject(vm), vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Iterable for PyMappingProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.to_object(vm)?;
        let iter = obj.get_iter(vm)?;
        Ok(iter.into())
    }
}

impl Representable for PyMappingProxy {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let obj = zelf.to_object(vm)?;
        Ok(format!("mappingproxy({})", obj.repr(vm)?))
    }
}

pub fn init(context: &Context) {
    PyMappingProxy::extend_class(context, context.types.mappingproxy_type)
}

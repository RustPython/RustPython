use super::{PyDict, PyDictRef, PyFrozenDict, dict::DictIter};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
    convert::ToPyObject,
    dict_inner::DictKey,
    object::{Traverse, TraverseFn},
};
use core::borrow::Borrow;

/// A checked reference to a dict or frozendict, preserving the object's mutability.
/// Its single-pointer layout also supports readonly object members such as __globals__.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct PyAnyDictRef(PyObjectRef);

impl super::descriptor::MemberLayout for PyAnyDictRef {
    const KIND: super::descriptor::MemberKind = super::descriptor::MemberKind::Object;
}

unsafe impl Traverse for PyAnyDictRef {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.0.traverse(tracer);
    }
}

impl Borrow<PyObject> for PyAnyDictRef {
    fn borrow(&self) -> &PyObject {
        &self.0
    }
}

impl From<PyDictRef> for PyAnyDictRef {
    fn from(dict: PyDictRef) -> Self {
        Self(dict.into())
    }
}
impl From<PyRef<PyFrozenDict>> for PyAnyDictRef {
    fn from(dict: PyRef<PyFrozenDict>) -> Self {
        Self(dict.into())
    }
}
impl From<PyAnyDictRef> for PyObjectRef {
    fn from(dict: PyAnyDictRef) -> Self {
        dict.0
    }
}
impl ToPyObject for PyAnyDictRef {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}
impl TryFromObject for PyAnyDictRef {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::from_object(&obj).ok_or_else(|| {
            vm.new_type_error(format!(
                "expected dict or frozendict, got {}",
                obj.class().name()
            ))
        })
    }
}

impl PyAnyDictRef {
    pub fn from_object(obj: &PyObject) -> Option<Self> {
        (obj.downcastable::<PyDict>() || obj.downcastable::<PyFrozenDict>())
            .then(|| Self(obj.to_owned()))
    }

    pub(crate) fn as_mutable(&self) -> Option<&Py<PyDict>> {
        self.0.downcast_ref()
    }

    pub(crate) fn is_frozen(&self) -> bool {
        self.0.downcastable::<PyFrozenDict>()
    }

    // The raw payload is private: callers outside the VM cannot use it to mutate a frozendict.
    pub(crate) fn as_dict(&self) -> &PyDict {
        if let Some(dict) = self.as_mutable() {
            dict
        } else {
            &self.0.downcast_ref::<PyFrozenDict>().unwrap().dict
        }
    }

    pub(crate) fn uses_builtin_iter(&self, vm: &VirtualMachine) -> bool {
        let base = if self.is_frozen() {
            vm.ctx.types.frozendict_type
        } else {
            vm.ctx.types.dict_type
        };
        self.as_object()
            .class()
            .slots
            .iter
            .load()
            .map(crate::types::fn_addr)
            == base.slots.iter.load().map(crate::types::fn_addr)
    }

    #[must_use]
    pub fn __len__(&self) -> usize {
        self.as_dict().__len__()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_dict().is_empty()
    }
    #[must_use]
    pub fn keys_vec(&self) -> Vec<PyObjectRef> {
        self.as_dict().keys_vec()
    }
    #[must_use]
    pub fn values_vec(&self) -> Vec<PyObjectRef> {
        self.as_dict().values_vec()
    }
    #[must_use]
    pub fn items_vec(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.as_dict().items_vec()
    }
    #[must_use]
    pub fn next_entry(&self, position: usize) -> Option<(usize, PyObjectRef, PyObjectRef)> {
        self.as_dict().next_entry(position)
    }
    pub(crate) fn size(&self) -> crate::dict_inner::DictSize {
        self.as_dict().size()
    }
    pub fn inner_getitem_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(dict) = self.as_mutable() {
            dict.inner_getitem_opt(key, vm)
        } else {
            self.0
                .downcast_ref::<PyFrozenDict>()
                .unwrap()
                .inner_getitem_opt(key, vm)
        }
    }
    pub fn get_item_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(dict) = self.as_mutable() {
            return dict.get_item_opt(key, vm);
        }
        if self.0.class().is(vm.ctx.types.frozendict_type) {
            return self.inner_getitem_opt(key, vm);
        }
        match self.0.get_item(key, vm) {
            Ok(value) => Ok(Some(value)),
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
            Err(exc) => Err(exc),
        }
    }
    pub fn get_item<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.as_mutable() {
            dict.get_item(key, vm)
        } else {
            self.0.get_item(key, vm)
        }
    }
    pub fn contains_key<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> bool {
        self.inner_getitem_opt(key, vm)
            .is_ok_and(|value| value.is_some())
    }
    pub fn set_item<K: DictKey + ?Sized>(
        &self,
        key: &K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match self.as_mutable() {
            Some(dict) => dict.set_item(key, value, vm),
            None => Err(vm.new_type_error("frozendict object does not support item assignment")),
        }
    }
    pub fn del_item<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult<()> {
        match self.as_mutable() {
            Some(dict) => dict.del_item(key, vm),
            None => Err(vm.new_type_error("frozendict object does not support item deletion")),
        }
    }
}

impl<'a> IntoIterator for &'a PyAnyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self.as_dict())
    }
}

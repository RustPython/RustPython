use super::{
    set::PySetInner, IterStatus, PositionIterInternal, PyBaseExceptionRef, PyGenericAlias, PySet,
    PyStrRef, PyTupleRef, PyTypeRef,
};
use crate::{
    builtins::{iter::builtins_iter, PyTuple},
    common::ascii,
    dictdatatype::{self, DictKey},
    function::{ArgIterable, FuncArgs, IntoPyObject, KwArgs, OptionalArg},
    protocol::{PyIterIter, PyIterReturn, PyMappingMethods},
    types::{
        AsMapping, Comparable, Constructor, Hashable, IterNext, IterNextIterable, Iterable,
        PyComparisonOp, Unconstructible, Unhashable,
    },
    vm::{ReprGuard, VirtualMachine},
    IdProtocol, ItemProtocol,
    PyArithmeticValue::*,
    PyAttributes, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef,
    PyObjectView, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use rustpython_common::lock::PyMutex;
use std::fmt;
use std::mem::size_of;

pub type DictContentType = dictdatatype::Dict;

/// dict() -> new empty dictionary
/// dict(mapping) -> new dictionary initialized from a mapping object's
///    (key, value) pairs
/// dict(iterable) -> new dictionary initialized as if via:
///    d = {}
///    for k, v in iterable:
///        d\[k\] = v
/// dict(**kwargs) -> new dictionary initialized with the name=value pairs
///    in the keyword argument list.  For example:  dict(one=1, two=2)
#[pyclass(module = false, name = "dict")]
#[derive(Default)]
pub struct PyDict {
    entries: DictContentType,
}
pub type PyDictRef = PyRef<PyDict>;

impl fmt::Debug for PyDict {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("dict")
    }
}

impl PyValue for PyDict {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.dict_type
    }
}

impl PyDict {
    pub fn new_ref(ctx: &PyContext) -> PyRef<Self> {
        PyRef::new_ref(Self::default(), ctx.types.dict_type.clone(), None)
    }
}

// Python dict methods:
#[allow(clippy::len_without_is_empty)]
#[pyimpl(with(AsMapping, Hashable, Comparable, Iterable), flags(BASETYPE))]
impl PyDict {
    /// escape hatch to access the underlying data structure directly. prefer adding a method on
    /// PyDict instead of using this
    pub(crate) fn _as_dict_inner(&self) -> &DictContentType {
        &self.entries
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyDict::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(
        &self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.update(dict_obj, kwargs, vm)
    }

    // Used in update and ior.
    fn merge_object(
        dict: &DictContentType,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let other = match other.downcast_exact(vm) {
            Ok(dict_other) => return Self::merge_dict(dict, dict_other, vm),
            Err(other) => other,
        };
        if let Some(keys) = vm.get_method(other.clone(), "keys") {
            let keys = vm.invoke(&keys?, ())?.get_iter(vm)?;
            while let PyIterReturn::Return(key) = keys.next(vm)? {
                let val = other.get_item(key.clone(), vm)?;
                dict.insert(vm, key, val)?;
            }
        } else {
            let iter = other.get_iter(vm)?;
            loop {
                fn err(vm: &VirtualMachine) -> PyBaseExceptionRef {
                    vm.new_value_error("Iterator must have exactly two elements".to_owned())
                }
                let element = match iter.next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(_) => break,
                };
                let elem_iter = element.get_iter(vm)?;
                let key = elem_iter.next(vm)?.into_result().map_err(|_| err(vm))?;
                let value = elem_iter.next(vm)?.into_result().map_err(|_| err(vm))?;
                if matches!(elem_iter.next(vm)?, PyIterReturn::Return(_)) {
                    return Err(err(vm));
                }
                dict.insert(vm, key, value)?;
            }
        }
        Ok(())
    }

    fn merge_dict(
        dict: &DictContentType,
        dict_other: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict_size = &dict_other.size();
        for (key, value) in &dict_other {
            dict.insert(vm, key, value)?;
        }
        if dict_other.entries.has_changed_size(dict_size) {
            return Err(vm.new_runtime_error("dict mutated during update".to_owned()));
        }
        Ok(())
    }

    #[pyclassmethod]
    fn fromkeys(
        class: PyTypeRef,
        iterable: ArgIterable,
        value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let dict = DictContentType::default();
        let value = value.unwrap_or_none(vm);
        for elem in iterable.iter(vm)? {
            let elem = elem?;
            dict.insert(vm, elem, value.clone())?;
        }
        PyDict { entries: dict }.into_ref_with_type(vm, class)
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.entries.is_empty()
    }

    fn inner_cmp(
        zelf: &PyObjectView<Self>,
        other: &PyObjectView<PyDict>,
        op: PyComparisonOp,
        item: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if op == PyComparisonOp::Ne {
            return Self::inner_cmp(zelf, other, PyComparisonOp::Eq, item, vm)
                .map(|x| x.map(|eq| !eq));
        }
        if !op.eval_ord(zelf.len().cmp(&other.len())) {
            return Ok(Implemented(false));
        }
        let (superset, subset) = if zelf.len() < other.len() {
            (other, zelf)
        } else {
            (zelf, other)
        };
        for (k, v1) in subset {
            match superset.get_item_option(k, vm)? {
                Some(v2) => {
                    if v1.is(&v2) {
                        continue;
                    }
                    if item && !vm.bool_eq(&v1, &v2)? {
                        return Ok(Implemented(false));
                    }
                }
                None => {
                    return Ok(Implemented(false));
                }
            }
        }
        Ok(Implemented(true))
    }

    #[pymethod(magic)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.entries.sizeof()
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.len());
            for (key, value) in zelf {
                let key_repr = &key.repr(vm)?;
                let value_repr = value.repr(vm)?;
                str_parts.push(format!("{}: {}", key_repr, value_repr));
            }

            format!("{{{}}}", str_parts.join(", "))
        } else {
            "{...}".to_owned()
        };
        Ok(s)
    }

    #[pymethod(magic)]
    fn contains(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.entries.contains(vm, &key)
    }

    #[pymethod(magic)]
    fn delitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.entries.delete(vm, key)
    }

    #[pymethod]
    fn clear(&self) {
        self.entries.clear()
    }

    #[pymethod]
    fn keys(zelf: PyRef<Self>) -> PyDictKeys {
        PyDictKeys::new(zelf)
    }

    #[pymethod]
    fn values(zelf: PyRef<Self>) -> PyDictValues {
        PyDictValues::new(zelf)
    }

    #[pymethod]
    fn items(zelf: PyRef<Self>) -> PyDictItems {
        PyDictItems::new(zelf)
    }

    #[pymethod(magic)]
    fn setitem(&self, key: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner_setitem_fast(key, value, vm)
    }

    /// Set item variant which can be called with multiple
    /// key types, such as str to name a notable one.
    fn inner_setitem_fast<K: DictKey + IntoPyObject>(
        &self,
        key: K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.insert(vm, key, value)
    }

    #[pymethod(magic)]
    #[cfg_attr(feature = "flame-it", flame("PyDictRef"))]
    fn getitem(zelf: PyRef<Self>, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = zelf.inner_getitem_option(key.clone(), zelf.exact_dict(vm), vm)? {
            Ok(value)
        } else {
            Err(vm.new_key_error(key))
        }
    }

    #[pymethod]
    fn get(
        &self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.get(vm, &key)? {
            Some(value) => Ok(value),
            None => Ok(default.unwrap_or_none(vm)),
        }
    }

    #[pymethod]
    fn setdefault(
        &self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.entries
            .setdefault(vm, key, || default.unwrap_or_none(vm))
    }

    pub fn get_or_insert(
        &self,
        vm: &VirtualMachine,
        key: PyObjectRef,
        default: impl FnOnce() -> PyObjectRef,
    ) -> PyResult {
        self.entries.setdefault(vm, key, default)
    }

    #[pymethod]
    pub fn copy(&self) -> PyDict {
        PyDict {
            entries: self.entries.clone(),
        }
    }

    #[pymethod]
    fn update(
        &self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let OptionalArg::Present(dict_obj) = dict_obj {
            Self::merge_object(&self.entries, dict_obj, vm)?;
        }
        for (key, value) in kwargs.into_iter() {
            self.entries.insert(vm, vm.new_pyobj(key), value)?;
        }
        Ok(())
    }

    #[pymethod(magic)]
    fn ior(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyDict::merge_object(&zelf.entries, other, vm)?;
        Ok(zelf)
    }

    #[pymethod(magic)]
    fn ror(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let dicted: Result<PyDictRef, _> = other.downcast();
        if let Ok(other) = dicted {
            let other_cp = other.copy();
            PyDict::merge_dict(&other_cp.entries, zelf, vm)?;
            return Ok(other_cp.into_object(vm));
        }
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod(magic)]
    fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let dicted: Result<PyDictRef, _> = other.downcast();
        if let Ok(other) = dicted {
            let self_cp = self.copy();
            PyDict::merge_dict(&self_cp.entries, other, vm)?;
            return Ok(self_cp.into_object(vm));
        }
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod]
    fn pop(
        &self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.pop(vm, &key)? {
            Some(value) => Ok(value),
            None => default.ok_or_else(|| vm.new_key_error(key)),
        }
    }

    #[pymethod]
    fn popitem(&self, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let (key, value) = self.entries.pop_back().ok_or_else(|| {
            let err_msg = vm
                .ctx
                .new_str(ascii!("popitem(): dictionary is empty"))
                .into();
            vm.new_key_error(err_msg)
        })?;
        Ok((key, value))
    }

    pub fn from_attributes(attrs: PyAttributes, vm: &VirtualMachine) -> PyResult<Self> {
        let dict = DictContentType::default();

        for (key, value) in attrs {
            dict.insert(vm, vm.new_pyobj(key), value)?;
        }

        Ok(PyDict { entries: dict })
    }

    pub fn contains_key<K: IntoPyObject>(&self, key: K, vm: &VirtualMachine) -> bool {
        let key = key.into_pyobject(vm);
        self.entries.contains(vm, &key).unwrap()
    }

    pub fn size(&self) -> dictdatatype::DictSize {
        self.entries.size()
    }

    #[pymethod(magic)]
    fn reversed(zelf: PyRef<Self>) -> PyDictReverseKeyIterator {
        PyDictReverseKeyIterator::new(zelf)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl AsMapping for PyDict {
    fn as_mapping(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: Some(Self::ass_subscript),
        }
    }

    #[inline]
    fn length(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        Self::downcast_ref(&zelf, vm).map(|zelf| Ok(zelf.len()))?
    }

    #[inline]
    fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::downcast(zelf, vm).map(|zelf| Self::getitem(zelf, needle, vm))?
    }

    #[inline]
    fn ass_subscript(
        zelf: PyObjectRef,
        needle: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::downcast_ref(&zelf, vm).map(|zelf| match value {
            Some(value) => zelf.setitem(needle, value, vm),
            None => zelf.delitem(needle, vm),
        })?
    }
}

impl Comparable for PyDict {
    fn cmp(
        zelf: &PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            Self::inner_cmp(zelf, other, PyComparisonOp::Eq, true, vm)
        })
    }
}

impl Unhashable for PyDict {}

impl Iterable for PyDict {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyDictKeyIterator::new(zelf).into_object(vm))
    }
}

impl PyObjectView<PyDict> {
    #[inline]
    fn exact_dict(&self, vm: &VirtualMachine) -> bool {
        self.class().is(&vm.ctx.types.dict_type)
    }

    /// Return an optional inner item, or an error (can be key error as well)
    #[inline]
    fn inner_getitem_option<K: DictKey + IntoPyObject>(
        &self,
        key: K,
        exact: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(value) = self.entries.get(vm, &key)? {
            return Ok(Some(value));
        }

        if !exact {
            if let Some(method_or_err) = vm.get_method(self.to_owned().into(), "__missing__") {
                let method = method_or_err?;
                return vm.invoke(&method, (key,)).map(Some);
            }
        }
        Ok(None)
    }

    /// Take a python dictionary and convert it to attributes.
    pub fn to_attributes(&self) -> PyAttributes {
        let mut attrs = PyAttributes::default();
        for (key, value) in self {
            let key: PyStrRef = key.downcast().expect("dict has non-string keys");
            attrs.insert(key.as_str().to_owned(), value);
        }
        attrs
    }

    /// This function can be used to get an item without raising the
    /// KeyError, so we can simply check upon the result being Some
    /// python value, or None.
    /// Note that we can pass any type which implements the DictKey
    /// trait. Notable examples are String and PyObjectRef.
    pub fn get_item_option<K: IntoPyObject + DictKey>(
        &self,
        key: K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        // Test if this object is a true dict, or maybe a subclass?
        // If it is a dict, we can directly invoke inner_get_item_option,
        // and prevent the creation of the KeyError exception.
        // Also note, that we prevent the creation of a full PyStr object
        // if we lookup local names (which happens all of the time).
        self._get_item_option_inner(key, self.exact_dict(vm), vm)
    }

    #[inline]
    fn _get_item_option_inner<K: IntoPyObject + DictKey>(
        &self,
        key: K,
        exact: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if exact {
            // We can take the short path here!
            self.entries.get(vm, &key)
        } else {
            // Fall back to full get_item with KeyError checking
            self._subcls_getitem_option(key.into_pyobject(vm), vm)
        }
    }

    #[cold]
    fn _subcls_getitem_option(
        &self,
        key: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match self.get_item(key, vm) {
            Ok(value) => Ok(Some(value)),
            Err(exc) if exc.isinstance(&vm.ctx.exceptions.key_error) => Ok(None),
            Err(exc) => Err(exc),
        }
    }

    pub fn get_chain<K: IntoPyObject + DictKey + Clone>(
        &self,
        other: &Self,
        key: K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let self_exact = self.class().is(&vm.ctx.types.dict_type);
        let other_exact = self.class().is(&vm.ctx.types.dict_type);
        if self_exact && other_exact {
            self.entries.get_chain(&other.entries, vm, &key)
        } else if let Some(value) = self._get_item_option_inner(key.clone(), self_exact, vm)? {
            Ok(Some(value))
        } else {
            other._get_item_option_inner(key, other_exact, vm)
        }
    }
}

impl<K> ItemProtocol<K> for PyObjectView<PyDict>
where
    K: DictKey + IntoPyObject,
{
    fn get_item(&self, key: K, vm: &VirtualMachine) -> PyResult {
        self.as_object().get_item(key, vm)
    }

    fn set_item(&self, key: K, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.class().is(&vm.ctx.types.dict_type) {
            self.inner_setitem_fast(key, value, vm)
        } else {
            // Fall back to slow path if we are in a dict subclass:
            self.as_object().set_item(key, value, vm)
        }
    }

    fn del_item(&self, key: K, vm: &VirtualMachine) -> PyResult<()> {
        if self.class().is(&vm.ctx.types.dict_type) {
            self.entries.delete(vm, key)
        } else {
            // Fall back to slow path if we are in a dict subclass:
            self.as_object().del_item(key, vm)
        }
    }
}

impl<K> ItemProtocol<K> for PyDictRef
where
    K: DictKey + IntoPyObject,
{
    fn get_item(&self, key: K, vm: &VirtualMachine) -> PyResult {
        (**self).get_item(key, vm)
    }

    fn set_item(&self, key: K, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        (**self).set_item(key, value, vm)
    }

    fn del_item(&self, key: K, vm: &VirtualMachine) -> PyResult<()> {
        (**self).del_item(key, vm)
    }
}

// Implement IntoIterator so that we can easily iterate dictionaries from rust code.
impl IntoIterator for PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self)
    }
}

impl IntoIterator for &PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self.clone())
    }
}

impl IntoIterator for &PyObjectView<PyDict> {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self.to_owned())
    }
}

pub struct DictIter {
    dict: PyDictRef,
    position: usize,
}

impl DictIter {
    pub fn new(dict: PyDictRef) -> DictIter {
        DictIter { dict, position: 0 }
    }
}

impl Iterator for DictIter {
    type Item = (PyObjectRef, PyObjectRef);

    fn next(&mut self) -> Option<Self::Item> {
        let (position, key, value) = self.dict.entries.next_entry(self.position)?;
        self.position = position;
        Some((key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let l = self.dict.entries.len_from_entry_index(self.position);
        (l, Some(l))
    }
}

#[pyimpl]
trait DictView: PyValue + PyClassDef + Iterable
where
    Self::ReverseIter: PyValue,
{
    type ReverseIter;

    fn dict(&self) -> &PyDictRef;
    fn item(vm: &VirtualMachine, key: PyObjectRef, value: PyObjectRef) -> PyObjectRef;

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.dict().len()
    }

    #[allow(clippy::redundant_closure_call)]
    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.len());
            for (key, value) in zelf.dict().clone() {
                let s = &Self::item(vm, key, value).repr(vm)?;
                str_parts.push(s.as_str().to_owned());
            }
            format!("{}([{}])", Self::NAME, str_parts.join(", "))
        } else {
            "{...}".to_owned()
        };
        Ok(s)
    }

    #[pymethod(magic)]
    fn reversed(&self) -> Self::ReverseIter;
}

macro_rules! dict_view {
    ( $name: ident, $iter_name: ident, $reverse_iter_name: ident,
      $class: ident, $iter_class: ident, $reverse_iter_class: ident,
      $class_name: literal, $iter_class_name: literal, $reverse_iter_class_name: literal,
      $result_fn: expr) => {
        #[pyclass(module = false, name = $class_name)]
        #[derive(Debug)]
        pub(crate) struct $name {
            pub dict: PyDictRef,
        }

        impl $name {
            pub fn new(dict: PyDictRef) -> Self {
                $name { dict }
            }
        }

        impl DictView for $name {
            type ReverseIter = $reverse_iter_name;
            fn dict(&self) -> &PyDictRef {
                &self.dict
            }
            fn item(vm: &VirtualMachine, key: PyObjectRef, value: PyObjectRef) -> PyObjectRef {
                $result_fn(vm, key, value)
            }
            fn reversed(&self) -> Self::ReverseIter {
                $reverse_iter_name::new(self.dict.clone())
            }
        }

        impl Iterable for $name {
            fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
                Ok($iter_name::new(zelf.dict.clone()).into_object(vm))
            }
        }

        impl PyValue for $name {
            fn class(vm: &VirtualMachine) -> &PyTypeRef {
                &vm.ctx.types.$class
            }
        }

        #[pyclass(module = false, name = $iter_class_name)]
        #[derive(Debug)]
        pub(crate) struct $iter_name {
            pub size: dictdatatype::DictSize,
            pub internal: PyMutex<PositionIterInternal<PyDictRef>>,
        }

        impl PyValue for $iter_name {
            fn class(vm: &VirtualMachine) -> &PyTypeRef {
                &vm.ctx.types.$iter_class
            }
        }

        #[pyimpl(with(Constructor, IterNext))]
        impl $iter_name {
            fn new(dict: PyDictRef) -> Self {
                $iter_name {
                    size: dict.size(),
                    internal: PyMutex::new(PositionIterInternal::new(dict, 0)),
                }
            }

            #[pymethod(magic)]
            fn length_hint(&self) -> usize {
                self.internal.lock().length_hint(|_| self.size.entries_size)
            }

            #[allow(clippy::redundant_closure_call)]
            #[pymethod(magic)]
            fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
                let iter = builtins_iter(vm).to_owned();
                let internal = zelf.internal.lock();
                let entries = match &internal.status {
                    IterStatus::Active(dict) => dict
                        .into_iter()
                        .map(|(key, value)| ($result_fn)(vm, key, value))
                        .collect::<Vec<_>>(),
                    IterStatus::Exhausted => vec![],
                };
                vm.new_tuple((iter, (vm.ctx.new_list(entries),)))
            }
        }
        impl Unconstructible for $iter_name {}

        impl IterNextIterable for $iter_name {}
        impl IterNext for $iter_name {
            #[allow(clippy::redundant_closure_call)]
            fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
                let mut internal = zelf.internal.lock();
                let next = if let IterStatus::Active(dict) = &internal.status {
                    if dict.entries.has_changed_size(&zelf.size) {
                        internal.status = IterStatus::Exhausted;
                        return Err(vm.new_runtime_error(
                            "dictionary changed size during iteration".to_owned(),
                        ));
                    }
                    match dict.entries.next_entry(internal.position) {
                        Some((position, key, value)) => {
                            internal.position = position;
                            PyIterReturn::Return(($result_fn)(vm, key, value))
                        }
                        None => {
                            internal.status = IterStatus::Exhausted;
                            PyIterReturn::StopIteration(None)
                        }
                    }
                } else {
                    PyIterReturn::StopIteration(None)
                };
                Ok(next)
            }
        }

        #[pyclass(module = false, name = $reverse_iter_class_name)]
        #[derive(Debug)]
        pub(crate) struct $reverse_iter_name {
            pub size: dictdatatype::DictSize,
            internal: PyMutex<PositionIterInternal<PyDictRef>>,
        }

        impl PyValue for $reverse_iter_name {
            fn class(vm: &VirtualMachine) -> &PyTypeRef {
                &vm.ctx.types.$reverse_iter_class
            }
        }

        #[pyimpl(with(Constructor, IterNext))]
        impl $reverse_iter_name {
            fn new(dict: PyDictRef) -> Self {
                let size = dict.size();
                let position = size.entries_size.saturating_sub(1);
                $reverse_iter_name {
                    size,
                    internal: PyMutex::new(PositionIterInternal::new(dict, position)),
                }
            }

            #[pymethod(magic)]
            fn length_hint(&self) -> usize {
                self.internal
                    .lock()
                    .rev_length_hint(|_| self.size.entries_size)
            }
        }
        impl Unconstructible for $reverse_iter_name {}

        impl IterNextIterable for $reverse_iter_name {}
        impl IterNext for $reverse_iter_name {
            #[allow(clippy::redundant_closure_call)]
            fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
                let mut internal = zelf.internal.lock();
                let next = if let IterStatus::Active(dict) = &internal.status {
                    if dict.entries.has_changed_size(&zelf.size) {
                        internal.status = IterStatus::Exhausted;
                        return Err(vm.new_runtime_error(
                            "dictionary changed size during iteration".to_owned(),
                        ));
                    }
                    match dict.entries.prev_entry(internal.position) {
                        Some((position, key, value)) => {
                            if internal.position == position {
                                internal.status = IterStatus::Exhausted;
                            } else {
                                internal.position = position;
                            }
                            PyIterReturn::Return(($result_fn)(vm, key, value))
                        }
                        None => {
                            internal.status = IterStatus::Exhausted;
                            PyIterReturn::StopIteration(None)
                        }
                    }
                } else {
                    PyIterReturn::StopIteration(None)
                };
                Ok(next)
            }
        }
    };
}

dict_view! {
    PyDictKeys,
    PyDictKeyIterator,
    PyDictReverseKeyIterator,
    dict_keys_type,
    dict_keyiterator_type,
    dict_reversekeyiterator_type,
    "dict_keys",
    "dict_keyiterator",
    "dict_reversekeyiterator",
    |_vm: &VirtualMachine, key: PyObjectRef, _value: PyObjectRef| key
}

dict_view! {
    PyDictValues,
    PyDictValueIterator,
    PyDictReverseValueIterator,
    dict_values_type,
    dict_valueiterator_type,
    dict_reversevalueiterator_type,
    "dict_values",
    "dict_valueiterator",
    "dict_reversevalueiterator",
    |_vm: &VirtualMachine, _key: PyObjectRef, value: PyObjectRef| value
}

dict_view! {
    PyDictItems,
    PyDictItemIterator,
    PyDictReverseItemIterator,
    dict_items_type,
    dict_itemiterator_type,
    dict_reverseitemiterator_type,
    "dict_items",
    "dict_itemiterator",
    "dict_reverseitemiterator",
    |vm: &VirtualMachine, key: PyObjectRef, value: PyObjectRef|
        vm.new_tuple((key, value)).into()
}

// Set operations defined on set-like views of the dictionary.
#[pyimpl]
trait ViewSetOps: DictView {
    fn to_set(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let len = zelf.dict().len();
        let zelf: PyObjectRef = Self::iter(zelf, vm)?;
        let iter = PyIterIter::new(vm, zelf, Some(len));
        PySetInner::from_iter(iter, vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySet> {
        let zelf = Self::to_set(zelf, vm)?;
        let inner = zelf.symmetric_difference(other, vm)?;
        Ok(PySet { inner })
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySet> {
        let zelf = Self::to_set(zelf, vm)?;
        let inner = zelf.intersection(other, vm)?;
        Ok(PySet { inner })
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySet> {
        let zelf = Self::to_set(zelf, vm)?;
        let inner = zelf.union(other, vm)?;
        Ok(PySet { inner })
    }

    #[pymethod(magic)]
    fn sub(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySet> {
        let zelf = Self::to_set(zelf, vm)?;
        let inner = zelf.difference(other, vm)?;
        Ok(PySet { inner })
    }

    #[pymethod(magic)]
    fn rsub(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySet> {
        let left = PySetInner::from_iter(other.iter(vm)?, vm)?;
        let right = ArgIterable::try_from_object(vm, Self::iter(zelf, vm)?)?;
        let inner = left.difference(right, vm)?;
        Ok(PySet { inner })
    }

    fn cmp(
        zelf: &PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        match_class!(match other {
            ref dictview @ Self => {
                PyDict::inner_cmp(
                    zelf.dict(),
                    dictview.dict(),
                    op,
                    !zelf.class().is(&vm.ctx.types.dict_keys_type),
                    vm,
                )
            }
            ref _set @ PySet => {
                let inner = Self::to_set(zelf.to_owned(), vm)?;
                let zelf_set = PySet { inner }.into_object(vm);
                PySet::cmp(zelf_set.downcast_ref().unwrap(), other, op, vm)
            }
            _ => {
                Ok(NotImplemented)
            }
        })
    }

    #[pymethod]
    fn isdisjoint(zelf: PyRef<Self>, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        // TODO: to_set is an expensive operation. After merging #3316 rewrite implementation using PySequence_Contains.
        let zelf = Self::to_set(zelf, vm)?;
        let result = zelf.isdisjoint(other, vm)?;
        Ok(result)
    }
}

impl ViewSetOps for PyDictKeys {}
#[pyimpl(with(DictView, Constructor, Comparable, Iterable, ViewSetOps))]
impl PyDictKeys {
    #[pymethod(magic)]
    fn contains(zelf: PyRef<Self>, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        zelf.dict().contains(key, vm)
    }
}
impl Unconstructible for PyDictKeys {}

impl Comparable for PyDictKeys {
    fn cmp(
        zelf: &PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        ViewSetOps::cmp(zelf, other, op, vm)
    }
}

impl ViewSetOps for PyDictItems {}
#[pyimpl(with(DictView, Constructor, Comparable, Iterable, ViewSetOps))]
impl PyDictItems {
    #[pymethod(magic)]
    fn contains(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        let needle = match_class! {
            match needle {
                tuple @ PyTuple => tuple,
                _ => return Ok(false),
            }
        };
        if needle.len() != 2 {
            return Ok(false);
        }
        let key = needle.fast_getitem(0);
        if !zelf.dict().contains(key.clone(), vm)? {
            return Ok(false);
        }
        let value = needle.fast_getitem(1);
        let found = PyDict::getitem(zelf.dict().clone(), key, vm)?;
        vm.identical_or_equal(&found, &value)
    }
}
impl Unconstructible for PyDictItems {}

impl Comparable for PyDictItems {
    fn cmp(
        zelf: &PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        ViewSetOps::cmp(zelf, other, op, vm)
    }
}

#[pyimpl(with(DictView, Constructor, Iterable))]
impl PyDictValues {}
impl Unconstructible for PyDictValues {}

pub(crate) fn init(context: &PyContext) {
    PyDict::extend_class(context, &context.types.dict_type);
    PyDictKeys::extend_class(context, &context.types.dict_keys_type);
    PyDictKeyIterator::extend_class(context, &context.types.dict_keyiterator_type);
    PyDictReverseKeyIterator::extend_class(context, &context.types.dict_reversekeyiterator_type);
    PyDictValues::extend_class(context, &context.types.dict_values_type);
    PyDictValueIterator::extend_class(context, &context.types.dict_valueiterator_type);
    PyDictReverseValueIterator::extend_class(
        context,
        &context.types.dict_reversevalueiterator_type,
    );
    PyDictItems::extend_class(context, &context.types.dict_items_type);
    PyDictItemIterator::extend_class(context, &context.types.dict_itemiterator_type);
    PyDictReverseItemIterator::extend_class(context, &context.types.dict_reverseitemiterator_type);
}

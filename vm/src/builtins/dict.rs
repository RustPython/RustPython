use super::{
    set::PySetInner, IterStatus, PositionIterInternal, PyBaseExceptionRef, PyGenericAlias, PySet,
    PyStrRef, PyTupleRef, PyType, PyTypeRef,
};
use crate::{
    builtins::{
        iter::{builtins_iter, builtins_reversed},
        type_::PyAttributes,
        PyTuple,
    },
    class::{PyClassDef, PyClassImpl},
    common::ascii,
    convert::ToPyObject,
    dictdatatype::{self, DictKey},
    function::{
        ArgIterable, FuncArgs, KwArgs, OptionalArg, PyArithmeticValue::*, PyComparisonValue,
    },
    protocol::{PyIterIter, PyIterReturn, PyMappingMethods, PySequenceMethods},
    recursion::ReprGuard,
    types::{
        AsMapping, AsSequence, Callable, Comparable, Constructor, Hashable, Initializer, IterNext,
        IterNextIterable, Iterable, PyComparisonOp, Unconstructible, Unhashable,
    },
    vm::VirtualMachine,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
};
use rustpython_common::lock::PyMutex;
use std::{borrow::Cow, fmt};

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

impl PyPayload for PyDict {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.dict_type
    }
}

impl PyDict {
    pub fn new_ref(ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::default(), ctx.types.dict_type.clone(), None)
    }

    /// escape hatch to access the underlying data structure directly. prefer adding a method on
    /// PyDict instead of using this
    pub(crate) fn _as_dict_inner(&self) -> &DictContentType {
        &self.entries
    }

    pub(crate) fn from_entries(entries: DictContentType) -> Self {
        Self { entries }
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
        if let Some(keys) = vm.get_method(other.clone(), vm.ctx.intern_str("keys")) {
            let keys = vm.invoke(&keys?, ())?.get_iter(vm)?;
            while let PyIterReturn::Return(key) = keys.next(vm)? {
                let val = other.get_item(&*key, vm)?;
                dict.insert(vm, &*key, val)?;
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
                dict.insert(vm, &*key, value)?;
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
            dict.insert(vm, &*key, value)?;
        }
        if dict_other.entries.has_changed_size(dict_size) {
            return Err(vm.new_runtime_error("dict mutated during update".to_owned()));
        }
        Ok(())
    }

    fn inner_cmp(
        zelf: &Py<Self>,
        other: &Py<PyDict>,
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
            match superset.get_item_opt(&*k, vm)? {
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

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// Set item variant which can be called with multiple
    /// key types, such as str to name a notable one.
    pub(crate) fn inner_setitem<K: DictKey + ?Sized>(
        &self,
        key: &K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.insert(vm, key, value)
    }

    pub(crate) fn inner_delitem<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.delete(vm, key)
    }

    pub fn get_or_insert(
        &self,
        vm: &VirtualMachine,
        key: PyObjectRef,
        default: impl FnOnce() -> PyObjectRef,
    ) -> PyResult {
        self.entries.setdefault(vm, &*key, default)
    }

    pub fn from_attributes(attrs: PyAttributes, vm: &VirtualMachine) -> PyResult<Self> {
        let entries = DictContentType::default();

        for (key, value) in attrs {
            entries.insert(vm, key, value)?;
        }

        Ok(Self { entries })
    }

    pub fn contains_key<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> bool {
        self.entries.contains(vm, key).unwrap()
    }

    pub fn size(&self) -> dictdatatype::DictSize {
        self.entries.size()
    }

    pub(crate) const MAPPING_METHODS: PyMappingMethods = PyMappingMethods {
        length: Some(|mapping, _vm| Ok(Self::mapping_downcast(mapping).len())),
        subscript: Some(|mapping, needle, vm| {
            Self::mapping_downcast(mapping).inner_getitem(needle, vm)
        }),
        ass_subscript: Some(|mapping, needle, value, vm| {
            let zelf = Self::mapping_downcast(mapping);
            if let Some(value) = value {
                zelf.inner_setitem(needle, value, vm)
            } else {
                zelf.inner_delitem(needle, vm)
            }
        }),
    };
}

// Python dict methods:
#[allow(clippy::len_without_is_empty)]
#[pyimpl(
    with(
        Constructor,
        Initializer,
        AsMapping,
        Hashable,
        Comparable,
        Iterable,
        AsSequence
    ),
    flags(BASETYPE)
)]
impl PyDict {
    #[pyclassmethod]
    fn fromkeys(
        class: PyTypeRef,
        iterable: ArgIterable,
        value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let value = value.unwrap_or_none(vm);
        let d = PyType::call(&class, ().into(), vm)?;
        match d.downcast_exact::<PyDict>(vm) {
            Ok(pydict) => {
                for key in iterable.iter(vm)? {
                    pydict.setitem(key?, value.clone(), vm)?;
                }
                Ok(pydict.to_pyobject(vm))
            }
            Err(pyobj) => {
                for key in iterable.iter(vm)? {
                    pyobj.set_item(&*key?, value.clone(), vm)?;
                }
                Ok(pyobj)
            }
        }
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.entries.is_empty()
    }

    #[pymethod(magic)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.entries.sizeof()
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
        self.entries.contains(vm, &*key)
    }

    #[pymethod(magic)]
    fn delitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner_delitem(&*key, vm)
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
        self.inner_setitem(&*key, value, vm)
    }

    #[pymethod(magic)]
    #[cfg_attr(feature = "flame-it", flame("PyDictRef"))]
    fn getitem(zelf: PyRef<Self>, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        zelf.inner_getitem(&*key, vm)
    }

    #[pymethod]
    fn get(
        &self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match self.entries.get(vm, &*key)? {
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
            .setdefault(vm, &*key, || default.unwrap_or_none(vm))
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
            self.entries.insert(vm, &key, value)?;
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
            return Ok(other_cp.into_pyobject(vm));
        }
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod(magic)]
    fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let dicted: Result<PyDictRef, _> = other.downcast();
        if let Ok(other) = dicted {
            let self_cp = self.copy();
            PyDict::merge_dict(&self_cp.entries, other, vm)?;
            return Ok(self_cp.into_pyobject(vm));
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
        match self.entries.pop(vm, &*key)? {
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

    #[pymethod(magic)]
    fn reversed(zelf: PyRef<Self>) -> PyDictReverseKeyIterator {
        PyDictReverseKeyIterator::new(zelf)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl Constructor for PyDict {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyDict::default()
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl Initializer for PyDict {
    type Args = (OptionalArg<PyObjectRef>, KwArgs);

    fn init(
        zelf: PyRef<Self>,
        (dict_obj, kwargs): Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        zelf.update(dict_obj, kwargs, vm)
    }
}

impl AsMapping for PyDict {
    fn as_mapping(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        Self::MAPPING_METHODS
    }
}

impl AsSequence for PyDict {
    fn as_sequence(_zelf: &Py<Self>, _vm: &VirtualMachine) -> Cow<'static, PySequenceMethods> {
        Cow::Borrowed(&Self::SEQUENCE_METHODS)
    }
}

impl PyDict {
    const SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
        contains: Some(|seq, target, vm| Self::sequence_downcast(seq).entries.contains(vm, target)),
        ..*PySequenceMethods::not_implemented()
    };
}

impl Comparable for PyDict {
    fn cmp(
        zelf: &Py<Self>,
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
        Ok(PyDictKeyIterator::new(zelf).into_pyobject(vm))
    }
}

impl Py<PyDict> {
    #[inline]
    fn exact_dict(&self, vm: &VirtualMachine) -> bool {
        self.class().is(&vm.ctx.types.dict_type)
    }

    fn missing_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        vm.get_method(self.to_owned().into(), identifier!(vm, __missing__))
            .map(|methods| vm.invoke(&methods?, (key.to_pyobject(vm),)))
            .transpose()
    }

    #[inline]
    fn inner_getitem<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        if let Some(value) = self.entries.get(vm, key)? {
            Ok(value)
        } else if let Some(value) = self.missing_opt(key, vm)? {
            Ok(value)
        } else {
            Err(vm.new_key_error(key.to_pyobject(vm)))
        }
    }

    /// Take a python dictionary and convert it to attributes.
    pub fn to_attributes(&self, vm: &VirtualMachine) -> PyAttributes {
        let mut attrs = PyAttributes::default();
        for (key, value) in self {
            // TODO: use PyRefExact for interning
            let key: PyStrRef = key.downcast().expect("dict has non-string keys");
            attrs.insert(vm.ctx.intern_str(key.as_str()), value);
        }
        attrs
    }

    pub fn get_item_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if self.exact_dict(vm) {
            self.entries.get(vm, key)
            // FIXME: check __missing__?
        } else {
            match self.as_object().get_item(key, vm) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.fast_isinstance(&vm.ctx.exceptions.key_error) => {
                    self.missing_opt(key, vm)
                }
                Err(e) => Err(e),
            }
        }
    }

    pub fn get_item<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult {
        if self.exact_dict(vm) {
            self.inner_getitem(key, vm)
        } else {
            self.as_object().get_item(key, vm)
        }
    }

    pub fn set_item<K: DictKey + ?Sized>(
        &self,
        key: &K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.exact_dict(vm) {
            self.inner_setitem(key, value, vm)
        } else {
            self.as_object().set_item(key, value, vm)
        }
    }

    pub fn del_item<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult<()> {
        if self.exact_dict(vm) {
            self.inner_delitem(key, vm)
        } else {
            self.as_object().del_item(key, vm)
        }
    }

    pub fn get_chain<K: DictKey + ?Sized>(
        &self,
        other: &Self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let self_exact = self.exact_dict(vm);
        let other_exact = other.exact_dict(vm);
        if self_exact && other_exact {
            self.entries.get_chain(&other.entries, vm, key)
        } else if let Some(value) = self.get_item_opt(key, vm)? {
            Ok(Some(value))
        } else {
            other.get_item_opt(key, vm)
        }
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

impl IntoIterator for &Py<PyDict> {
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
trait DictView: PyPayload + PyClassDef + Iterable
where
    Self::ReverseIter: PyPayload,
{
    type ReverseIter;

    fn dict(&self) -> &PyDictRef;
    fn item(vm: &VirtualMachine, key: PyObjectRef, value: PyObjectRef) -> PyObjectRef;

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.dict().len()
    }

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
                Ok($iter_name::new(zelf.dict.clone()).into_pyobject(vm))
            }
        }

        impl PyPayload for $name {
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

        impl PyPayload for $iter_name {
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
            fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

        impl PyPayload for $reverse_iter_name {
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

            #[allow(clippy::redundant_closure_call)]
            #[pymethod(magic)]
            fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
                let iter = builtins_reversed(vm).to_owned();
                let internal = zelf.internal.lock();
                // TODO: entries must be reversed too
                let entries = match &internal.status {
                    IterStatus::Active(dict) => dict
                        .into_iter()
                        .map(|(key, value)| ($result_fn)(vm, key, value))
                        .collect::<Vec<_>>(),
                    IterStatus::Exhausted => vec![],
                };
                vm.new_tuple((iter, (vm.ctx.new_list(entries),)))
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
            fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
        zelf: &Py<Self>,
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
                let zelf_set = PySet { inner }.into_pyobject(vm);
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
#[pyimpl(with(DictView, Constructor, Comparable, Iterable, ViewSetOps, AsSequence))]
impl PyDictKeys {
    #[pymethod(magic)]
    fn contains(zelf: PyRef<Self>, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        zelf.dict().contains(key, vm)
    }
}
impl Unconstructible for PyDictKeys {}

impl Comparable for PyDictKeys {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        ViewSetOps::cmp(zelf, other, op, vm)
    }
}

impl AsSequence for PyDictKeys {
    fn as_sequence(_zelf: &Py<Self>, _vm: &VirtualMachine) -> Cow<'static, PySequenceMethods> {
        Cow::Borrowed(&Self::SEQUENCE_METHODS)
    }
}
impl PyDictKeys {
    const SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
        length: Some(|seq, _vm| Ok(Self::sequence_downcast(seq).len())),
        contains: Some(|seq, target, vm| {
            Self::sequence_downcast(seq)
                .dict
                .entries
                .contains(vm, target)
        }),
        ..*PySequenceMethods::not_implemented()
    };
}

impl ViewSetOps for PyDictItems {}
#[pyimpl(with(DictView, Constructor, Comparable, Iterable, ViewSetOps, AsSequence))]
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
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        ViewSetOps::cmp(zelf, other, op, vm)
    }
}

impl AsSequence for PyDictItems {
    fn as_sequence(_zelf: &Py<Self>, _vm: &VirtualMachine) -> Cow<'static, PySequenceMethods> {
        Cow::Borrowed(&Self::SEQUENCE_METHODS)
    }
}
impl PyDictItems {
    const SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
        length: Some(|seq, _vm| Ok(Self::sequence_downcast(seq).len())),
        contains: Some(|seq, target, vm| {
            Self::sequence_downcast(seq)
                .dict
                .entries
                .contains(vm, target)
        }),
        ..*PySequenceMethods::not_implemented()
    };
}

#[pyimpl(with(DictView, Constructor, Iterable, AsSequence))]
impl PyDictValues {}
impl Unconstructible for PyDictValues {}

impl AsSequence for PyDictValues {
    fn as_sequence(_zelf: &Py<Self>, _vm: &VirtualMachine) -> Cow<'static, PySequenceMethods> {
        Cow::Borrowed(&Self::SEQUENCE_METHODS)
    }
}
impl PyDictValues {
    const SEQUENCE_METHODS: PySequenceMethods = PySequenceMethods {
        length: Some(|seq, _vm| Ok(Self::sequence_downcast(seq).len())),
        ..*PySequenceMethods::not_implemented()
    };
}

pub(crate) fn init(context: &Context) {
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

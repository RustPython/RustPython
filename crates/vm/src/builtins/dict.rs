use super::{
    PyBaseExceptionRef, PyFrozenSet, PyGenericAlias, PyMappingProxy, PySet, PyStr, PyStrRef,
    PyTupleRef, PyType, PyTypeRef, set, set::PySetInner,
};
use crate::common::lock::LazyLock;
use crate::object::{Traverse, TraverseFn};
use crate::{
    AsObject, Context, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromObject, atomic_func,
    builtins::{PyList, PyTuple, iter::builtins_iter, type_::PyAttributes},
    class::{PyClassDef, PyClassImpl},
    common::{
        ascii,
        hash::{self, PyHash},
    },
    dict_inner::{self, DictKey},
    function::{ArgIterable, FuncArgs, KwArgs, OptionalArg, PyArithmeticValue, PyComparisonValue},
    protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    recursion::ReprGuard,
    types::{
        AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, DefaultConstructor,
        Hashable, Initializer, IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
    },
    vm::VirtualMachine,
};
use alloc::fmt;
use core::borrow::Borrow;
use core::cell::Cell;
use core::ptr::NonNull;
use rustpython_common::atomic::{Ordering, PyAtomic, Radium};
use rustpython_common::lock::PyMutex;
use rustpython_common::wtf8::Wtf8Buf;
use rustpython_common::wtf8::wtf8_concat;

pub(crate) type DictContentType = dict_inner::Dict;

pub(crate) fn key_hash<K: DictKey + ?Sized>(
    key: &K,
    mapping_name: &str,
    vm: &VirtualMachine,
) -> PyResult<PyHash> {
    key.key_hash(vm)
        .map_err(|exc| contextualize_hash_error(key, mapping_name, "key", exc, vm))
}

pub(crate) fn contextualize_hash_error<K: DictKey + ?Sized>(
    key: &K,
    container: &str,
    role: &str,
    exc: PyBaseExceptionRef,
    vm: &VirtualMachine,
) -> PyBaseExceptionRef {
    let format_error = || -> PyResult<PyBaseExceptionRef> {
        match exc {
            // Exact TypeError only: exception subclasses retain their identity.
            exc if exc.class().is(vm.ctx.exceptions.type_error) => {
                let name = key.to_pyobject(vm).class().fully_qualified_name(vm)?;
                // Match BaseException_str without the diagnostic formatter's
                // fallback: argument __str__/__repr__ failures must propagate.
                let args = exc.args();
                let message = match args.as_slice() {
                    [] => vm.ctx.empty_str.to_owned(),
                    [arg] => arg.str(vm)?,
                    _ => args.as_object().str(vm)?,
                };
                Ok(vm.new_type_error(wtf8_concat!(
                    "cannot use '",
                    name,
                    "' as a ",
                    container,
                    " ",
                    role,
                    " (",
                    message.as_wtf8(),
                    ")"
                )))
            }
            exc => Ok(exc),
        }
    };
    format_error().unwrap_or_else(|exc| exc)
}

#[pyclass(module = false, name = "dict", unhashable = true, traverse = "manual")]
#[derive(Default)]
// OrderedDict contains eight-byte-aligned fields on 32-bit targets too.
#[repr(align(8))]
pub struct PyDict {
    pub(crate) entries: DictContentType,
}
pub type PyDictRef = PyRef<PyDict>;

/// A borrowed view of a dict or frozendict, including subclasses.
///
/// This is an erased header view, not a shared payload layout. Every payload
/// access downcasts to the actual alternative, preserving its mutability.
#[repr(transparent)]
pub struct PyAnyDict {
    object: PyObject,
}

// SAFETY: this owner contributes exactly one strong reference to the object.
unsafe impl Traverse for PyRef<PyAnyDict> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        tracer(self.as_object());
    }
}

impl crate::convert::ToPyObject for PyRef<PyAnyDict> {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into_object()
    }
}

impl Borrow<PyObject> for PyAnyDict {
    fn borrow(&self) -> &PyObject {
        &self.object
    }
}

impl Borrow<PyAnyDict> for PyRef<PyAnyDict> {
    fn borrow(&self) -> &PyAnyDict {
        self
    }
}

impl ToOwned for PyAnyDict {
    type Owned = PyRef<Self>;

    fn to_owned(&self) -> PyRef<Self> {
        // SAFETY: this borrowed view already guarantees a dictionary payload.
        unsafe { PyRef::<Self>::from_object_unchecked(self.object.to_owned()) }
    }
}

impl From<PyDictRef> for PyRef<PyAnyDict> {
    fn from(dict: PyDictRef) -> Self {
        // SAFETY: the typed owner guarantees the dict alternative.
        unsafe { Self::from_object_unchecked(dict.into()) }
    }
}

impl From<PyRef<PyFrozenDict>> for PyRef<PyAnyDict> {
    fn from(dict: PyRef<PyFrozenDict>) -> Self {
        // SAFETY: the typed owner guarantees the frozendict alternative.
        unsafe { Self::from_object_unchecked(dict.into()) }
    }
}

impl From<PyRef<PyAnyDict>> for PyObjectRef {
    fn from(dict: PyRef<PyAnyDict>) -> Self {
        dict.into_object()
    }
}

// SAFETY: the transparent PyRef owner has the same layout as PyObjectRef.
// A successful check proves the actual dict or frozendict payload invariant.
unsafe impl crate::convert::TransmuteFromObject for PyRef<PyAnyDict> {
    fn check(vm: &VirtualMachine, obj: &PyObject) -> PyResult<()> {
        if PyAnyDict::from_object(obj).is_some() {
            Ok(())
        } else {
            Err(vm.new_type_error(format!(
                "expected dict or frozendict, got {}",
                obj.class().name()
            )))
        }
    }
}

impl TryFromObject for PyRef<PyAnyDict> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        <Self as crate::convert::TransmuteFromObject>::check(vm, &obj)?;
        // SAFETY: the shared checker proved the union view's payload invariant.
        Ok(unsafe { Self::from_object_unchecked(obj) })
    }
}

impl PyAnyDict {
    pub fn from_object(obj: &PyObject) -> Option<&Self> {
        if obj.downcast_ref::<PyDict>().is_some() || obj.downcast_ref::<PyFrozenDict>().is_some() {
            // SAFETY: PyAnyDict is transparent over PyObject and the payload was checked.
            Some(unsafe { &*(obj as *const PyObject).cast::<Self>() })
        } else {
            None
        }
    }

    pub(crate) fn as_mutable(&self) -> Option<&Py<PyDict>> {
        self.object.downcast_ref()
    }

    pub(crate) fn is_frozen(&self) -> bool {
        self.object.downcastable::<PyFrozenDict>()
    }

    // The raw payload is private: callers outside the VM cannot use it to mutate a frozendict.
    pub(crate) fn as_dict(&self) -> &PyDict {
        if let Some(dict) = self.as_mutable() {
            dict
        } else {
            &self.object.downcast_ref::<PyFrozenDict>().unwrap().dict
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
    pub fn copy_as_dict(&self, vm: &VirtualMachine) -> PyResult<PyDict> {
        let size = self.size();
        // Sparse copies go through merge, where equality between colliding
        // keys remains observable. Dense native tables can be cloned directly.
        if size.used == 0 || (self.uses_builtin_iter(vm) && size.used >= size.entries_size * 2 / 3)
        {
            return Ok(self.as_dict().copy());
        }
        let copied = PyDict::default();
        copied.merge_mapping(self.as_object().to_owned(), vm)?;
        Ok(copied)
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
            self.object
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
        if self.object.class().is(vm.ctx.types.frozendict_type) {
            return self.inner_getitem_opt(key, vm);
        }
        match self.object.get_item(key, vm) {
            Ok(value) => Ok(Some(value)),
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
            Err(exc) => Err(exc),
        }
    }
    pub fn get_item<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.as_mutable() {
            dict.get_item(key, vm)
        } else {
            self.object.get_item(key, vm)
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

impl<'a> IntoIterator for &'a PyAnyDict {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self.as_dict())
    }
}

// SAFETY: Traverse properly visits all owned PyObjectRefs
unsafe impl Traverse for PyDict {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        self.entries.traverse(traverse_fn);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        // Pop all entries and collect both keys and values
        for (key, value) in self.entries.drain_entries() {
            out.push(key);
            out.push(value);
        }
    }
}

impl fmt::Debug for PyDict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("dict")
    }
}

thread_local! {
    static DICT_FREELIST: Cell<crate::object::FreeList<PyDict>> = const { Cell::new(crate::object::FreeList::new()) };
}

impl PyPayload for PyDict {
    const MAX_FREELIST: usize = 80;
    const HAS_FREELIST: bool = true;

    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.dict_type
    }

    #[inline]
    unsafe fn freelist_push(obj: *mut PyObject) -> bool {
        DICT_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let stored = if list.len() < Self::MAX_FREELIST {
                    list.push(obj);
                    true
                } else {
                    false
                };
                fl.set(list);
                stored
            })
            .unwrap_or(false)
    }

    #[inline]
    unsafe fn freelist_pop(_payload: &Self) -> Option<NonNull<PyObject>> {
        DICT_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let result = list.pop().map(|p| unsafe { NonNull::new_unchecked(p) });
                fl.set(list);
                result
            })
            .ok()
            .flatten()
    }
}

impl PyDict {
    #[deprecated(note = "use PyDict::default().into_ref() instead")]
    pub fn new_ref(ctx: &Context) -> PyRef<Self> {
        Self::default().into_ref(ctx)
    }

    /// escape hatch to access the underlying data structure directly. prefer adding a method on
    /// PyDict instead of using this
    pub(crate) const fn _as_dict_inner(&self) -> &DictContentType {
        &self.entries
    }

    /// Returns all keys as a Vec, atomically under a single read lock.
    /// Thread-safe: prevents "dictionary changed size during iteration" errors.
    pub fn keys_vec(&self) -> Vec<PyObjectRef> {
        self.entries.keys()
    }

    /// Returns all values as a Vec, atomically under a single read lock.
    /// Thread-safe: prevents "dictionary changed size during iteration" errors.
    pub fn values_vec(&self) -> Vec<PyObjectRef> {
        self.entries.values()
    }

    /// Returns all items as a Vec, atomically under a single read lock.
    /// Thread-safe: prevents "dictionary changed size during iteration" errors.
    pub fn items_vec(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        self.entries.items()
    }

    pub(crate) fn items_with_hashes_vec(&self) -> Vec<(PyObjectRef, PyObjectRef, PyHash)> {
        self.entries.items_with_hashes()
    }

    /// `dict_update_arg`. `mapping_name` names the target type in the
    /// unhashable key error, as `dict_unhashable_type` does.
    pub(crate) fn update_arg(
        &self,
        other: PyObjectRef,
        override_existing: bool,
        mapping_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.merge_arg(other, override_existing, mapping_name, true, vm)
    }

    // PyDict_Update accepts mappings, without dict.update's sequence fallback.
    pub(crate) fn merge_mapping(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.merge_arg(other, true, "dict", false, vm)
    }

    fn merge_arg(
        &self,
        other: PyObjectRef,
        override_existing: bool,
        mapping_name: &str,
        allow_sequence: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(other_dict) = PyAnyDict::from_object(&other)
            && other_dict.uses_builtin_iter(vm)
        {
            return self.merge_dict(other_dict.as_dict(), override_existing, vm);
        }
        let dict = &self.entries;
        // Use get_attr to properly invoke __getattribute__ for proxy objects
        let keys_method = match other.get_attr(vm.ctx.intern_str("keys"), vm) {
            Ok(keys_method) => keys_method,
            Err(e) if allow_sequence && e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                return self.merge_seq2(&other, override_existing, mapping_name, vm);
            }
            Err(e) => return Err(e),
        };
        let keys_result = keys_method.call((), vm);
        drop(keys_method);
        let keys = other.mapping_unchecked().collect_method_output(
            vm.ctx.intern_str("keys"),
            keys_result?,
            vm,
        )?;
        let keys = PyIter::try_from_object(vm, keys)?;
        while let PyIterReturn::Return(key) = keys.next(vm)? {
            if !override_existing {
                let hash = key_hash(&*key, mapping_name, vm)?;
                if dict.contains(vm, &*key, hash)? {
                    continue;
                }
            }
            let value = match other.get_item(&*key, vm) {
                Ok(value) => value,
                Err(exc) => {
                    drop(keys);
                    return Err(exc);
                }
            };
            let outcome = key_hash(&*key, mapping_name, vm)
                .and_then(|hash| dict.insert(vm, &*key, hash, value.clone()));
            // Keep both owners until insertion returns, including on failure.
            drop(key);
            outcome?;
        }
        Ok(())
    }

    // Used in update and ior.
    pub fn merge_object(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.update_arg(other, true, "dict", vm)
    }

    pub fn merge_object_if_missing(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.update_arg(other, false, "dict", vm)
    }

    fn add_update_sequence_note(
        exc: PyBaseExceptionRef,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        if !exc.fast_isinstance(vm.ctx.exceptions.type_error) {
            return exc;
        }

        let note =
            format!("Cannot convert dictionary update sequence element #{index} to a sequence");
        match vm.call_method(exc.as_object(), "add_note", (vm.ctx.new_str(note),)) {
            Ok(_) => exc,
            Err(note_err) => {
                note_err.set_context(Some(exc));
                note_err
            }
        }
    }

    fn update_sequence_pair_from_slice(
        elements: &[PyObjectRef],
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let [key, value] = elements else {
            return Err(vm.new_value_error(format!(
                "dictionary update sequence element #{index} has length {}; 2 is required",
                elements.len()
            )));
        };
        Ok((key.clone(), value.clone()))
    }

    pub(crate) fn update_sequence_pair(
        element: PyObjectRef,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef)> {
        let element = match element.downcast_exact::<PyList>(vm) {
            Ok(list) => {
                let elements = list.borrow_vec();
                return Self::update_sequence_pair_from_slice(&elements, index, vm);
            }
            Err(element) => element,
        };
        let element = match element.downcast_exact::<PyTuple>(vm) {
            Ok(tuple) => {
                return Self::update_sequence_pair_from_slice(tuple.as_slice(), index, vm);
            }
            Err(element) => element,
        };

        let elements = (|| {
            let elem_iter = PyIter::try_from_object(vm, element).map_err(|exc| {
                if exc.fast_isinstance(vm.ctx.exceptions.type_error) {
                    vm.new_type_error("object is not iterable")
                } else {
                    exc
                }
            })?;
            elem_iter
                .into_iter::<PyObjectRef>(vm)
                .collect::<PyResult<Vec<_>>>()
        })()
        .map_err(|exc| Self::add_update_sequence_note(exc, index, vm))?;

        Self::update_sequence_pair_from_slice(&elements, index, vm)
    }

    pub fn merge_from_seq2(
        &self,
        seq2: &PyObject,
        override_existing: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.merge_seq2(seq2, override_existing, "dict", vm)
    }

    /// `dict_merge_from_seq2`
    fn merge_seq2(
        &self,
        seq2: &PyObject,
        override_existing: bool,
        mapping_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let iter = seq2.get_iter(vm)?;
        let dict = &self.entries;

        for (index, element) in iter.iter::<PyObjectRef>(vm)?.enumerate() {
            let (key, value) = Self::update_sequence_pair(element?, index, vm)?;

            let hash = key_hash(&*key, mapping_name, vm)?;
            if !override_existing && dict.contains(vm, &*key, hash)? {
                continue;
            }
            dict.insert(vm, &*key, hash, value)?;
        }
        Ok(())
    }

    pub(crate) fn merge_dict(
        &self,
        dict_other: &Self,
        override_existing: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = &self.entries;
        let source_len = dict_other.__len__();
        let mut position = 0;
        while let Some((next, key, value, hash)) = dict_other.entries.next_entry_with_hash(position)
        {
            position = next;
            let outcome: PyResult<()> = (|| {
                if override_existing || !dict.contains(vm, &*key, hash)? {
                    dict.insert(vm, &*key, hash, value.clone())?;
                }
                Ok(())
            })();
            // Release current references before checking mutations from
            // comparisons or finalizers, including a skipped existing key.
            drop(value);
            drop(key);
            outcome?;
            if dict_other.__len__() != source_len {
                return Err(vm.new_runtime_error("dict mutated during update"));
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// Hash `key`, turning a hashing failure into the dict-specific
    /// "cannot use 'X' as a dict key (...)" wording.
    ///
    /// The returned hash is passed into the inner map, so the key is hashed
    /// exactly once. A `TypeError` from key comparison (e.g. a colliding key's
    /// `__eq__`) is raised by the operation itself and must propagate unchanged.
    /// Hashing up front also means a `__hash__` that fails only intermittently
    /// is still reported.
    pub(crate) fn hash_or_unhashable<K: DictKey + ?Sized>(
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<PyHash> {
        key_hash(key, "dict", vm)
    }

    /// Set item variant which can be called with multiple
    /// key types, such as str to name a notable one.
    pub fn inner_setitem<K: DictKey + ?Sized>(
        &self,
        key: &K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let hash = Self::hash_or_unhashable(key, vm)?;
        self.entries.insert(vm, key, hash, value)
    }

    pub fn inner_delitem<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> PyResult<()> {
        let hash = Self::hash_or_unhashable(key, vm)?;
        if self.entries.delete_if_exists(vm, key, hash)? {
            Ok(())
        } else {
            Err(vm.new_key_error(key.to_pyobject(vm)))
        }
    }

    pub(crate) fn inner_pop_item<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let hash = Self::hash_or_unhashable(key, vm)?;
        self.entries.remove_if_exists(vm, key, hash)
    }

    pub fn get_or_insert(
        &self,
        vm: &VirtualMachine,
        key: &PyObject,
        default: impl FnOnce() -> PyObjectRef,
    ) -> PyResult {
        let hash = Self::hash_or_unhashable(key, vm)?;
        self.entries.setdefault(vm, key, hash, default)
    }

    pub fn from_attributes(attrs: PyAttributes, vm: &VirtualMachine) -> PyResult<Self> {
        let entries = DictContentType::default();

        for (key, value) in attrs {
            let hash = key.key_hash(vm)?;
            entries.insert(vm, key, hash, value)?;
        }

        Ok(Self { entries })
    }

    pub fn contains_key<K: DictKey + ?Sized>(&self, key: &K, vm: &VirtualMachine) -> bool {
        let hash = key.key_hash(vm).unwrap();
        self.entries.contains(vm, key, hash).unwrap()
    }

    pub fn size(&self) -> dict_inner::DictSize {
        self.entries.size()
    }

    pub fn next_entry(&self, position: usize) -> Option<(usize, PyObjectRef, PyObjectRef)> {
        self.entries.next_entry(position)
    }

    pub fn inner_getitem_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let hash = Self::hash_or_unhashable(key, vm)?;
        self.entries.get(vm, key, hash)
    }

    /// Keys of `obj` with their stored hashes, or `None` if it must be iterated
    /// generically. Only exact dicts and sets qualify, as in CPython's
    /// `_PyDict_FromKeys`: a subclass may override `__iter__`.
    fn fromkeys_known_hashes(
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> Option<Vec<(PyObjectRef, PyHash)>> {
        if let Some(dict) = obj.downcast_ref_if_exact::<Self>(vm) {
            Some(dict.entries.keys_with_hashes())
        } else if let Some(frozen) = obj.downcast_ref_if_exact::<PyFrozenDict>(vm) {
            Some(frozen.dict.entries.keys_with_hashes())
        } else {
            set::exact_set_keys_with_hashes(obj, vm)
        }
    }
}

#[derive(FromArgs)]
struct DictGetArgs {
    #[pyarg(positional)]
    key: PyObjectRef,
    #[pyarg(positional, optional)]
    default: Option<PyObjectRef>,
}

#[derive(FromArgs)]
struct FromKeysArgs {
    #[pyarg(positional)]
    iterable: PyObjectRef,
    #[pyarg(positional, optional)]
    value: Option<PyObjectRef>,
}

// Python dict methods:
impl PyDict {
    pub(crate) fn compare_entries(
        &self,
        other: &Self,
        op: PyComparisonOp,
        item: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if op == PyComparisonOp::Ne {
            return Self::compare_entries(self, other, PyComparisonOp::Eq, item, vm)
                .map(|x| x.map(|eq| !eq));
        }
        if !op.eval_ord(self.__len__().cmp(&other.__len__())) {
            return Ok(PyArithmeticValue::Implemented(false));
        }
        let mut position = 0;
        while let Some((next, k, v1, hash)) = self.entries.next_entry_with_hash(position) {
            position = next;
            let found = other.entries.get(vm, &*k, hash)?;
            match found {
                Some(v2) => {
                    if v1.is(&v2) {
                        continue;
                    }
                    if item && !vm.bool_eq(&v1, &v2)? {
                        return Ok(PyArithmeticValue::Implemented(false));
                    }
                }
                None => {
                    return Ok(PyArithmeticValue::Implemented(false));
                }
            }
        }
        Ok(PyArithmeticValue::Implemented(true))
    }

    pub fn __len__(&self) -> usize {
        self.entries.len()
    }

    fn __contains__(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        let hash = Self::hash_or_unhashable(key, vm)?;
        self.entries.contains(vm, key, hash)
    }

    fn __delitem__(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        self.inner_delitem(key, vm)
    }

    fn __setitem__(&self, key: &PyObject, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner_setitem(key, value, vm)
    }

    pub(crate) fn setdefault(
        &self,
        key: PyObjectRef,
        default: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let hash = Self::hash_or_unhashable(&*key, vm)?;
        self.entries.setdefault(vm, &*key, hash, || default)
    }

    pub(crate) fn __sizeof__(&self) -> usize {
        core::mem::size_of::<Self>() + self.entries.sizeof()
    }
    pub fn clear(&self) {
        self.entries.clear()
    }
    #[must_use]
    pub fn copy(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
    pub(crate) fn update(
        &self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let OptionalArg::Present(dict_obj) = dict_obj {
            self.merge_object(dict_obj, vm)?;
        }
        for (key, value) in kwargs {
            let hash = Self::hash_or_unhashable(&key, vm)?;
            self.entries.insert(vm, &key, hash, value)?;
        }
        Ok(())
    }
}

#[pyclass(
    with(
        Py,
        PyRef,
        Constructor,
        Initializer,
        Comparable,
        Iterable,
        AsSequence,
        AsNumber,
        AsMapping,
        Representable
    ),
    flags(BASETYPE, MAPPING, _MATCH_SELF, DICT_SUBCLASS)
)]
impl PyDict {}

#[pyclass]
impl Py<PyDict> {
    #[cfg_attr(feature = "flame-it", flame("PyDictRef"))]
    #[pymethod(coexist)]
    fn __getitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner_getitem(&*key, vm)
    }

    #[pyclassmethod]
    fn fromkeys(class: PyTypeRef, args: FromKeysArgs, vm: &VirtualMachine) -> PyResult {
        dict_fromkeys(
            class,
            args.iterable,
            args.value.unwrap_or_else(|| vm.ctx.none()),
            vm,
        )
    }

    #[pymethod]
    pub(crate) fn __sizeof__(&self) -> usize {
        self.payload.__sizeof__()
    }

    #[pymethod]
    pub fn clear(&self) {
        self.payload.clear()
    }

    #[pymethod]
    fn get(&self, args: DictGetArgs, vm: &VirtualMachine) -> PyResult {
        let hash = PyDict::hash_or_unhashable(&*args.key, vm)?;
        let found = self.entries.get(vm, &*args.key, hash)?;
        Ok(found.unwrap_or_else(|| args.default.unwrap_or_else(|| vm.ctx.none())))
    }

    #[pymethod(name = "setdefault")]
    fn setdefault_py(&self, args: DictGetArgs, vm: &VirtualMachine) -> PyResult {
        let default = args.default.unwrap_or_else(|| vm.ctx.none());
        self.setdefault(args.key, default, vm)
    }

    #[pymethod]
    #[must_use]
    pub fn copy(&self) -> PyDict {
        self.payload.copy()
    }

    #[pymethod]
    pub(crate) fn update(
        &self,
        dict_obj: OptionalArg<PyObjectRef>,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.payload.update(dict_obj, kwargs, vm)
    }

    #[pymethod]
    fn pop(
        &self,
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let hash = PyDict::hash_or_unhashable(&*key, vm)?;
        match self.entries.pop(vm, &*key, hash)? {
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

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }
}

#[pyclass]
impl PyRef<PyDict> {
    #[pymethod]
    fn keys(self) -> PyDictKeys {
        PyDictKeys::new(self.into())
    }

    #[pymethod]
    fn values(self) -> PyDictValues {
        PyDictValues::new(self.into())
    }

    #[pymethod]
    fn items(self) -> PyDictItems {
        PyDictItems::new(self.into())
    }

    #[pymethod]
    fn __reversed__(self) -> PyDictReverseKeyIterator {
        PyDictReverseKeyIterator::new(self.into())
    }

    fn __ior__(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        self.merge_object(other, vm)?;
        Ok(self)
    }
}

impl DefaultConstructor for PyDict {}

impl Initializer for PyDict {
    type Args = (OptionalArg<PyObjectRef>, KwArgs);

    fn init(zelf: &Py<Self>, (dict_obj, kwargs): Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        zelf.update(dict_obj, kwargs, vm)
    }
}

impl AsMapping for PyDict {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyDict::mapping_downcast(mapping).__len__())),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyDict::mapping_downcast(mapping).inner_getitem(needle, vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyDict::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.inner_setitem(needle, value, vm)?;
                    if zelf.as_object().is(vm.builtins.dict().as_object()) {
                        crate::stdlib::_testinternalcapi::note_builtin_dict();
                    }
                    Ok(())
                } else {
                    zelf.inner_delitem(needle, vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyDict {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            contains: atomic_func!(|seq, target, vm| {
                let hash = PyDict::hash_or_unhashable(target, vm)?;
                PyDict::sequence_downcast(seq)
                    .entries
                    .contains(vm, target, hash)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyDict {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(dict_or),
            inplace_or: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyDict>() {
                    a.to_owned()
                        .__ior__(b.to_pyobject(vm), vm)
                        .map(|d| d.into())
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Comparable for PyDict {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| match PyAnyDict::from_object(other) {
            Some(other) => {
                zelf.payload
                    .compare_entries(other.as_dict(), PyComparisonOp::Eq, true, vm)
            }
            None => Ok(PyComparisonValue::NotImplemented),
        })
    }
}

impl Iterable for PyDict {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyDictKeyIterator::new(zelf.into()).into_pyobject(vm))
    }
}

impl Representable for PyDict {
    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut result = Wtf8Buf::from("{");
            let mut first = true;
            for (key, value) in zelf {
                if !first {
                    result.push_str(", ");
                }
                first = false;
                result.push_wtf8(key.repr(vm)?.as_wtf8());
                result.push_str(": ");
                result.push_wtf8(value.repr(vm)?.as_wtf8());
            }
            result.push_char('}');
            vm.ctx.new_str(result)
        } else {
            vm.ctx.intern_str("{...}").to_owned()
        };
        Ok(s)
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

impl Py<PyDict> {
    #[inline]
    fn exact_dict(&self, vm: &VirtualMachine) -> bool {
        self.class().is(vm.ctx.types.dict_type)
    }

    fn missing_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        vm.get_method(self.to_owned().into(), identifier!(vm, __missing__))
            .map(|methods| methods?.call((key.to_pyobject(vm),), vm))
            .transpose()
    }

    #[inline]
    fn inner_getitem<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let hash = PyDict::hash_or_unhashable(key, vm)?;
        let found = self.entries.get(vm, key, hash)?;
        if let Some(value) = found {
            Ok(value)
        } else if let Some(value) = self.missing_opt(key, vm)? {
            Ok(value)
        } else {
            Err(vm.new_key_error(key.to_pyobject(vm)))
        }
    }

    /// Take a python dictionary and convert it to attributes.
    ///
    /// `PyAttributes` is keyed by interned strings, so a key that is not a
    /// string has nowhere to go. Those keys are left out, and `on_non_string`
    /// is called once, the first time one turns up, so the caller can decide
    /// whether that deserves an error or a warning.
    pub fn to_attributes(
        &self,
        vm: &VirtualMachine,
        on_non_string: impl FnOnce(&VirtualMachine) -> PyResult<()>,
    ) -> PyResult<PyAttributes> {
        let mut attrs = PyAttributes::default();
        let mut on_non_string = Some(on_non_string);
        for (key, value) in self {
            let key = match key.downcast_exact::<PyStr>(vm) {
                Ok(key) => vm.ctx.intern_str(key),
                // `PyStr`, not the exact type: a `str` subclass names an
                // attribute just as well, the same way it does as a keyword
                // argument. Interning drops the subclass, which nothing but
                // the key object itself can observe.
                Err(key) => match key.downcast_ref::<PyStr>() {
                    Some(key) => vm.ctx.intern_str(key.as_wtf8()),
                    None => {
                        if let Some(on_non_string) = on_non_string.take() {
                            on_non_string(vm)?;
                        }
                        continue;
                    }
                },
            };
            attrs.insert(key, value);
        }
        Ok(attrs)
    }

    pub(crate) fn get_item_known_hash(
        &self,
        key: &PyObject,
        hash: crate::common::hash::PyHash,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        self.entries.get(vm, key, hash)
    }

    pub(crate) fn set_item_known_hash(
        &self,
        key: &PyObject,
        hash: crate::common::hash::PyHash,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.entries.insert(vm, key, hash, value)
    }

    pub(crate) fn del_item_known_hash(
        &self,
        key: &PyObject,
        hash: crate::common::hash::PyHash,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.entries.delete_if_exists(vm, key, hash)? {
            Ok(())
        } else {
            Err(vm.new_key_error(key.to_owned()))
        }
    }

    pub(crate) fn contains_known_hash(
        &self,
        key: &PyObject,
        hash: crate::common::hash::PyHash,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.entries.contains(vm, key, hash)
    }

    pub fn get_item_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if self.exact_dict(vm) {
            let hash = PyDict::hash_or_unhashable(key, vm)?;
            self.entries.get(vm, key, hash)
            // Match CPython's exact-dict fast path: __missing__ only participates
            // for dict subclasses through the generic mapping lookup path below.
        } else {
            match self.as_object().get_item(key, vm) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                    self.missing_opt(key, vm)
                }
                Err(e) => Err(e),
            }
        }
    }

    /// Return a cached-entry hint for exact dict fast paths.
    pub(crate) fn hint_for_key<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<u16>> {
        if self.exact_dict(vm) {
            let hash = PyDict::hash_or_unhashable(key, vm)?;
            self.entries.hint_for_key(vm, key, hash)
        } else {
            Ok(None)
        }
    }

    /// Read a cached exact-dict entry after validating its key-layout stamp.
    #[inline]
    pub(crate) fn get_item_by_index_and_keys_version(
        &self,
        version: u16,
        index: u16,
    ) -> Option<PyObjectRef> {
        self.entries
            .get_index_if_keys_version(u32::from(version), usize::from(index))
    }

    /// Lookup trying a cached entry index hint first.
    ///
    /// When the hint misses but the key is present, also returns a refreshed
    /// hint (`None` when the hint hit or no hint is representable).
    pub(crate) fn get_item_opt_refresh_hint<K: DictKey + ?Sized>(
        &self,
        key: &K,
        hint: u16,
        vm: &VirtualMachine,
    ) -> PyResult<Option<(PyObjectRef, Option<u16>)>> {
        if self.exact_dict(vm) {
            if let Some(value) = self.entries.get_hint(key, usize::from(hint)) {
                return Ok(Some((value, None)));
            }
            let hash = PyDict::hash_or_unhashable(key, vm)?;
            self.entries.get_with_hint(vm, key, hash)
        } else {
            Ok(self.get_item_opt(key, vm)?.map(|value| (value, None)))
        }
    }

    pub(crate) fn replace_if_identity(
        &self,
        name: &Py<PyStr>,
        expected: &PyObject,
        replacement: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let hash = PyDict::hash_or_unhashable(name, vm)?;
        self.entries
            .replace_if(vm, name, hash, replacement, |current| current.is(expected))
    }

    /// Store using a cached entry index hint for the value-replace fast path.
    ///
    /// On a hint miss, returns a refreshed hint for the key (`None` when the
    /// hint hit or no hint is representable).
    pub(crate) fn set_item_with_hint<K: DictKey + ?Sized>(
        &self,
        key: &K,
        hint: u16,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<u16>> {
        if self.exact_dict(vm) {
            let hash = PyDict::hash_or_unhashable(key, vm)?;
            self.entries
                .insert_with_hint(vm, key, hash, usize::from(hint), value)
        } else {
            self.as_object().set_item(key, value, vm)?;
            Ok(None)
        }
    }

    /// Current keys-version stamp of the underlying storage (0 if unset).
    pub(crate) fn keys_version(&self) -> u32 {
        self.entries.keys_version()
    }

    pub(crate) fn module_attr_cache(
        &self,
        name: &super::PyStrInterned,
        vm: &VirtualMachine,
    ) -> Option<(u32, u16)> {
        self.exact_dict(vm)
            .then(|| self.entries.module_attr_cache(name, vm))
            .flatten()
    }

    #[inline]
    pub(crate) fn get_cached_module_attr(
        &self,
        name: &super::PyStrInterned,
        version: usize,
        index: usize,
        vm: &VirtualMachine,
    ) -> Option<PyObjectRef> {
        self.exact_dict(vm)
            .then(|| {
                self.entries
                    .get_cached_module_attr(name, version, index, vm)
            })
            .flatten()
    }

    /// Current keys-version stamp, assigning one if none is set.
    ///
    /// Returns 0 for dict subclasses: their lookup can be overridden, so a
    /// key-set attestation on the raw storage must never be cached for them.
    pub(crate) fn assign_keys_version(&self, vm: &VirtualMachine) -> u32 {
        if self.exact_dict(vm) {
            self.entries.assign_keys_version()
        } else {
            0
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
            self.inner_setitem(key, value, vm)?;
            if self.as_object().is(vm.builtins.dict().as_object()) {
                crate::stdlib::_testinternalcapi::note_builtin_dict();
            }
            Ok(())
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

    pub fn pop_item<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if self.exact_dict(vm) {
            self.inner_pop_item(key, vm)
        } else {
            let value = self.as_object().get_item(key, vm)?;
            self.as_object().del_item(key, vm)?;
            Ok(Some(value))
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
            // SAFETY: exact_dict checks passed
            let self_exact = unsafe { PyExact::ref_unchecked(self) };
            let other_exact = unsafe { PyExact::ref_unchecked(other) };
            self_exact.get_chain_exact(other_exact, key, vm)
        } else if let Some(value) = self.get_item_opt(key, vm)? {
            Ok(Some(value))
        } else {
            other.get_item_opt(key, vm)
        }
    }
}

impl PyExact<PyDict> {
    pub(crate) fn move_to_end(
        &self,
        key: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let hash = PyDict::hash_or_unhashable(&*key, vm)?;
        self.entries
            .move_to_end(vm, &*key, hash)?
            .ok_or_else(|| vm.new_key_error(key))
    }

    /// Look up `key` in `self`, falling back to `other`.
    /// Both dicts must be exact `dict` types (enforced by `PyExact`).
    pub(crate) fn get_chain_exact<K: DictKey + ?Sized>(
        &self,
        other: &Self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        debug_assert!(self.class().is(vm.ctx.types.dict_type));
        debug_assert!(other.class().is(vm.ctx.types.dict_type));
        let hash = PyDict::hash_or_unhashable(key, vm)?;
        self.entries.get_chain(&other.entries, vm, key, hash)
    }
}

#[pyclass(module = false, name = "frozendict", traverse = "manual")]
#[derive(Debug)]
pub struct PyFrozenDict {
    // Own the table directly, never a mutable Python dict visible through gc or a view.
    pub(crate) dict: PyDict,
    hash: PyAtomic<PyHash>,
}

unsafe impl Traverse for PyFrozenDict {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.dict.traverse(tracer);
    }
    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        Traverse::clear(&mut self.dict, out);
    }
}

impl PyPayload for PyFrozenDict {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.frozendict_type
    }
}

impl PyFrozenDict {
    pub(crate) fn from_dict(dict: PyDict) -> Self {
        Self {
            dict,
            hash: Radium::new(hash::SENTINEL),
        }
    }

    /// Hash a dictionary key, adding the frozendict context to an exact TypeError.
    pub fn key_hash<K: DictKey + ?Sized>(key: &K, vm: &VirtualMachine) -> PyResult<PyHash> {
        key_hash(key, "frozendict", vm)
    }

    pub fn inner_getitem_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let hash = Self::key_hash(key, vm)?;
        self.dict.entries.get(vm, key, hash)
    }
}

impl Constructor for PyFrozenDict {
    type Args = (OptionalArg<PyObjectRef>, KwArgs);
    fn py_new(
        _cls: &Py<PyType>,
        (input, kwargs): Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // The table is built before the frozendict is published, as
        // frozendict_new_untracked keeps it away from the GC.
        let dict = PyDict::default();
        if let OptionalArg::Present(input) = input {
            dict.update_arg(input, true, "frozendict", vm)?;
        }
        for (key, value) in kwargs {
            dict.inner_setitem(&key, value, vm)?;
        }
        Ok(Self::from_dict(dict))
    }
}

#[pyclass(
    with(
        Py,
        PyRef,
        Constructor,
        Hashable,
        Comparable,
        Iterable,
        AsMapping,
        AsSequence,
        AsNumber,
        Representable
    ),
    flags(BASETYPE, MAPPING, _MATCH_SELF)
)]
impl PyFrozenDict {
    fn __len__(&self) -> usize {
        self.dict.__len__()
    }

    #[pymethod]
    fn get(
        zelf: &Py<Self>,
        DictGetArgs { key, default }: DictGetArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        Ok(zelf
            .inner_getitem_opt(&*key, vm)?
            .unwrap_or_else(|| default.unwrap_or_else(|| vm.ctx.none())))
    }

    #[pymethod]
    fn __sizeof__(zelf: &Py<Self>) -> usize {
        zelf.dict.__sizeof__() + core::mem::size_of::<PyAtomic<PyHash>>()
    }

    #[pyclassmethod]
    fn fromkeys(
        cls: PyTypeRef,
        FromKeysArgs { iterable, value }: FromKeysArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        dict_fromkeys(cls, iterable, value.unwrap_or_else(|| vm.ctx.none()), vm)
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }
}

#[pyclass]
impl Py<PyFrozenDict> {
    #[pymethod(coexist)]
    fn __getitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(value) = self.inner_getitem_opt(&*key, vm)? {
            return Ok(value);
        }
        if !self.class().is(vm.ctx.types.frozendict_type)
            && let Some(method) =
                vm.get_method(self.to_owned().into(), identifier!(vm, __missing__))
        {
            return method?.call((key,), vm);
        }
        Err(vm.new_key_error(key))
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyFrozenDict>> {
        if self.class().is(vm.ctx.types.frozendict_type) {
            Ok(self.to_owned())
        } else {
            let dict = PyDict::default();
            if !self.dict.is_empty() {
                dict.update_arg(self.to_owned().into(), true, "frozendict", vm)?;
            }
            Ok(PyFrozenDict::from_dict(dict).into_ref(&vm.ctx))
        }
    }

    #[pymethod]
    fn __getnewargs__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let dict = vm
            .ctx
            .types
            .dict_type
            .as_object()
            .call((self.to_owned(),), vm)?;
        Ok(vm.new_tuple((dict,)))
    }
}

#[pyclass]
impl PyRef<PyFrozenDict> {
    #[pymethod]
    fn keys(self) -> PyDictKeys {
        PyDictKeys::new(self.into())
    }
    #[pymethod]
    fn values(self) -> PyDictValues {
        PyDictValues::new(self.into())
    }
    #[pymethod]
    fn items(self) -> PyDictItems {
        PyDictItems::new(self.into())
    }
    #[pymethod]
    fn __reversed__(self) -> PyDictReverseKeyIterator {
        PyDictReverseKeyIterator::new(self.into())
    }
}

impl AsMapping for PyFrozenDict {
    fn as_mapping() -> &'static PyMappingMethods {
        static METHODS: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|obj, _vm| Ok(PyFrozenDict::mapping_downcast(obj).__len__())),
            subscript: atomic_func!(
                |obj, key, vm| PyFrozenDict::mapping_downcast(obj).__getitem__(key.to_owned(), vm)
            ),
            ass_subscript: None,
        };
        &METHODS
    }
}
impl AsSequence for PyFrozenDict {
    fn as_sequence() -> &'static PySequenceMethods {
        static METHODS: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            contains: atomic_func!(|obj, key, vm| Ok(PyFrozenDict::sequence_downcast(obj)
                .inner_getitem_opt(key, vm)?
                .is_some())),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &METHODS
    }
}
impl AsNumber for PyFrozenDict {
    fn as_number() -> &'static PyNumberMethods {
        static METHODS: PyNumberMethods = PyNumberMethods {
            or: Some(frozendict_or),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &METHODS
    }
}
impl Iterable for PyFrozenDict {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyDictKeyIterator::new(zelf.into()).into_pyobject(vm))
    }
}
impl Comparable for PyFrozenDict {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| match PyAnyDict::from_object(other) {
            Some(other) => zelf
                .dict
                .compare_entries(other.as_dict(), PyComparisonOp::Eq, true, vm),
            None => Ok(PyComparisonValue::NotImplemented),
        })
    }
}
impl Hashable for PyFrozenDict {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let cached = zelf.hash.load(Ordering::Relaxed);
        if cached != hash::SENTINEL {
            return Ok(cached);
        }
        let mut hasher = hash::FrozenSetHash::new(zelf.__len__());
        let mut position = 0;
        while let Some((next, _key, value, key_hash)) =
            zelf.dict.entries.next_entry_with_hash(position)
        {
            position = next;
            // next_entry releases the table lock before invoking arbitrary Python __hash__.
            let pair_hash = hash::hash_tuple([Ok(key_hash), value.hash(vm)])?;
            hasher.add(pair_hash);
        }
        let result = hasher.finish();
        zelf.hash.store(result, Ordering::Relaxed);
        Ok(result)
    }
}
impl Representable for PyFrozenDict {
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let name = zelf.class().name();
        if zelf.dict.is_empty() {
            return Ok(vm.ctx.new_str(format!("{name}()")));
        }
        let mut result = Wtf8Buf::from(format!("{name}("));
        if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            result.push_str("{");
            for (index, (key, value)) in DictIter::new(&zelf.dict).enumerate() {
                if index != 0 {
                    result.push_str(", ");
                }
                result.push_wtf8(key.repr(vm)?.as_wtf8());
                result.push_str(": ");
                result.push_wtf8(value.repr(vm)?.as_wtf8());
            }
            result.push_str("}");
        } else {
            result.push_str("{...}");
        }
        result.push_str(")");
        Ok(vm.ctx.new_str(result))
    }
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

// Implement IntoIterator so that we can easily iterate dictionaries from rust code.
impl IntoIterator for PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        DictIntoIter::new(self)
    }
}

impl<'a> IntoIterator for &'a PyDictRef {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self)
    }
}

impl<'a> IntoIterator for &'a Py<PyDict> {
    type Item = (PyObjectRef, PyObjectRef);
    type IntoIter = DictIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        DictIter::new(self)
    }
}

pub struct DictIntoIter {
    dict: PyDictRef,
    position: usize,
}

impl DictIntoIter {
    pub const fn new(dict: PyDictRef) -> Self {
        Self { dict, position: 0 }
    }
}

impl Iterator for DictIntoIter {
    type Item = (PyObjectRef, PyObjectRef);

    fn next(&mut self) -> Option<Self::Item> {
        let (position, key, value) = self.dict.entries.next_entry(self.position)?;
        self.position = position;
        Some((key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let l = self.len();
        (l, Some(l))
    }
}

impl ExactSizeIterator for DictIntoIter {
    fn len(&self) -> usize {
        self.dict.entries.len_from_entry_index(self.position)
    }
}

pub struct DictIter<'a> {
    dict: &'a PyDict,
    position: usize,
}

impl<'a> DictIter<'a> {
    pub const fn new(dict: &'a PyDict) -> Self {
        DictIter { dict, position: 0 }
    }
}

impl Iterator for DictIter<'_> {
    type Item = (PyObjectRef, PyObjectRef);

    fn next(&mut self) -> Option<Self::Item> {
        let (position, key, value) = self.dict.entries.next_entry(self.position)?;
        self.position = position;
        Some((key, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let l = self.len();
        (l, Some(l))
    }
}

impl ExactSizeIterator for DictIter<'_> {
    fn len(&self) -> usize {
        self.dict.entries.len_from_entry_index(self.position)
    }
}

#[pyclass]
trait DictView: PyPayload + PyClassDef + Iterable + Representable {
    type ReverseIter: PyPayload + core::fmt::Debug;

    fn dict(&self) -> &PyAnyDict;

    fn __len__(&self) -> usize {
        self.dict().__len__()
    }

    #[pymethod]
    fn __reversed__(zelf: &Py<Self>) -> Self::ReverseIter;
}

#[derive(Debug, Clone)]
struct DictIterState {
    dict: Option<PyRef<PyAnyDict>>,
    size: dict_inner::DictSize,
    position: usize,
    remaining: usize,
    changed: bool,
    reversed: bool,
}

// SAFETY: the optional dictionary is the sole owned Python reference and is
// visited once. The remaining fields contain only scalar iterator state.
unsafe impl Traverse for DictIterState {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.dict.traverse(tracer_fn)
    }
}

enum DictIterError {
    SizeChanged,
    KeysChanged,
}

type DictIterStep<T> = (Result<Option<T>, DictIterError>, Option<PyRef<PyAnyDict>>);

impl DictIterState {
    fn new(dict: PyRef<PyAnyDict>, reversed: bool) -> Self {
        let size = dict.size();
        // usize::MAX represents the position before the first reverse entry.
        let position = if reversed {
            size.entries_size.wrapping_sub(1)
        } else {
            0
        };
        Self {
            dict: Some(dict),
            position,
            remaining: size.used,
            size,
            changed: false,
            reversed,
        }
    }

    fn length_hint(&self) -> usize {
        match &self.dict {
            Some(dict)
                if !self.changed && dict.size().has_same_used_and_relocations(&self.size) =>
            {
                self.remaining
            }
            _ => 0,
        }
    }

    // Only clone the projected references while the table and iterator are
    // locked. Allocation and the final release of the dictionary happen later.
    fn step<T>(&mut self, project: impl FnOnce(&PyObject, &PyObjectRef) -> T) -> DictIterStep<T> {
        let Some(dict) = &self.dict else {
            return (Ok(None), None);
        };
        if self.changed {
            return (Err(DictIterError::SizeChanged), None);
        }
        let remaining = self.remaining;
        let project =
            |key: &PyObject, value: &PyObjectRef| (remaining != 0).then(|| project(key, value));
        let entries = &dict.as_dict().entries;
        let entry = if self.reversed {
            entries.prev_entry_checked(self.position, &self.size, project)
        } else {
            entries.next_dict_entry_checked(self.position, &self.size, project)
        };
        match entry {
            Err(dict_inner::DictChanged) => {
                // Size failure is sticky and retains the dictionary owner.
                self.changed = true;
                (Err(DictIterError::SizeChanged), None)
            }
            Ok(Some((_, None))) => {
                // Excess keys exhaust the iterator rather than latching a
                // size failure. Its detached dictionary must be dropped unlocked.
                (Err(DictIterError::KeysChanged), self.dict.take())
            }
            Ok(Some((position, Some(item)))) => {
                self.position = if self.reversed {
                    position.wrapping_sub(1)
                } else {
                    position
                };
                self.remaining -= 1;
                (Ok(Some(item)), None)
            }
            Ok(None) => (Ok(None), self.dict.take()),
        }
    }
}

fn finish_dict_iter_step<T>(step: DictIterStep<T>, vm: &VirtualMachine) -> PyResult<Option<T>> {
    let (result, _released) = step;
    result.map_err(|error| {
        let message = match error {
            DictIterError::SizeChanged => "dictionary changed size during iteration",
            DictIterError::KeysChanged => "dictionary keys changed during iteration",
        };
        vm.new_runtime_error(message)
    })
}

macro_rules! dict_iterator {
    ($name:ident, $class:ident, $class_name:literal, $reversed:literal, $project:expr, $result:expr) => {
        #[pyclass(module = false, name = $class_name, traverse)]
        #[derive(Debug)]
        pub(crate) struct $name {
            internal: PyMutex<DictIterState>,
        }

        impl PyPayload for $name {
            #[inline]
            fn class(ctx: &Context) -> &'static Py<PyType> {
                ctx.types.$class
            }
        }

        impl $name {
            pub(crate) fn new(dict: PyRef<PyAnyDict>) -> Self {
                Self {
                    internal: PyMutex::new(DictIterState::new(dict, $reversed)),
                }
            }
        }

        #[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
        impl Py<$name> {
            #[pymethod]
            fn __length_hint__(&self) -> usize {
                self.internal.lock().length_hint()
            }

            #[pymethod]
            fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
                // Walk a copy through the same checks as next(), without
                // advancing or invalidating the original iterator on failure.
                let mut state = self.internal.lock().clone();
                let mut entries = Vec::new();
                while let Some(item) = finish_dict_iter_step(state.step($project), vm)? {
                    entries.push(($result)(vm, item));
                }
                let list = vm.ctx.new_list(entries);
                let iter = builtins_iter(vm)?;
                Ok(vm.new_tuple((iter, (list,))))
            }
        }

        impl SelfIter for $name {}

        impl IterNext for $name {
            fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
                let step = zelf.internal.lock().step($project);
                match finish_dict_iter_step(step, vm)? {
                    Some(item) => Ok(PyIterReturn::Return(($result)(vm, item))),
                    None => Ok(PyIterReturn::StopIteration(None)),
                }
            }
        }
    };
}

macro_rules! dict_view {
    (
        $name: ident,
        $iter_name: ident,
        $reverse_iter_name: ident,
        $class: ident,
        $iter_class: ident,
        $reverse_iter_class: ident,
        $class_name: literal,
        $iter_class_name: literal,
        $reverse_iter_class_name: literal,
        $unhashable: literal,
        $project_fn: expr,
        $result_fn: expr
    ) => {
        #[pyclass(module = false, name = $class_name, unhashable = $unhashable, traverse)]
        #[derive(Debug)]
        pub(crate) struct $name {
            pub(crate) dict: PyRef<PyAnyDict>,
        }

        impl $name {
            pub(crate) fn new(dict: PyRef<PyAnyDict>) -> Self {
                $name { dict }
            }
        }

        impl DictView for $name {
            type ReverseIter = $reverse_iter_name;

            fn dict(&self) -> &PyAnyDict {
                &self.dict
            }

            fn __reversed__(zelf: &Py<Self>) -> Self::ReverseIter {
                $reverse_iter_name::new(zelf.dict.clone())
            }
        }

        impl Iterable for $name {
            fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
                Ok($iter_name::new(zelf.dict.clone()).into_pyobject(vm))
            }
        }

        impl PyPayload for $name {
            fn class(ctx: &Context) -> &'static Py<PyType> {
                ctx.types.$class
            }
        }

        impl Representable for $name {
            #[inline]
            fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
                let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                    // CPython snapshots through the view's actual iterator before
                    // calling element representations, including for OrderedDict subclasses.
                    let items = ArgIterable::<PyObjectRef>::try_from_object(
                        vm,
                        zelf.as_object().to_owned(),
                    )?
                    .iter(vm)?
                    .collect::<PyResult<Vec<_>>>()?;
                    let list_repr = vm.ctx.new_list(items).as_object().repr(vm)?;
                    let mut result = Wtf8Buf::from(format!("{}(", zelf.class().slot_name()));
                    result.push_wtf8(list_repr.as_wtf8());
                    result.push_str(")");
                    vm.ctx.new_str(result)
                } else {
                    vm.ctx.intern_str("...").to_owned()
                };
                Ok(s)
            }

            #[cold]
            fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
                unreachable!("use repr instead")
            }
        }

        dict_iterator! {
            $iter_name, $iter_class, $iter_class_name, false, $project_fn, $result_fn
        }

        dict_iterator! {
            $reverse_iter_name, $reverse_iter_class, $reverse_iter_class_name,
            true, $project_fn, $result_fn
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
    true,
    |key: &PyObject, _value| key.to_owned(),
    |_vm: &VirtualMachine, key: PyObjectRef| key
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
    false,
    |_key: &PyObject, value: &PyObjectRef| value.clone(),
    |_vm: &VirtualMachine, value: PyObjectRef| value
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
    true,
    |key: &PyObject, value: &PyObjectRef| (key.to_owned(), value.clone()),
    // Builds a tuple, so it runs after the dict's read guard is released.
    |vm: &VirtualMachine, (key, value): (PyObjectRef, PyObjectRef)|
        vm.new_tuple((key, value)).into()
}

fn set_view_dict(obj: &PyObject) -> Option<&PyAnyDict> {
    obj.downcast_ref::<PyDictKeys>()
        .map(|view| view.dict())
        .or_else(|| obj.downcast_ref::<PyDictItems>().map(|view| view.dict()))
}

fn is_set_or_dict_view(obj: &PyObject) -> bool {
    obj.downcast_ref::<PySet>().is_some()
        || obj.downcast_ref::<PyFrozenSet>().is_some()
        || set_view_dict(obj).is_some()
}

// Set operations defined on set-like views of the dictionary.
#[pyclass]
trait ViewSetOps: DictView {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if !is_set_or_dict_view(other) {
            return Ok(PyComparisonValue::NotImplemented);
        }
        if op == PyComparisonOp::Ne {
            return ViewSetOps::cmp(zelf, other, PyComparisonOp::Eq, vm)
                .map(|result| result.map(|equal| !equal));
        }
        if !op.eval_ord(zelf.__len__().cmp(&other.length(vm)?)) {
            return Ok(PyComparisonValue::Implemented(false));
        }
        let (subset, superset) = if matches!(op, PyComparisonOp::Gt | PyComparisonOp::Ge) {
            (other, zelf.as_object())
        } else {
            (zelf.as_object(), other)
        };
        let subset = ArgIterable::<PyObjectRef>::try_from_object(vm, subset.to_owned())?;
        for item in subset.iter(vm)? {
            let item = item?;
            if !superset.sequence_unchecked().contains(&item, vm)? {
                return Ok(PyComparisonValue::Implemented(false));
            }
        }
        Ok(PyComparisonValue::Implemented(true))
    }

    #[pymethod]
    fn isdisjoint(zelf: PyRef<Self>, object: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf.is(object.as_object()) {
            return Ok(zelf.__len__() == 0);
        }
        let other_is_larger = if is_set_or_dict_view(object.as_object()) {
            let len = zelf.__len__();
            object.as_object().length(vm)? > len
        } else {
            false
        };
        let (container, iterable) = if other_is_larger {
            (object.as_object(), zelf.as_object())
        } else {
            (zelf.as_object(), object.as_object())
        };
        let iterable = ArgIterable::<PyObjectRef>::try_from_object(vm, iterable.to_owned())?;
        for item in iterable.iter(vm)? {
            let item = item?;
            if container.sequence_unchecked().contains(&item, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl ViewSetOps for PyDictKeys {}

#[pyclass(
    flags(DISALLOW_INSTANTIATION),
    with(
        DictView,
        Comparable,
        Iterable,
        ViewSetOps,
        AsSequence,
        AsNumber,
        Representable
    )
)]
impl PyDictKeys {
    fn __contains__(zelf: &PyObject, key: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        zelf.sequence_unchecked().contains(key, vm)
    }

    #[pygetset]
    fn mapping(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::from_any_dict(zelf.dict().to_owned())
    }
}

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
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyDictKeys::sequence_downcast(seq).__len__())),
            contains: atomic_func!(|seq, target, vm| {
                Ok(PyDictKeys::sequence_downcast(seq)
                    .dict
                    .inner_getitem_opt(target, vm)?
                    .is_some())
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyDictKeys {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            subtract: Some(set_inner_number_subtract),
            and: Some(set_inner_number_and),
            xor: Some(set_inner_number_xor),
            or: Some(set_inner_number_or),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl ViewSetOps for PyDictItems {}
#[pyclass(
    flags(DISALLOW_INSTANTIATION),
    with(
        DictView,
        Comparable,
        Iterable,
        ViewSetOps,
        AsSequence,
        AsNumber,
        Representable
    )
)]
impl PyDictItems {
    fn __contains__(zelf: &PyObject, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        zelf.sequence_unchecked().contains(needle, vm)
    }
    #[pygetset]
    fn mapping(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::from_any_dict(zelf.dict().to_owned())
    }
}

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
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyDictItems::sequence_downcast(seq).__len__())),
            contains: atomic_func!(|seq, target, vm| {
                let needle: &Py<PyTuple> = match target.downcast_ref() {
                    Some(needle) => needle,
                    None => return Ok(false),
                };
                if needle.as_slice().len() != 2 {
                    return Ok(false);
                }

                let zelf = PyDictItems::sequence_downcast(seq);
                let key = &needle.as_slice()[0];
                let Some(found) = zelf.dict().inner_getitem_opt(&**key, vm)? else {
                    return Ok(false);
                };
                let value = &needle.as_slice()[1];
                vm.identical_or_equal(&found, value)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyDictItems {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            subtract: Some(set_inner_number_subtract),
            and: Some(set_inner_number_and),
            xor: Some(dict_item_view_number_xor),
            or: Some(set_inner_number_or),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION),
    with(DictView, Iterable, AsSequence, Representable)
)]
impl PyDictValues {
    #[pygetset]
    fn mapping(zelf: PyRef<Self>) -> PyMappingProxy {
        PyMappingProxy::from_any_dict(zelf.dict().to_owned())
    }
}

impl AsSequence for PyDictValues {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyDictValues::sequence_downcast(seq).__len__())),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

fn dict_view_to_set(obj: &PyObject, vm: &VirtualMachine) -> PyResult<PySetInner> {
    let mut source = obj;
    if let Some(keys) = obj.downcast_ref::<PyDictKeys>() {
        let dict = keys.dict();
        if dict.as_object().class().is(vm.ctx.types.dict_type)
            || dict.as_object().class().is(vm.ctx.types.frozendict_type)
        {
            // Exact dictionary keys are already hashed. Subclasses instead
            // go through the view iterator, including OrderedDict views.
            source = dict.as_object();
        }
    }
    // Reflected operations can put a set on the left. Reuse the set
    // constructor's native paths rather than calling an overridden iterator.
    PySetInner::from_object(source.to_owned(), vm)
}

fn set_inner_number_op<F>(a: &PyObject, b: &PyObject, f: F, vm: &VirtualMachine) -> PyResult
where
    F: FnOnce(PySetInner, PyObjectRef) -> PyResult<PySetInner>,
{
    let a = dict_view_to_set(a, vm)?;
    Ok(PySet {
        inner: f(a, b.to_owned())?,
    }
    .into_pyobject(vm))
}

pub(crate) fn set_inner_number_subtract(
    a: &PyObject,
    b: &PyObject,
    vm: &VirtualMachine,
) -> PyResult {
    set_inner_number_op(a, b, |a, b| a.difference(b, vm), vm)
}

pub(crate) fn set_inner_number_and(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    let (view, other) = if set_view_dict(a).is_some() {
        (a, b)
    } else {
        (b, a)
    };
    set_view_number_and(view, other, vm)
}

pub(crate) fn set_view_number_and(
    view: &PyObject,
    other: &PyObject,
    vm: &VirtualMachine,
) -> PyResult {
    let view_len = set_view_dict(view).unwrap().__len__();
    if other.class().is(vm.ctx.types.set_type) && view_len <= other.length(vm)? {
        // The set path hashes each candidate once and can stop when the
        // result has reached the set's size. It also preserves the key from
        // the view, rather than the equal key resident in the set.
        return vm.call_method(other, "intersection", (view.to_owned(),));
    }
    let (view, other) = if set_view_dict(other).is_some_and(|dict| dict.__len__() > view_len) {
        (other, view)
    } else {
        (view, other)
    };
    let other = ArgIterable::<PyObjectRef>::try_from_object(vm, other.to_owned())?;
    let result = PySet::default();
    let mut iterator = other.iter(vm)?;
    let mut current = None;
    let outcome = (|| -> PyResult<()> {
        for item in iterator.by_ref() {
            current = Some(item?);
            let item = current.as_ref().unwrap();
            if view.sequence_unchecked().contains(item, vm)? {
                result.add(item.clone(), vm)?;
            }
            drop(current.take());
        }
        Ok(())
    })();
    // On failure, release the iterator and partial result before the current
    // item. Retain its owner through both membership and result insertion.
    drop(iterator);
    if let Err(exc) = outcome {
        drop(result);
        return Err(exc);
    }
    Ok(result.into_pyobject(vm))
}

pub(crate) fn set_inner_number_xor(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    set_inner_number_op(
        a,
        b,
        |a, b| {
            a.symmetric_difference_update(core::iter::once(b), vm)?;
            Ok(a)
        },
        vm,
    )
}

fn set_item_view_number_xor(a: &PyDictItems, b: &PyDictItems, vm: &VirtualMachine) -> PyResult {
    // Membership comparisons may mutate either original dictionary. Keep
    // unmatched left entries in a private copy, as in dictitems_xor.
    let remaining = a.dict().copy_as_dict(vm)?;
    let result = PySet::default();
    let mut current_key = None;
    let mut current_value = None;
    let outcome = (|| -> PyResult<()> {
        // Read the right dictionary live, without a public iterator's size
        // check. Release each table guard before calling value comparisons.
        let mut position = 0;
        while let Some((next_position, key, value, hash)) =
            b.dict().as_dict().entries.next_entry_with_hash(position)
        {
            position = next_position;
            current_key = Some(key);
            current_value = Some(value);
            let key = current_key.as_ref().unwrap().as_object();
            // Only unmatched pairs need to be hashable, and stored key hashes
            // must not invoke __hash__ again during lookup or deletion.
            let remove = match remaining.entries.get(vm, key, hash)? {
                Some(left_value) => {
                    vm.identical_or_equal(&left_value, current_value.as_ref().unwrap())?
                }
                None => false,
            };
            if remove {
                drop(current_value.take());
                remaining.entries.delete(vm, key, hash)?;
                drop(current_key.take());
            } else {
                let pair =
                    vm.new_tuple((current_key.take().unwrap(), current_value.take().unwrap()));
                result.add(pair.into(), vm)?;
            }
        }
        // Append the entries still present in the private copy.
        let mut position = 0;
        while let Some((next_position, key, value)) = remaining.entries.next_entry(position) {
            position = next_position;
            result.add(vm.new_tuple((key, value)).into(), vm)?;
        }
        Ok(())
    })();
    // On failure, CPython releases the private dictionary and result before
    // the current right item. Their finalizers may observe the distinction.
    drop(remaining);
    if let Err(exc) = outcome {
        drop(result);
        drop(current_key);
        return Err(exc);
    }
    Ok(result.into_pyobject(vm))
}

fn dict_item_view_number_xor(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    match (
        a.downcast_ref::<PyDictItems>(),
        b.downcast_ref::<PyDictItems>(),
    ) {
        (Some(a), Some(b)) => set_item_view_number_xor(a, b, vm),
        _ => set_inner_number_xor(a, b, vm),
    }
}

pub(crate) fn set_inner_number_or(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    set_inner_number_op(a, b, |a, b| a.union(b, vm), vm)
}

/// `_PyDict_Or`
fn dict_or(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    let (Some(left), Some(_)) = (PyAnyDict::from_object(a), PyAnyDict::from_object(b)) else {
        return Ok(vm.ctx.not_implemented());
    };
    // anydict_copy_untracked converts any frozendict to an exact frozendict.
    let frozen = left.is_frozen();
    let mapping_name = if frozen { "frozendict" } else { "dict" };
    let dict = PyDict::default();
    if !left.is_empty() {
        dict.update_arg(a.to_owned(), true, mapping_name, vm)?;
    }
    dict.update_arg(b.to_owned(), true, mapping_name, vm)?;
    Ok(if frozen {
        PyFrozenDict::from_dict(dict).into_pyobject(vm)
    } else {
        dict.into_pyobject(vm)
    })
}

fn frozendict_or(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
    if a.class().is(vm.ctx.types.frozendict_type)
        && let Some(left) = PyAnyDict::from_object(a)
        && let Some(right) = PyAnyDict::from_object(b)
    {
        // frozendict() | frozendict(...) => frozendict(...)
        if left.is_empty() && b.class().is(vm.ctx.types.frozendict_type) {
            return Ok(b.to_owned());
        }
        // frozendict(...) | frozendict() => frozendict(...)
        if right.is_empty()
            && (b.class().is(vm.ctx.types.dict_type) || b.class().is(vm.ctx.types.frozendict_type))
        {
            return Ok(a.to_owned());
        }
    }
    dict_or(a, b, vm)
}

pub(crate) fn dict_fromkeys(
    class: PyTypeRef,
    iterable: PyObjectRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    let exact_frozen = class.is(vm.ctx.types.frozendict_type);
    let created = if exact_frozen {
        None
    } else {
        Some(PyType::call(&class, ().into(), vm)?)
    };
    let frozen_source = created
        .as_ref()
        .and_then(|obj| obj.downcast_ref::<PyFrozenDict>());
    if exact_frozen || frozen_source.is_some() {
        let dict = PyDict::default();
        if let Some(source) = frozen_source {
            dict.update_arg(source.to_owned().into(), true, "frozendict", vm)?;
        }
        let iterable = ArgIterable::<PyObjectRef>::try_from_object(vm, iterable)?;
        if let Some(keys) = PyDict::fromkeys_known_hashes(iterable.as_object(), vm) {
            for (key, hash) in keys {
                dict.entries.insert(vm, &*key, hash, value.clone())?;
            }
        } else {
            for key in iterable.iter(vm)? {
                let key = key?;
                let hash = PyFrozenDict::key_hash(&*key, vm)?;
                dict.entries.insert(vm, &*key, hash, value.clone())?;
            }
        }
        let frozen = PyFrozenDict::from_dict(dict).into_ref(&vm.ctx);
        return if exact_frozen {
            Ok(frozen.into())
        } else {
            class.as_object().call((frozen,), vm)
        };
    }
    let iterable = ArgIterable::<PyObjectRef>::try_from_object(vm, iterable)?;
    let created = created.unwrap();
    if let Some(dict) = created.downcast_ref_if_exact::<PyDict>(vm) {
        if let Some(keys) = PyDict::fromkeys_known_hashes(iterable.as_object(), vm) {
            if class.is(vm.ctx.types.dict_type) {
                dict.entries.reserve_for_empty(keys.len());
            }
            for (key, hash) in keys {
                dict.entries.insert(vm, &*key, hash, value.clone())?;
            }
        } else {
            for key in iterable.iter(vm)? {
                dict.inner_setitem(&*key?, value.clone(), vm)?;
            }
        }
    } else {
        for key in iterable.iter(vm)? {
            created.set_item(&*key?, value.clone(), vm)?;
        }
    }
    Ok(created)
}

fn vectorcall_dict(
    zelf_obj: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyType> = zelf_obj.downcast_ref().unwrap();
    let obj = PyDict::default().into_ref_with_type(vm, zelf.to_owned())?;
    let func_args = FuncArgs::from_vectorcall_owned(args, nargs, kwnames);
    PyDict::slot_init(obj.as_object(), func_args, vm)?;
    Ok(obj.into())
}

fn vectorcall_frozendict(
    zelf: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    // frozendict(frozendict) returns the same object unmodified
    if nargs == 1
        && kwnames.is_none_or(<[_]>::is_empty)
        && args[0].class().is(vm.ctx.types.frozendict_type)
        && zelf.is(vm.ctx.types.frozendict_type)
    {
        return Ok(args[0].clone());
    }
    PyFrozenDict::slot_new(
        zelf.downcast_ref::<PyType>().unwrap().to_owned(),
        FuncArgs::from_vectorcall_owned(args, nargs, kwnames),
        vm,
    )
}

pub(crate) fn init(context: &'static Context) {
    PyDict::extend_class(context, context.types.dict_type);
    context
        .types
        .dict_type
        .slots
        .constructor_vectorcall
        .store(Some(vectorcall_dict));
    PyDictKeys::extend_class(context, context.types.dict_keys_type);
    PyDictKeyIterator::extend_class(context, context.types.dict_keyiterator_type);
    PyDictReverseKeyIterator::extend_class(context, context.types.dict_reversekeyiterator_type);
    PyDictValues::extend_class(context, context.types.dict_values_type);
    PyDictValueIterator::extend_class(context, context.types.dict_valueiterator_type);
    PyDictReverseValueIterator::extend_class(context, context.types.dict_reversevalueiterator_type);
    PyDictItems::extend_class(context, context.types.dict_items_type);
    PyDictItemIterator::extend_class(context, context.types.dict_itemiterator_type);
    PyDictReverseItemIterator::extend_class(context, context.types.dict_reverseitemiterator_type);
    PyFrozenDict::extend_class(context, context.types.frozendict_type);
    context
        .types
        .frozendict_type
        .slots
        .constructor_vectorcall
        .store(Some(vectorcall_frozendict));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anydict_reference_layout_and_ownership() {
        static_assertions::assert_not_impl_any!(PyAnyDict: PyPayload);
        static_assertions::assert_type_eq_all!(
            <PyAnyDict as crate::object::PyRefTarget>::Target,
            PyAnyDict
        );
        static_assertions::assert_eq_size!(PyRef<PyAnyDict>, PyObjectRef, Option<PyRef<PyAnyDict>>);
        static_assertions::assert_eq_align!(PyRef<PyAnyDict>, PyObjectRef);
        static_assertions::assert_eq_size!(PyAnyDict, PyObject);
        static_assertions::assert_eq_align!(PyAnyDict, PyObject);
        #[cfg(feature = "threading")]
        static_assertions::assert_impl_all!(PyRef<PyAnyDict>: Send, Sync);
        #[cfg(not(feature = "threading"))]
        static_assertions::assert_not_impl_any!(PyRef<PyAnyDict>: Send, Sync);

        let ctx = Context::genesis();
        let dict: PyRef<PyAnyDict> = PyDict::default().into_ref(ctx).into();
        let frozen: PyRef<PyAnyDict> = PyFrozenDict::from_dict(PyDict::default())
            .into_ref(ctx)
            .into();
        for owner in [dict, frozen] {
            let object = owner.as_object() as *const PyObject;
            assert_eq!(owner.as_object().strong_count(), 1);
            let borrowed = PyAnyDict::from_object(owner.as_object()).unwrap();
            assert!(core::ptr::eq(borrowed.as_object(), object));
            assert_eq!(owner.as_object().strong_count(), 1);
            assert_eq!(borrowed.as_mutable().is_none(), borrowed.is_frozen());
            let copy = borrowed.to_owned();
            assert_eq!(owner.as_object().strong_count(), 2);
            let clone = owner.clone();
            assert_eq!(owner.as_object().strong_count(), 3);
            let erased = clone.into_object();
            assert_eq!(erased.strong_count(), 3);
            assert!(core::ptr::eq(&*erased, object));
            let mut visited = 0;
            owner.traverse(&mut |edge| {
                assert!(core::ptr::eq(edge, object));
                visited += 1;
            });
            assert_eq!(visited, 1);
            drop(copy);
            drop(owner);
            assert_eq!(erased.strong_count(), 1);
        }
    }

    #[test]
    fn anydict_checked_reference_conversion() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let dict = PyDict::default().into_pyobject(vm);
            let frozen = PyFrozenDict::from_dict(PyDict::default()).into_pyobject(vm);
            for object in [dict, frozen] {
                let owner = PyRef::<PyAnyDict>::try_from_object(vm, object.clone()).unwrap();
                assert_eq!(object.strong_count(), 2);
                assert!(core::ptr::eq(owner.as_object(), &*object));
                let erased: PyObjectRef = owner.into();
                assert_eq!(object.strong_count(), 2);
                assert!(core::ptr::eq(&*erased, &*object));
                drop(erased);
                assert_eq!(object.strong_count(), 1);
            }
            let unrelated: PyObjectRef = vm.ctx.new_int(1000).into();
            assert!(PyAnyDict::from_object(&unrelated).is_none());
            let error = PyRef::<PyAnyDict>::try_from_object(vm, unrelated.clone()).unwrap_err();
            assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
            assert_eq!(unrelated.strong_count(), 1);
        });
    }
}

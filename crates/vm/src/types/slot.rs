use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLockReadGuard, PyRwLockWriteGuard,
};
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyInt, PyStr, PyStrInterned, PyStrRef, PyType, PyTypeRef},
    bytecode::ComparisonOperator,
    common::hash::PyHash,
    convert::ToPyObject,
    function::{
        Either, FromArgs, FuncArgs, OptionalArg, PyComparisonValue, PyMethodDef, PySetterValue,
    },
    protocol::{
        PyBuffer, PyIterReturn, PyMapping, PyMappingMethods, PyMappingSlots, PyNumber,
        PyNumberMethods, PyNumberSlots, PySequence, PySequenceMethods, PySequenceSlots,
    },
    vm::Context,
};
use crossbeam_utils::atomic::AtomicCell;
use malachite_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};
use std::{any::Any, any::TypeId, borrow::Borrow, cmp::Ordering, ops::Deref};

/// Type-erased storage for extension module data attached to heap types.
pub struct TypeDataSlot {
    // PyObject_GetTypeData
    type_id: TypeId,
    data: Box<dyn Any + Send + Sync>,
}

impl TypeDataSlot {
    /// Create a new type data slot with the given data.
    pub fn new<T: Any + Send + Sync + 'static>(data: T) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            data: Box::new(data),
        }
    }

    /// Get a reference to the data if the type matches.
    pub fn get<T: Any + 'static>(&self) -> Option<&T> {
        if self.type_id == TypeId::of::<T>() {
            self.data.downcast_ref()
        } else {
            None
        }
    }

    /// Get a mutable reference to the data if the type matches.
    pub fn get_mut<T: Any + 'static>(&mut self) -> Option<&mut T> {
        if self.type_id == TypeId::of::<T>() {
            self.data.downcast_mut()
        } else {
            None
        }
    }
}

/// Read guard for type data access, using mapped guard for zero-cost deref.
pub struct TypeDataRef<'a, T: 'static> {
    guard: PyMappedRwLockReadGuard<'a, T>,
}

impl<'a, T: Any + 'static> TypeDataRef<'a, T> {
    /// Try to create a TypeDataRef from a read guard.
    /// Returns None if the slot is empty or contains a different type.
    pub fn try_new(guard: PyRwLockReadGuard<'a, Option<TypeDataSlot>>) -> Option<Self> {
        PyRwLockReadGuard::try_map(guard, |opt| opt.as_ref().and_then(|slot| slot.get::<T>()))
            .ok()
            .map(|guard| Self { guard })
    }
}

impl<T: Any + 'static> std::ops::Deref for TypeDataRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

/// Write guard for type data access, using mapped guard for zero-cost deref.
pub struct TypeDataRefMut<'a, T: 'static> {
    guard: PyMappedRwLockWriteGuard<'a, T>,
}

impl<'a, T: Any + 'static> TypeDataRefMut<'a, T> {
    /// Try to create a TypeDataRefMut from a write guard.
    /// Returns None if the slot is empty or contains a different type.
    pub fn try_new(guard: PyRwLockWriteGuard<'a, Option<TypeDataSlot>>) -> Option<Self> {
        PyRwLockWriteGuard::try_map(guard, |opt| {
            opt.as_mut().and_then(|slot| slot.get_mut::<T>())
        })
        .ok()
        .map(|guard| Self { guard })
    }
}

impl<T: Any + 'static> std::ops::Deref for TypeDataRefMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<T: Any + 'static> std::ops::DerefMut for TypeDataRefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[macro_export]
macro_rules! atomic_func {
    ($x:expr) => {
        crossbeam_utils::atomic::AtomicCell::new(Some($x))
    };
}

// The corresponding field in CPython is `tp_` prefixed.
// e.g. name -> tp_name
#[derive(Default)]
#[non_exhaustive]
pub struct PyTypeSlots {
    /// # Safety
    /// For static types, always safe.
    /// For heap types, `__name__` must alive
    pub(crate) name: &'static str, // tp_name with <module>.<class> for print, not class name

    pub basicsize: usize,
    // tp_itemsize

    // Methods to implement standard operations

    // Method suites for standard classes
    pub as_number: PyNumberSlots,
    pub as_sequence: PySequenceSlots,
    pub as_mapping: PyMappingSlots,

    // More standard operations (here for binary compatibility)
    pub hash: AtomicCell<Option<HashFunc>>,
    pub call: AtomicCell<Option<GenericMethod>>,
    pub str: AtomicCell<Option<StringifyFunc>>,
    pub repr: AtomicCell<Option<StringifyFunc>>,
    pub getattro: AtomicCell<Option<GetattroFunc>>,
    pub setattro: AtomicCell<Option<SetattroFunc>>,

    // Functions to access object as input/output buffer
    pub as_buffer: Option<AsBufferFunc>,

    // Assigned meaning in release 2.1
    // rich comparisons
    pub richcompare: AtomicCell<Option<RichCompareFunc>>,

    // Iterators
    pub iter: AtomicCell<Option<IterFunc>>,
    pub iternext: AtomicCell<Option<IterNextFunc>>,

    pub methods: &'static [PyMethodDef],

    // Flags to define presence of optional/expanded features
    pub flags: PyTypeFlags,

    // tp_doc
    pub doc: Option<&'static str>,

    // Strong reference on a heap type, borrowed reference on a static type
    // tp_base
    // tp_dict
    pub descr_get: AtomicCell<Option<DescrGetFunc>>,
    pub descr_set: AtomicCell<Option<DescrSetFunc>>,
    // tp_dictoffset
    pub init: AtomicCell<Option<InitFunc>>,
    // tp_alloc
    pub new: AtomicCell<Option<NewFunc>>,
    // tp_free
    // tp_is_gc
    // tp_bases
    // tp_mro
    // tp_cache
    // tp_subclasses
    // tp_weaklist
    pub del: AtomicCell<Option<DelFunc>>,

    // The count of tp_members.
    pub member_count: usize,
}

impl PyTypeSlots {
    pub fn new(name: &'static str, flags: PyTypeFlags) -> Self {
        Self {
            name,
            flags,
            ..Default::default()
        }
    }

    pub fn heap_default() -> Self {
        Self {
            // init: AtomicCell::new(Some(init_wrapper)),
            ..Default::default()
        }
    }
}

impl std::fmt::Debug for PyTypeSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PyTypeSlots")
    }
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    #[non_exhaustive]
    pub struct PyTypeFlags: u64 {
        const MANAGED_DICT = 1 << 4;
        const SEQUENCE = 1 << 5;
        const MAPPING = 1 << 6;
        const DISALLOW_INSTANTIATION = 1 << 7;
        const IMMUTABLETYPE = 1 << 8;
        const HEAPTYPE = 1 << 9;
        const BASETYPE = 1 << 10;
        const METHOD_DESCRIPTOR = 1 << 17;
        // For built-in types that match the subject itself in pattern matching
        // (bool, int, float, str, bytes, bytearray, list, tuple, dict, set, frozenset)
        // This is not a stable API
        const _MATCH_SELF = 1 << 22;
        const HAS_DICT = 1 << 40;

        #[cfg(debug_assertions)]
        const _CREATED_WITH_FLAGS = 1 << 63;
    }
}

impl PyTypeFlags {
    // Default used for both built-in and normal classes: empty, for now.
    // CPython default: Py_TPFLAGS_HAVE_STACKLESS_EXTENSION | Py_TPFLAGS_HAVE_VERSION_TAG
    pub const DEFAULT: Self = Self::empty();

    // CPython: See initialization of flags in type_new.
    /// Used for types created in Python. Subclassable and are a
    /// heaptype.
    pub const fn heap_type_flags() -> Self {
        match Self::from_bits(Self::DEFAULT.bits() | Self::HEAPTYPE.bits() | Self::BASETYPE.bits())
        {
            Some(flags) => flags,
            None => unreachable!(),
        }
    }

    pub const fn has_feature(self, flag: Self) -> bool {
        self.contains(flag)
    }

    #[cfg(debug_assertions)]
    pub const fn is_created_with_flags(self) -> bool {
        self.contains(Self::_CREATED_WITH_FLAGS)
    }
}

impl Default for PyTypeFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub(crate) type GenericMethod = fn(&PyObject, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type HashFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyHash>;
// CallFunc = GenericMethod
pub(crate) type StringifyFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyRef<PyStr>>;
pub(crate) type GetattroFunc = fn(&PyObject, &Py<PyStr>, &VirtualMachine) -> PyResult;
pub(crate) type SetattroFunc =
    fn(&PyObject, &Py<PyStr>, PySetterValue, &VirtualMachine) -> PyResult<()>;
pub(crate) type AsBufferFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyBuffer>;
pub(crate) type RichCompareFunc = fn(
    &PyObject,
    &PyObject,
    PyComparisonOp,
    &VirtualMachine,
) -> PyResult<Either<PyObjectRef, PyComparisonValue>>;
pub(crate) type IterFunc = fn(PyObjectRef, &VirtualMachine) -> PyResult;
pub(crate) type IterNextFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyIterReturn>;
pub(crate) type DescrGetFunc =
    fn(PyObjectRef, Option<PyObjectRef>, Option<PyObjectRef>, &VirtualMachine) -> PyResult;
pub(crate) type DescrSetFunc =
    fn(&PyObject, PyObjectRef, PySetterValue, &VirtualMachine) -> PyResult<()>;
pub(crate) type NewFunc = fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type InitFunc = fn(PyObjectRef, FuncArgs, &VirtualMachine) -> PyResult<()>;
pub(crate) type DelFunc = fn(&PyObject, &VirtualMachine) -> PyResult<()>;

// slot_sq_length
pub(crate) fn len_wrapper(obj: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    let ret = vm.call_special_method(obj, identifier!(vm, __len__), ())?;
    let len = ret.downcast_ref::<PyInt>().ok_or_else(|| {
        vm.new_type_error(format!(
            "'{}' object cannot be interpreted as an integer",
            ret.class()
        ))
    })?;
    let len = len.as_bigint();
    if len.is_negative() {
        return Err(vm.new_value_error("__len__() should return >= 0"));
    }
    let len = len
        .to_isize()
        .ok_or_else(|| vm.new_overflow_error("cannot fit 'int' into an index-sized integer"))?;
    Ok(len as usize)
}

pub(crate) fn contains_wrapper(
    obj: &PyObject,
    needle: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let ret = vm.call_special_method(obj, identifier!(vm, __contains__), (needle,))?;
    ret.try_to_bool(vm)
}

macro_rules! number_unary_op_wrapper {
    ($name:ident) => {
        |a, vm| vm.call_special_method(a.deref(), identifier!(vm, $name), ())
    };
}
macro_rules! number_binary_op_wrapper {
    ($name:ident) => {
        |a, b, vm| vm.call_special_method(a, identifier!(vm, $name), (b.to_owned(),))
    };
}
macro_rules! number_binary_right_op_wrapper {
    ($name:ident) => {
        |a, b, vm| vm.call_special_method(b, identifier!(vm, $name), (a.to_owned(),))
    };
}
fn getitem_wrapper<K: ToPyObject>(obj: &PyObject, needle: K, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(obj, identifier!(vm, __getitem__), (needle,))
}

fn setitem_wrapper<K: ToPyObject>(
    obj: &PyObject,
    needle: K,
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match value {
        Some(value) => vm.call_special_method(obj, identifier!(vm, __setitem__), (needle, value)),
        None => vm.call_special_method(obj, identifier!(vm, __delitem__), (needle,)),
    }
    .map(drop)
}

fn repr_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
    let ret = vm.call_special_method(zelf, identifier!(vm, __repr__), ())?;
    ret.downcast::<PyStr>().map_err(|obj| {
        vm.new_type_error(format!(
            "__repr__ returned non-string (type {})",
            obj.class()
        ))
    })
}

fn str_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
    let ret = vm.call_special_method(zelf, identifier!(vm, __str__), ())?;
    ret.downcast::<PyStr>().map_err(|obj| {
        vm.new_type_error(format!(
            "__str__ returned non-string (type {})",
            obj.class()
        ))
    })
}

fn hash_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    let hash_obj = vm.call_special_method(zelf, identifier!(vm, __hash__), ())?;
    let py_int = hash_obj
        .downcast_ref::<PyInt>()
        .ok_or_else(|| vm.new_type_error("__hash__ method should return an integer"))?;
    let big_int = py_int.as_bigint();
    let hash: PyHash = big_int
        .to_i64()
        .unwrap_or_else(|| (big_int % BigInt::from(u64::MAX)).to_i64().unwrap());
    Ok(hash)
}

/// Marks a type as unhashable. Similar to PyObject_HashNotImplemented in CPython
pub fn hash_not_implemented(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    Err(vm.new_type_error(format!("unhashable type: {}", zelf.class().name())))
}

fn call_wrapper(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf, identifier!(vm, __call__), args)
}

fn getattro_wrapper(zelf: &PyObject, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
    let __getattribute__ = identifier!(vm, __getattribute__);
    let __getattr__ = identifier!(vm, __getattr__);
    match vm.call_special_method(zelf, __getattribute__, (name.to_owned(),)) {
        Ok(r) => Ok(r),
        Err(e)
            if e.fast_isinstance(vm.ctx.exceptions.attribute_error)
                && zelf.class().has_attr(__getattr__) =>
        {
            vm.call_special_method(zelf, __getattr__, (name.to_owned(),))
        }
        Err(e) => Err(e),
    }
}

fn setattro_wrapper(
    zelf: &PyObject,
    name: &Py<PyStr>,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let name = name.to_owned();
    match value {
        PySetterValue::Assign(value) => {
            vm.call_special_method(zelf, identifier!(vm, __setattr__), (name, value))?;
        }
        PySetterValue::Delete => {
            vm.call_special_method(zelf, identifier!(vm, __delattr__), (name,))?;
        }
    };
    Ok(())
}

pub(crate) fn richcompare_wrapper(
    zelf: &PyObject,
    other: &PyObject,
    op: PyComparisonOp,
    vm: &VirtualMachine,
) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
    vm.call_special_method(zelf, op.method_name(&vm.ctx), (other.to_owned(),))
        .map(Either::A)
}

fn iter_wrapper(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(&zelf, identifier!(vm, __iter__), ())
}

fn bool_wrapper(num: PyNumber<'_>, vm: &VirtualMachine) -> PyResult<bool> {
    let result = vm.call_special_method(num.obj, identifier!(vm, __bool__), ())?;
    // __bool__ must return exactly bool, not int subclass
    if !result.class().is(vm.ctx.types.bool_type) {
        return Err(vm.new_type_error(format!(
            "__bool__ should return bool, returned {}",
            result.class().name()
        )));
    }
    Ok(crate::builtins::bool_::get_value(&result))
}

// PyObject_SelfIter in CPython
const fn self_iter(zelf: PyObjectRef, _vm: &VirtualMachine) -> PyResult {
    Ok(zelf)
}

fn iternext_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
    PyIterReturn::from_pyresult(
        vm.call_special_method(zelf, identifier!(vm, __next__), ()),
        vm,
    )
}

fn descr_get_wrapper(
    zelf: PyObjectRef,
    obj: Option<PyObjectRef>,
    cls: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    vm.call_special_method(&zelf, identifier!(vm, __get__), (obj, cls))
}

fn descr_set_wrapper(
    zelf: &PyObject,
    obj: PyObjectRef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match value {
        PySetterValue::Assign(val) => {
            vm.call_special_method(zelf, identifier!(vm, __set__), (obj, val))
        }
        PySetterValue::Delete => vm.call_special_method(zelf, identifier!(vm, __delete__), (obj,)),
    }
    .map(drop)
}

fn init_wrapper(obj: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
    let res = vm.call_special_method(&obj, identifier!(vm, __init__), args)?;
    if !vm.is_none(&res) {
        return Err(vm.new_type_error(format!(
            "__init__ should return None, not '{:.200}'",
            res.class().name()
        )));
    }
    Ok(())
}

pub(crate) fn new_wrapper(cls: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    let new = cls.get_attr(identifier!(vm, __new__)).unwrap();
    args.prepend_arg(cls.into());
    new.call(args, vm)
}

fn del_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
    vm.call_special_method(zelf, identifier!(vm, __del__), ())?;
    Ok(())
}

impl PyType {
    pub(crate) fn update_slot<const ADD: bool>(&self, name: &'static PyStrInterned, ctx: &Context) {
        debug_assert!(name.as_str().starts_with("__"));
        debug_assert!(name.as_str().ends_with("__"));

        macro_rules! toggle_sub_slot {
            ($group:ident, $name:ident, $func:expr) => {{
                if ADD {
                    self.slots.$group.$name.store(Some($func));
                } else {
                    // When deleting, re-inherit from MRO (skip self)
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.$group.$name.load());
                    self.slots.$group.$name.store(inherited);
                }
            }};
        }

        // If the method is a slot wrapper, extract and use the original slot directly.
        // Otherwise use the generic wrapper.
        macro_rules! update_one_slot {
            ($slot:ident, $wrapper:expr, $variant:ident) => {{
                use crate::builtins::descriptor::SlotFunc;
                if ADD {
                    // Try to extract the original slot from a slot wrapper
                    if let Some(func) = self.lookup_slot_in_mro(name, ctx, |sf| {
                        if let SlotFunc::$variant(f) = sf {
                            Some(*f)
                        } else {
                            None
                        }
                    }) {
                        // Found slot wrapper - use the original slot directly
                        self.slots.$slot.store(Some(func));
                    } else {
                        // Real method found or no method - use generic wrapper
                        self.slots.$slot.store(Some($wrapper));
                    }
                } else {
                    // When deleting, re-inherit from MRO (skip self)
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.$slot.load());
                    self.slots.$slot.store(inherited);
                }
            }};
        }

        // For setattro slot: matches SetAttro or DelAttro
        macro_rules! update_setattro {
            ($wrapper:expr) => {{
                use crate::builtins::descriptor::SlotFunc;
                if ADD {
                    if let Some(func) = self.lookup_slot_in_mro(name, ctx, |sf| match sf {
                        SlotFunc::SetAttro(f) | SlotFunc::DelAttro(f) => Some(*f),
                        _ => None,
                    }) {
                        self.slots.setattro.store(Some(func));
                    } else {
                        self.slots.setattro.store(Some($wrapper));
                    }
                } else {
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.setattro.load());
                    self.slots.setattro.store(inherited);
                }
            }};
        }

        // For richcompare slot: matches RichCompare with any op
        macro_rules! update_richcompare {
            ($wrapper:expr) => {{
                use crate::builtins::descriptor::SlotFunc;
                if ADD {
                    if let Some(func) = self.lookup_slot_in_mro(name, ctx, |sf| {
                        if let SlotFunc::RichCompare(f, _) = sf {
                            Some(*f)
                        } else {
                            None
                        }
                    }) {
                        self.slots.richcompare.store(Some(func));
                    } else {
                        self.slots.richcompare.store(Some($wrapper));
                    }
                } else {
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.richcompare.load());
                    self.slots.richcompare.store(inherited);
                }
            }};
        }

        // For descr_set slot: matches DescrSet or DescrDel
        macro_rules! update_descr_set {
            ($wrapper:expr) => {{
                use crate::builtins::descriptor::SlotFunc;
                if ADD {
                    if let Some(func) = self.lookup_slot_in_mro(name, ctx, |sf| match sf {
                        SlotFunc::DescrSet(f) | SlotFunc::DescrDel(f) => Some(*f),
                        _ => None,
                    }) {
                        self.slots.descr_set.store(Some(func));
                    } else {
                        self.slots.descr_set.store(Some($wrapper));
                    }
                } else {
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.descr_set.load());
                    self.slots.descr_set.store(inherited);
                }
            }};
        }

        match name {
            _ if name == identifier!(ctx, __len__) => {
                toggle_sub_slot!(as_sequence, length, |seq, vm| len_wrapper(seq.obj, vm));
                toggle_sub_slot!(as_mapping, length, |mapping, vm| len_wrapper(
                    mapping.obj,
                    vm
                ));
            }
            _ if name == identifier!(ctx, __getitem__) => {
                toggle_sub_slot!(as_sequence, item, |seq, i, vm| getitem_wrapper(
                    seq.obj, i, vm
                ));
                toggle_sub_slot!(as_mapping, subscript, |mapping, key, vm| {
                    getitem_wrapper(mapping.obj, key, vm)
                });
            }
            _ if name == identifier!(ctx, __setitem__) || name == identifier!(ctx, __delitem__) => {
                toggle_sub_slot!(as_sequence, ass_item, |seq, i, value, vm| {
                    setitem_wrapper(seq.obj, i, value, vm)
                });
                toggle_sub_slot!(as_mapping, ass_subscript, |mapping, key, value, vm| {
                    setitem_wrapper(mapping.obj, key, value, vm)
                });
            }
            _ if name == identifier!(ctx, __contains__) => {
                toggle_sub_slot!(as_sequence, contains, |seq, needle, vm| {
                    contains_wrapper(seq.obj, needle, vm)
                });
            }
            _ if name == identifier!(ctx, __repr__) => {
                update_one_slot!(repr, repr_wrapper, Repr);
            }
            _ if name == identifier!(ctx, __str__) => {
                update_one_slot!(str, str_wrapper, Str);
            }
            _ if name == identifier!(ctx, __hash__) => {
                use crate::builtins::descriptor::SlotFunc;
                if ADD {
                    // Check for __hash__ = None first (descr == Py_None)
                    let method = self.attributes.read().get(name).cloned().or_else(|| {
                        self.mro
                            .read()
                            .iter()
                            .find_map(|cls| cls.attributes.read().get(name).cloned())
                    });

                    if method.as_ref().is_some_and(|m| m.is(&ctx.none)) {
                        self.slots.hash.store(Some(hash_not_implemented));
                    } else if let Some(func) = self.lookup_slot_in_mro(name, ctx, |sf| {
                        if let SlotFunc::Hash(f) = sf {
                            Some(*f)
                        } else {
                            None
                        }
                    }) {
                        self.slots.hash.store(Some(func));
                    } else {
                        self.slots.hash.store(Some(hash_wrapper));
                    }
                } else {
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.hash.load());
                    self.slots.hash.store(inherited);
                }
            }
            _ if name == identifier!(ctx, __call__) => {
                update_one_slot!(call, call_wrapper, Call);
            }
            _ if name == identifier!(ctx, __getattr__)
                || name == identifier!(ctx, __getattribute__) =>
            {
                update_one_slot!(getattro, getattro_wrapper, GetAttro);
            }
            _ if name == identifier!(ctx, __setattr__) || name == identifier!(ctx, __delattr__) => {
                update_setattro!(setattro_wrapper);
            }
            _ if name == identifier!(ctx, __eq__)
                || name == identifier!(ctx, __ne__)
                || name == identifier!(ctx, __le__)
                || name == identifier!(ctx, __lt__)
                || name == identifier!(ctx, __ge__)
                || name == identifier!(ctx, __gt__) =>
            {
                update_richcompare!(richcompare_wrapper);
            }
            _ if name == identifier!(ctx, __iter__) => {
                update_one_slot!(iter, iter_wrapper, Iter);
            }
            _ if name == identifier!(ctx, __next__) => {
                update_one_slot!(iternext, iternext_wrapper, IterNext);
            }
            _ if name == identifier!(ctx, __get__) => {
                update_one_slot!(descr_get, descr_get_wrapper, DescrGet);
            }
            _ if name == identifier!(ctx, __set__) || name == identifier!(ctx, __delete__) => {
                update_descr_set!(descr_set_wrapper);
            }
            _ if name == identifier!(ctx, __init__) => {
                update_one_slot!(init, init_wrapper, Init);
            }
            _ if name == identifier!(ctx, __new__) => {
                // __new__ is not wrapped via PyWrapper
                if ADD {
                    self.slots.new.store(Some(new_wrapper));
                } else {
                    let inherited = self
                        .mro
                        .read()
                        .iter()
                        .skip(1)
                        .find_map(|cls| cls.slots.new.load());
                    self.slots.new.store(inherited);
                }
            }
            _ if name == identifier!(ctx, __del__) => {
                update_one_slot!(del, del_wrapper, Del);
            }
            _ if name == identifier!(ctx, __bool__) => {
                toggle_sub_slot!(as_number, boolean, bool_wrapper);
            }
            _ if name == identifier!(ctx, __int__) => {
                toggle_sub_slot!(as_number, int, number_unary_op_wrapper!(__int__));
            }
            _ if name == identifier!(ctx, __index__) => {
                toggle_sub_slot!(as_number, index, number_unary_op_wrapper!(__index__));
            }
            _ if name == identifier!(ctx, __float__) => {
                toggle_sub_slot!(as_number, float, number_unary_op_wrapper!(__float__));
            }
            _ if name == identifier!(ctx, __add__) => {
                toggle_sub_slot!(as_number, add, number_binary_op_wrapper!(__add__));
            }
            _ if name == identifier!(ctx, __radd__) => {
                toggle_sub_slot!(
                    as_number,
                    right_add,
                    number_binary_right_op_wrapper!(__radd__)
                );
            }
            _ if name == identifier!(ctx, __iadd__) => {
                toggle_sub_slot!(as_number, inplace_add, number_binary_op_wrapper!(__iadd__));
            }
            _ if name == identifier!(ctx, __sub__) => {
                toggle_sub_slot!(as_number, subtract, number_binary_op_wrapper!(__sub__));
            }
            _ if name == identifier!(ctx, __rsub__) => {
                toggle_sub_slot!(
                    as_number,
                    right_subtract,
                    number_binary_right_op_wrapper!(__rsub__)
                );
            }
            _ if name == identifier!(ctx, __isub__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_subtract,
                    number_binary_op_wrapper!(__isub__)
                );
            }
            _ if name == identifier!(ctx, __mul__) => {
                toggle_sub_slot!(as_number, multiply, number_binary_op_wrapper!(__mul__));
            }
            _ if name == identifier!(ctx, __rmul__) => {
                toggle_sub_slot!(
                    as_number,
                    right_multiply,
                    number_binary_right_op_wrapper!(__rmul__)
                );
            }
            _ if name == identifier!(ctx, __imul__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_multiply,
                    number_binary_op_wrapper!(__imul__)
                );
            }
            _ if name == identifier!(ctx, __mod__) => {
                toggle_sub_slot!(as_number, remainder, number_binary_op_wrapper!(__mod__));
            }
            _ if name == identifier!(ctx, __rmod__) => {
                toggle_sub_slot!(
                    as_number,
                    right_remainder,
                    number_binary_right_op_wrapper!(__rmod__)
                );
            }
            _ if name == identifier!(ctx, __imod__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_remainder,
                    number_binary_op_wrapper!(__imod__)
                );
            }
            _ if name == identifier!(ctx, __divmod__) => {
                toggle_sub_slot!(as_number, divmod, number_binary_op_wrapper!(__divmod__));
            }
            _ if name == identifier!(ctx, __rdivmod__) => {
                toggle_sub_slot!(
                    as_number,
                    right_divmod,
                    number_binary_right_op_wrapper!(__rdivmod__)
                );
            }
            _ if name == identifier!(ctx, __pow__) => {
                toggle_sub_slot!(as_number, power, |a, b, c, vm| {
                    let args = if vm.is_none(c) {
                        vec![b.to_owned()]
                    } else {
                        vec![b.to_owned(), c.to_owned()]
                    };
                    vm.call_special_method(a, identifier!(vm, __pow__), args)
                });
            }
            _ if name == identifier!(ctx, __rpow__) => {
                toggle_sub_slot!(as_number, right_power, |a, b, c, vm| {
                    let args = if vm.is_none(c) {
                        vec![a.to_owned()]
                    } else {
                        vec![a.to_owned(), c.to_owned()]
                    };
                    vm.call_special_method(b, identifier!(vm, __rpow__), args)
                });
            }
            _ if name == identifier!(ctx, __ipow__) => {
                toggle_sub_slot!(as_number, inplace_power, |a, b, _, vm| {
                    vm.call_special_method(a, identifier!(vm, __ipow__), (b.to_owned(),))
                });
            }
            _ if name == identifier!(ctx, __lshift__) => {
                toggle_sub_slot!(as_number, lshift, number_binary_op_wrapper!(__lshift__));
            }
            _ if name == identifier!(ctx, __rlshift__) => {
                toggle_sub_slot!(
                    as_number,
                    right_lshift,
                    number_binary_right_op_wrapper!(__rlshift__)
                );
            }
            _ if name == identifier!(ctx, __ilshift__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_lshift,
                    number_binary_op_wrapper!(__ilshift__)
                );
            }
            _ if name == identifier!(ctx, __rshift__) => {
                toggle_sub_slot!(as_number, rshift, number_binary_op_wrapper!(__rshift__));
            }
            _ if name == identifier!(ctx, __rrshift__) => {
                toggle_sub_slot!(
                    as_number,
                    right_rshift,
                    number_binary_right_op_wrapper!(__rrshift__)
                );
            }
            _ if name == identifier!(ctx, __irshift__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_rshift,
                    number_binary_op_wrapper!(__irshift__)
                );
            }
            _ if name == identifier!(ctx, __and__) => {
                toggle_sub_slot!(as_number, and, number_binary_op_wrapper!(__and__));
            }
            _ if name == identifier!(ctx, __rand__) => {
                toggle_sub_slot!(
                    as_number,
                    right_and,
                    number_binary_right_op_wrapper!(__rand__)
                );
            }
            _ if name == identifier!(ctx, __iand__) => {
                toggle_sub_slot!(as_number, inplace_and, number_binary_op_wrapper!(__iand__));
            }
            _ if name == identifier!(ctx, __xor__) => {
                toggle_sub_slot!(as_number, xor, number_binary_op_wrapper!(__xor__));
            }
            _ if name == identifier!(ctx, __rxor__) => {
                toggle_sub_slot!(
                    as_number,
                    right_xor,
                    number_binary_right_op_wrapper!(__rxor__)
                );
            }
            _ if name == identifier!(ctx, __ixor__) => {
                toggle_sub_slot!(as_number, inplace_xor, number_binary_op_wrapper!(__ixor__));
            }
            _ if name == identifier!(ctx, __or__) => {
                toggle_sub_slot!(as_number, or, number_binary_op_wrapper!(__or__));
            }
            _ if name == identifier!(ctx, __ror__) => {
                toggle_sub_slot!(
                    as_number,
                    right_or,
                    number_binary_right_op_wrapper!(__ror__)
                );
            }
            _ if name == identifier!(ctx, __ior__) => {
                toggle_sub_slot!(as_number, inplace_or, number_binary_op_wrapper!(__ior__));
            }
            _ if name == identifier!(ctx, __floordiv__) => {
                toggle_sub_slot!(
                    as_number,
                    floor_divide,
                    number_binary_op_wrapper!(__floordiv__)
                );
            }
            _ if name == identifier!(ctx, __rfloordiv__) => {
                toggle_sub_slot!(
                    as_number,
                    right_floor_divide,
                    number_binary_right_op_wrapper!(__rfloordiv__)
                );
            }
            _ if name == identifier!(ctx, __ifloordiv__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_floor_divide,
                    number_binary_op_wrapper!(__ifloordiv__)
                );
            }
            _ if name == identifier!(ctx, __truediv__) => {
                toggle_sub_slot!(
                    as_number,
                    true_divide,
                    number_binary_op_wrapper!(__truediv__)
                );
            }
            _ if name == identifier!(ctx, __rtruediv__) => {
                toggle_sub_slot!(
                    as_number,
                    right_true_divide,
                    number_binary_right_op_wrapper!(__rtruediv__)
                );
            }
            _ if name == identifier!(ctx, __itruediv__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_true_divide,
                    number_binary_op_wrapper!(__itruediv__)
                );
            }
            _ if name == identifier!(ctx, __matmul__) => {
                toggle_sub_slot!(
                    as_number,
                    matrix_multiply,
                    number_binary_op_wrapper!(__matmul__)
                );
            }
            _ if name == identifier!(ctx, __rmatmul__) => {
                toggle_sub_slot!(
                    as_number,
                    right_matrix_multiply,
                    number_binary_right_op_wrapper!(__rmatmul__)
                );
            }
            _ if name == identifier!(ctx, __imatmul__) => {
                toggle_sub_slot!(
                    as_number,
                    inplace_matrix_multiply,
                    number_binary_op_wrapper!(__imatmul__)
                );
            }
            _ => {}
        }
    }

    /// Look up a method in MRO and extract the slot function if it's a slot wrapper.
    /// Returns Some(slot_func) if a matching slot wrapper is found, None if a real method
    /// is found or no method exists.
    fn lookup_slot_in_mro<T: Copy>(
        &self,
        name: &'static PyStrInterned,
        ctx: &Context,
        extract: impl Fn(&crate::builtins::descriptor::SlotFunc) -> Option<T>,
    ) -> Option<T> {
        use crate::builtins::descriptor::PyWrapper;

        // Helper to extract slot from an attribute if it's a wrapper descriptor
        let try_extract = |attr: &PyObjectRef| -> Option<T> {
            if attr.class().is(ctx.types.wrapper_descriptor_type) {
                attr.downcast_ref::<PyWrapper>()
                    .and_then(|wrapper| extract(&wrapper.wrapped))
            } else {
                None
            }
        };

        // Look up in self's dict first
        if let Some(attr) = self.attributes.read().get(name).cloned() {
            if let Some(func) = try_extract(&attr) {
                return Some(func);
            }
            return None;
        }

        // Look up in MRO
        for cls in self.mro.read().iter() {
            if let Some(attr) = cls.attributes.read().get(name).cloned() {
                if let Some(func) = try_extract(&attr) {
                    return Some(func);
                }
                return None;
            }
        }
        // No method found in MRO
        None
    }
}

/// Trait for types that can be constructed via Python's `__new__` method.
///
/// `slot_new` corresponds to the `__new__` type slot.
///
/// In most cases, `__new__` simply initializes the payload and assigns a type,
/// so you only need to override `py_new`. The default `slot_new` implementation
/// will call `py_new` and then wrap the result with `into_ref_with_type`.
///
/// However, if a subtype requires more than just payload initialization
/// (e.g., returning an existing object for optimization, setting attributes
/// after creation, or special handling of the class type), you should override
/// `slot_new` directly instead of `py_new`.
///
/// # When to use `py_new` only (most common case):
/// - Simple payload initialization that just creates `Self`
/// - The type doesn't need special handling for subtypes
///
/// # When to override `slot_new`:
/// - Returning existing objects (e.g., `PyInt`, `PyStr`, `PyBool` for optimization)
/// - Setting attributes or dict entries after object creation
/// - Special class type handling (e.g., `PyType` and its metaclasses)
/// - Post-creation mutations that require `PyRef`
#[pyclass]
pub trait Constructor: PyPayload + std::fmt::Debug {
    type Args: FromArgs;

    /// The type slot for `__new__`. Override this only when you need special
    /// behavior beyond simple payload creation.
    #[inline]
    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        let payload = Self::py_new(&cls, args, vm)?;
        payload.into_ref_with_type(vm, cls).map(Into::into)
    }

    /// Creates the payload for this type. In most cases, just implement this method
    /// and let the default `slot_new` handle wrapping with the correct type.
    fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self>;
}

pub trait DefaultConstructor: PyPayload + Default + std::fmt::Debug {
    fn construct_and_init(args: Self::Args, vm: &VirtualMachine) -> PyResult<PyRef<Self>>
    where
        Self: Initializer,
    {
        let this = Self::default().into_ref(&vm.ctx);
        Self::init(this.clone(), args, vm)?;
        Ok(this)
    }
}

impl<T> Constructor for T
where
    T: DefaultConstructor,
{
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::default().into_ref_with_type(vm, cls).map(Into::into)
    }

    fn py_new(cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        Err(vm.new_type_error(format!("cannot create {} instances", cls.slot_name())))
    }
}

#[pyclass]
pub trait Initializer: PyPayload {
    type Args: FromArgs;

    #[inline]
    #[pyslot]
    fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        #[cfg(debug_assertions)]
        let class_name_for_debug = zelf.class().name().to_string();

        let zelf = match zelf.try_into_value(vm) {
            Ok(zelf) => zelf,
            Err(err) => {
                #[cfg(debug_assertions)]
                {
                    if let Ok(msg) = err.as_object().repr(vm) {
                        let double_appearance =
                            msg.as_str().matches(&class_name_for_debug as &str).count() == 2;
                        if double_appearance {
                            panic!(
                                "This type `{}` doesn't seem to support `init`. Override `slot_init` instead: {}",
                                class_name_for_debug, msg
                            );
                        }
                    }
                }
                return Err(err);
            }
        };
        let args: Self::Args = args.bind(vm)?;
        Self::init(zelf, args, vm)
    }

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()>;
}

#[pyclass]
pub trait Destructor: PyPayload {
    #[inline] // for __del__
    #[pyslot]
    fn slot_del(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __del__"))?;
        Self::del(zelf, vm)
    }

    #[pymethod]
    fn __del__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::slot_del(&zelf, vm)
    }

    fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()>;
}

#[pyclass]
pub trait Callable: PyPayload {
    type Args: FromArgs;

    #[inline]
    #[pyslot]
    fn slot_call(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let zelf = zelf.downcast_ref().ok_or_else(|| {
            let repr = zelf.repr(vm);
            let help = if let Ok(repr) = repr.as_ref() {
                repr.as_str().to_owned()
            } else {
                zelf.class().name().to_owned()
            };
            vm.new_type_error(format!("unexpected payload for __call__ of {help}"))
        })?;
        let args = args.bind(vm)?;
        Self::call(zelf, args, vm)
    }

    #[inline]
    #[pymethod]
    fn __call__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::slot_call(&zelf, args.bind(vm)?, vm)
    }
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult;
}

#[pyclass]
pub trait GetDescriptor: PyPayload {
    #[pyslot]
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult;

    #[inline]
    #[pymethod]
    fn __get__(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Self::descr_get(zelf, Some(obj), cls.into_option(), vm)
    }

    #[inline]
    fn _as_pyref<'a>(zelf: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a Py<Self>> {
        zelf.try_to_value(vm)
    }

    #[inline]
    fn _unwrap<'a>(
        zelf: &'a PyObject,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(&'a Py<Self>, PyObjectRef)> {
        let zelf = Self::_as_pyref(zelf, vm)?;
        let obj = vm.unwrap_or_none(obj);
        Ok((zelf, obj))
    }

    #[inline]
    fn _check<'a>(
        zelf: &'a PyObject,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> Option<(&'a Py<Self>, PyObjectRef)> {
        // CPython descr_check
        let obj = obj?;
        // if (!PyObject_TypeCheck(obj, descr->d_type)) {
        //     PyErr_Format(PyExc_TypeError,
        //                  "descriptor '%V' for '%.100s' objects "
        //                  "doesn't apply to a '%.100s' object",
        //                  descr_name((PyDescrObject *)descr), "?",
        //                  descr->d_type->slot_name,
        //                  obj->ob_type->slot_name);
        //     *pres = NULL;
        //     return 1;
        // } else {
        Some((Self::_as_pyref(zelf, vm).unwrap(), obj))
    }

    #[inline]
    fn _cls_is(cls: &Option<PyObjectRef>, other: &impl Borrow<PyObject>) -> bool {
        cls.as_ref().is_some_and(|cls| other.borrow().is(cls))
    }
}

#[pyclass]
pub trait Hashable: PyPayload {
    #[inline]
    #[pyslot]
    fn slot_hash(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __hash__"))?;
        Self::hash(zelf, vm)
    }

    // __hash__ is now exposed via SlotFunc::Hash wrapper in extend_class()

    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
}

#[pyclass]
pub trait Representable: PyPayload {
    #[inline]
    #[pyslot]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __repr__"))?;
        Self::repr(zelf, vm)
    }

    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        let repr = Self::repr_str(zelf, vm)?;
        Ok(vm.ctx.new_str(repr))
    }

    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String>;
}

#[pyclass]
pub trait Comparable: PyPayload {
    #[inline]
    #[pyslot]
    fn slot_richcompare(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        let zelf = zelf.downcast_ref().ok_or_else(|| {
            vm.new_type_error(format!(
                "unexpected payload for {}",
                op.method_name(&vm.ctx).as_str()
            ))
        })?;
        Self::cmp(zelf, other, op, vm).map(Either::B)
    }

    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue>;

    #[inline]
    #[pymethod]
    fn __eq__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Eq, vm)
    }
    #[inline]
    #[pymethod]
    fn __ne__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Ne, vm)
    }
    #[inline]
    #[pymethod]
    fn __lt__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Lt, vm)
    }
    #[inline]
    #[pymethod]
    fn __le__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Le, vm)
    }
    #[inline]
    #[pymethod]
    fn __ge__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Ge, vm)
    }
    #[inline]
    #[pymethod]
    fn __gt__(
        zelf: &Py<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Gt, vm)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct PyComparisonOp(ComparisonOperator);

impl From<ComparisonOperator> for PyComparisonOp {
    fn from(op: ComparisonOperator) -> Self {
        Self(op)
    }
}

#[allow(non_upper_case_globals)]
impl PyComparisonOp {
    pub const Lt: Self = Self(ComparisonOperator::Less);
    pub const Gt: Self = Self(ComparisonOperator::Greater);
    pub const Ne: Self = Self(ComparisonOperator::NotEqual);
    pub const Eq: Self = Self(ComparisonOperator::Equal);
    pub const Le: Self = Self(ComparisonOperator::LessOrEqual);
    pub const Ge: Self = Self(ComparisonOperator::GreaterOrEqual);
}

impl PyComparisonOp {
    pub fn eq_only(
        self,
        f: impl FnOnce() -> PyResult<PyComparisonValue>,
    ) -> PyResult<PyComparisonValue> {
        match self {
            Self::Eq => f(),
            Self::Ne => f().map(|x| x.map(|eq| !eq)),
            _ => Ok(PyComparisonValue::NotImplemented),
        }
    }

    pub const fn eval_ord(self, ord: Ordering) -> bool {
        let bit = match ord {
            Ordering::Less => Self::Lt,
            Ordering::Equal => Self::Eq,
            Ordering::Greater => Self::Gt,
        };
        self.0 as u8 & bit.0 as u8 != 0
    }

    pub const fn swapped(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Eq => Self::Eq,
            Self::Ne => Self::Ne,
            Self::Ge => Self::Le,
            Self::Gt => Self::Lt,
        }
    }

    pub fn method_name(self, ctx: &Context) -> &'static PyStrInterned {
        match self {
            Self::Lt => identifier!(ctx, __lt__),
            Self::Le => identifier!(ctx, __le__),
            Self::Eq => identifier!(ctx, __eq__),
            Self::Ne => identifier!(ctx, __ne__),
            Self::Ge => identifier!(ctx, __ge__),
            Self::Gt => identifier!(ctx, __gt__),
        }
    }

    pub const fn operator_token(self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Ge => ">=",
            Self::Gt => ">",
        }
    }

    /// Returns an appropriate return value for the comparison when a and b are the same object, if an
    /// appropriate return value exists.
    #[inline]
    pub fn identical_optimization(
        self,
        a: &impl Borrow<PyObject>,
        b: &impl Borrow<PyObject>,
    ) -> Option<bool> {
        self.map_eq(|| a.borrow().is(b.borrow()))
    }

    /// Returns `Some(true)` when self is `Eq` and `f()` returns true. Returns `Some(false)` when self
    /// is `Ne` and `f()` returns true. Otherwise returns `None`.
    #[inline]
    pub fn map_eq(self, f: impl FnOnce() -> bool) -> Option<bool> {
        let eq = match self {
            Self::Eq => true,
            Self::Ne => false,
            _ => return None,
        };
        f().then_some(eq)
    }
}

#[pyclass]
pub trait GetAttr: PyPayload {
    #[pyslot]
    fn slot_getattro(obj: &PyObject, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let zelf = obj
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __getattribute__"))?;
        Self::getattro(zelf, name, vm)
    }

    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult;

    #[inline]
    #[pymethod]
    fn __getattribute__(zelf: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::slot_getattro(&zelf, &name, vm)
    }
}

#[pyclass]
pub trait SetAttr: PyPayload {
    #[pyslot]
    #[inline]
    fn slot_setattro(
        obj: &PyObject,
        name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = obj
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __setattr__"))?;
        Self::setattro(zelf, name, value, vm)
    }

    fn setattro(
        zelf: &Py<Self>,
        name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()>;

    #[inline]
    #[pymethod]
    fn __setattr__(
        zelf: PyObjectRef,
        name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::slot_setattro(&zelf, &name, PySetterValue::Assign(value), vm)
    }

    #[inline]
    #[pymethod]
    fn __delattr__(zelf: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::slot_setattro(&zelf, &name, PySetterValue::Delete, vm)
    }
}

#[pyclass]
pub trait AsBuffer: PyPayload {
    // TODO: `flags` parameter
    #[inline]
    #[pyslot]
    fn slot_as_buffer(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer"))?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer>;
}

#[pyclass]
pub trait AsMapping: PyPayload {
    fn as_mapping() -> &'static PyMappingMethods;

    #[inline]
    fn mapping_downcast(mapping: PyMapping<'_>) -> &Py<Self> {
        unsafe { mapping.obj.downcast_unchecked_ref() }
    }

    fn extend_slots(slots: &mut PyTypeSlots) {
        slots.as_mapping.copy_from(Self::as_mapping());
    }
}

#[pyclass]
pub trait AsSequence: PyPayload {
    fn as_sequence() -> &'static PySequenceMethods;

    #[inline]
    fn sequence_downcast(seq: PySequence<'_>) -> &Py<Self> {
        unsafe { seq.obj.downcast_unchecked_ref() }
    }

    fn extend_slots(slots: &mut PyTypeSlots) {
        slots.as_sequence.copy_from(Self::as_sequence());
    }
}

#[pyclass]
pub trait AsNumber: PyPayload {
    #[pyslot]
    fn as_number() -> &'static PyNumberMethods;

    fn clone_exact(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        // not all AsNumber requires this implementation.
        unimplemented!()
    }

    #[inline]
    fn number_downcast(num: PyNumber<'_>) -> &Py<Self> {
        unsafe { num.obj.downcast_unchecked_ref() }
    }

    #[inline]
    fn number_downcast_exact(num: PyNumber<'_>, vm: &VirtualMachine) -> PyRef<Self> {
        if let Some(zelf) = num.downcast_ref_if_exact::<Self>(vm) {
            zelf.to_owned()
        } else {
            Self::clone_exact(Self::number_downcast(num), vm)
        }
    }
}

#[pyclass]
pub trait Iterable: PyPayload {
    #[pyslot]
    fn slot_iter(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = zelf
            .downcast()
            .map_err(|_| vm.new_type_error("unexpected payload for __iter__"))?;
        Self::iter(zelf, vm)
    }

    // __iter__ is exposed via SlotFunc::Iter wrapper in extend_class()

    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult;

    fn extend_slots(_slots: &mut PyTypeSlots) {}
}

// `Iterator` fits better, but to avoid confusion with rust std::iter::Iterator
#[pyclass(with(Iterable))]
pub trait IterNext: PyPayload + Iterable {
    #[pyslot]
    fn slot_iternext(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __next__"))?;
        Self::next(zelf, vm)
    }

    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn>;

    // __next__ is exposed via SlotFunc::IterNext wrapper in extend_class()
}

pub trait SelfIter: PyPayload {}

impl<T> Iterable for T
where
    T: SelfIter,
{
    #[cold]
    fn slot_iter(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let repr = zelf.repr(vm)?;
        unreachable!("slot must be overridden for {}", repr.as_str());
    }

    // __iter__ is exposed via SlotFunc::Iter wrapper in extend_class()

    #[cold]
    fn iter(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
        unreachable!("slot_iter is implemented");
    }

    fn extend_slots(slots: &mut PyTypeSlots) {
        let prev = slots.iter.swap(Some(self_iter));
        debug_assert!(prev.is_some()); // slot_iter would be set
    }
}

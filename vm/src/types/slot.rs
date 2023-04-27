use crate::{
    builtins::{type_::PointerSlot, PyInt, PyStr, PyStrInterned, PyStrRef, PyType, PyTypeRef},
    bytecode::ComparisonOperator,
    common::hash::PyHash,
    convert::{ToPyObject, ToPyResult},
    function::{Either, FromArgs, FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
    identifier,
    protocol::{
        PyBuffer, PyIterReturn, PyMapping, PyMappingMethods, PyNumber, PyNumberMethods,
        PyNumberSlots, PySequence, PySequenceMethods,
    },
    vm::Context,
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use std::{borrow::Borrow, cmp::Ordering, ops::Deref};

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
    pub as_sequence: AtomicCell<Option<PointerSlot<PySequenceMethods>>>,
    pub as_mapping: AtomicCell<Option<PointerSlot<PyMappingMethods>>>,

    // More standard operations (here for binary compatibility)
    pub hash: AtomicCell<Option<HashFunc>>,
    pub call: AtomicCell<Option<GenericMethod>>,
    // tp_str
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
    #[non_exhaustive]
    pub struct PyTypeFlags: u64 {
        const IMMUTABLETYPE = 1 << 8;
        const HEAPTYPE = 1 << 9;
        const BASETYPE = 1 << 10;
        const METHOD_DESCR = 1 << 17;
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
        unsafe {
            Self::from_bits_unchecked(
                Self::DEFAULT.bits | Self::HEAPTYPE.bits | Self::BASETYPE.bits,
            )
        }
    }

    pub fn has_feature(self, flag: Self) -> bool {
        self.contains(flag)
    }

    #[cfg(debug_assertions)]
    pub fn is_created_with_flags(self) -> bool {
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
pub(crate) type StringifyFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyStrRef>;
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
    let len = ret.payload::<PyInt>().ok_or_else(|| {
        vm.new_type_error(format!(
            "'{}' object cannot be interpreted as an integer",
            ret.class()
        ))
    })?;
    let len = len.as_bigint();
    if len.is_negative() {
        return Err(vm.new_value_error("__len__() should return >= 0".to_owned()));
    }
    let len = len.to_isize().ok_or_else(|| {
        vm.new_overflow_error("cannot fit 'int' into an index-sized integer".to_owned())
    })?;
    Ok(len as usize)
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

fn repr_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
    let ret = vm.call_special_method(zelf, identifier!(vm, __repr__), ())?;
    ret.downcast::<PyStr>().map_err(|obj| {
        vm.new_type_error(format!(
            "__repr__ returned non-string (type {})",
            obj.class()
        ))
    })
}

fn hash_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    let hash_obj = vm.call_special_method(zelf, identifier!(vm, __hash__), ())?;
    let py_int = hash_obj
        .payload_if_subclass::<PyInt>(vm)
        .ok_or_else(|| vm.new_type_error("__hash__ method should return an integer".to_owned()))?;
    Ok(rustpython_common::hash::hash_bigint(py_int.as_bigint()))
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
        Err(_) if zelf.class().has_attr(__getattr__) => {
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
        return Err(vm.new_type_error("__init__ must return None".to_owned()));
    }
    Ok(())
}

fn new_wrapper(cls: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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

        macro_rules! toggle_slot {
            ($name:ident, $func:expr) => {{
                self.slots.$name.store(if ADD { Some($func) } else { None });
            }};
        }

        macro_rules! toggle_subslot {
            ($group:ident, $name:ident, $func:expr) => {
                self.slots
                    .$group
                    .$name
                    .store(if ADD { Some($func) } else { None });
            };
        }

        macro_rules! update_slot {
            ($name:ident, $func:expr) => {{
                self.slots.$name.store(Some($func));
            }};
        }

        macro_rules! update_pointer_slot {
            ($name:ident, $pointed:ident) => {{
                self.slots
                    .$name
                    .store(unsafe { PointerSlot::from_heaptype(self, |ext| &ext.$pointed) });
            }};
        }

        macro_rules! toggle_ext_func {
            ($n1:ident, $n2:ident, $func:expr) => {{
                self.heaptype_ext.as_ref().unwrap().$n1.$n2.store(if ADD {
                    Some($func)
                } else {
                    None
                });
            }};
        }

        match name {
            _ if name == identifier!(ctx, __len__) => {
                // update_slot!(as_mapping, slot_as_mapping);
                toggle_ext_func!(sequence_methods, length, |seq, vm| len_wrapper(seq.obj, vm));
                update_pointer_slot!(as_sequence, sequence_methods);
                toggle_ext_func!(mapping_methods, length, |mapping, vm| len_wrapper(
                    mapping.obj,
                    vm
                ));
                update_pointer_slot!(as_mapping, mapping_methods);
            }
            _ if name == identifier!(ctx, __getitem__) => {
                // update_slot!(as_mapping, slot_as_mapping);
                toggle_ext_func!(sequence_methods, item, |seq, i, vm| getitem_wrapper(
                    seq.obj, i, vm
                ));
                update_pointer_slot!(as_sequence, sequence_methods);
                toggle_ext_func!(mapping_methods, subscript, |mapping, key, vm| {
                    getitem_wrapper(mapping.obj, key, vm)
                });
                update_pointer_slot!(as_mapping, mapping_methods);
            }
            _ if name == identifier!(ctx, __setitem__) || name == identifier!(ctx, __delitem__) => {
                // update_slot!(as_mapping, slot_as_mapping);
                toggle_ext_func!(sequence_methods, ass_item, |seq, i, value, vm| {
                    setitem_wrapper(seq.obj, i, value, vm)
                });
                update_pointer_slot!(as_sequence, sequence_methods);
                toggle_ext_func!(mapping_methods, ass_subscript, |mapping, key, value, vm| {
                    setitem_wrapper(mapping.obj, key, value, vm)
                });
                update_pointer_slot!(as_mapping, mapping_methods);
            }
            _ if name == identifier!(ctx, __repr__) => {
                update_slot!(repr, repr_wrapper);
            }
            _ if name == identifier!(ctx, __hash__) => {
                let is_unhashable = self
                    .attributes
                    .read()
                    .get(identifier!(ctx, __hash__))
                    .map_or(false, |a| a.is(&ctx.none));
                let wrapper = if is_unhashable {
                    hash_not_implemented
                } else {
                    hash_wrapper
                };
                toggle_slot!(hash, wrapper);
            }
            _ if name == identifier!(ctx, __call__) => {
                toggle_slot!(call, call_wrapper);
            }
            _ if name == identifier!(ctx, __getattr__)
                || name == identifier!(ctx, __getattribute__) =>
            {
                update_slot!(getattro, getattro_wrapper);
            }
            _ if name == identifier!(ctx, __setattr__) || name == identifier!(ctx, __delattr__) => {
                update_slot!(setattro, setattro_wrapper);
            }
            _ if name == identifier!(ctx, __eq__)
                || name == identifier!(ctx, __ne__)
                || name == identifier!(ctx, __le__)
                || name == identifier!(ctx, __lt__)
                || name == identifier!(ctx, __ge__)
                || name == identifier!(ctx, __gt__) =>
            {
                update_slot!(richcompare, richcompare_wrapper);
            }
            _ if name == identifier!(ctx, __iter__) => {
                toggle_slot!(iter, iter_wrapper);
            }
            _ if name == identifier!(ctx, __next__) => {
                toggle_slot!(iternext, iternext_wrapper);
            }
            _ if name == identifier!(ctx, __get__) => {
                toggle_slot!(descr_get, descr_get_wrapper);
            }
            _ if name == identifier!(ctx, __set__) || name == identifier!(ctx, __delete__) => {
                update_slot!(descr_set, descr_set_wrapper);
            }
            _ if name == identifier!(ctx, __init__) => {
                toggle_slot!(init, init_wrapper);
            }
            _ if name == identifier!(ctx, __new__) => {
                toggle_slot!(new, new_wrapper);
            }
            _ if name == identifier!(ctx, __del__) => {
                toggle_slot!(del, del_wrapper);
            }
            _ if name == identifier!(ctx, __int__) => {
                toggle_subslot!(as_number, int, number_unary_op_wrapper!(__int__));
            }
            _ if name == identifier!(ctx, __index__) => {
                toggle_subslot!(as_number, index, number_unary_op_wrapper!(__index__));
            }
            _ if name == identifier!(ctx, __float__) => {
                toggle_subslot!(as_number, float, number_unary_op_wrapper!(__float__));
            }
            _ if name == identifier!(ctx, __add__) => {
                toggle_subslot!(as_number, add, number_binary_op_wrapper!(__add__));
            }
            _ if name == identifier!(ctx, __radd__) => {
                toggle_subslot!(
                    as_number,
                    right_add,
                    number_binary_right_op_wrapper!(__radd__)
                );
            }
            _ if name == identifier!(ctx, __iadd__) => {
                toggle_subslot!(as_number, inplace_add, number_binary_op_wrapper!(__iadd__));
            }
            _ if name == identifier!(ctx, __sub__) => {
                toggle_subslot!(as_number, subtract, number_binary_op_wrapper!(__sub__));
            }
            _ if name == identifier!(ctx, __rsub__) => {
                toggle_subslot!(
                    as_number,
                    right_subtract,
                    number_binary_right_op_wrapper!(__rsub__)
                );
            }
            _ if name == identifier!(ctx, __isub__) => {
                toggle_subslot!(
                    as_number,
                    inplace_subtract,
                    number_binary_op_wrapper!(__isub__)
                );
            }
            _ if name == identifier!(ctx, __mul__) => {
                toggle_subslot!(as_number, multiply, number_binary_op_wrapper!(__mul__));
            }
            _ if name == identifier!(ctx, __rmul__) => {
                toggle_subslot!(
                    as_number,
                    right_multiply,
                    number_binary_right_op_wrapper!(__rmul__)
                );
            }
            _ if name == identifier!(ctx, __imul__) => {
                toggle_subslot!(
                    as_number,
                    inplace_multiply,
                    number_binary_op_wrapper!(__imul__)
                );
            }
            _ if name == identifier!(ctx, __mod__) => {
                toggle_subslot!(as_number, remainder, number_binary_op_wrapper!(__mod__));
            }
            _ if name == identifier!(ctx, __rmod__) => {
                toggle_subslot!(
                    as_number,
                    right_remainder,
                    number_binary_right_op_wrapper!(__rmod__)
                );
            }
            _ if name == identifier!(ctx, __imod__) => {
                toggle_subslot!(
                    as_number,
                    inplace_remainder,
                    number_binary_op_wrapper!(__imod__)
                );
            }
            _ if name == identifier!(ctx, __divmod__) => {
                toggle_subslot!(as_number, divmod, number_binary_op_wrapper!(__divmod__));
            }
            _ if name == identifier!(ctx, __rdivmod__) => {
                toggle_subslot!(
                    as_number,
                    right_divmod,
                    number_binary_right_op_wrapper!(__rdivmod__)
                );
            }
            _ if name == identifier!(ctx, __pow__) => {
                toggle_subslot!(as_number, power, |a, b, c, vm| {
                    let args = if vm.is_none(c) {
                        vec![b.to_owned()]
                    } else {
                        vec![b.to_owned(), c.to_owned()]
                    };
                    vm.call_special_method(a, identifier!(vm, __pow__), args)
                });
            }
            _ if name == identifier!(ctx, __rpow__) => {
                toggle_subslot!(as_number, right_power, |a, b, c, vm| {
                    let args = if vm.is_none(c) {
                        vec![a.to_owned()]
                    } else {
                        vec![a.to_owned(), c.to_owned()]
                    };
                    vm.call_special_method(b, identifier!(vm, __rpow__), args)
                });
            }
            _ if name == identifier!(ctx, __ipow__) => {
                toggle_subslot!(as_number, inplace_power, |a, b, _, vm| {
                    vm.call_special_method(a, identifier!(vm, __ipow__), (b.to_owned(),))
                });
            }
            _ if name == identifier!(ctx, __lshift__) => {
                toggle_subslot!(as_number, lshift, number_binary_op_wrapper!(__lshift__));
            }
            _ if name == identifier!(ctx, __rlshift__) => {
                toggle_subslot!(
                    as_number,
                    right_lshift,
                    number_binary_right_op_wrapper!(__rlshift__)
                );
            }
            _ if name == identifier!(ctx, __ilshift__) => {
                toggle_subslot!(
                    as_number,
                    inplace_lshift,
                    number_binary_op_wrapper!(__ilshift__)
                );
            }
            _ if name == identifier!(ctx, __rshift__) => {
                toggle_subslot!(as_number, rshift, number_binary_op_wrapper!(__rshift__));
            }
            _ if name == identifier!(ctx, __rrshift__) => {
                toggle_subslot!(
                    as_number,
                    right_rshift,
                    number_binary_right_op_wrapper!(__rrshift__)
                );
            }
            _ if name == identifier!(ctx, __irshift__) => {
                toggle_subslot!(
                    as_number,
                    inplace_rshift,
                    number_binary_op_wrapper!(__irshift__)
                );
            }
            _ if name == identifier!(ctx, __and__) => {
                toggle_subslot!(as_number, and, number_binary_op_wrapper!(__and__));
            }
            _ if name == identifier!(ctx, __rand__) => {
                toggle_subslot!(
                    as_number,
                    right_and,
                    number_binary_right_op_wrapper!(__rand__)
                );
            }
            _ if name == identifier!(ctx, __iand__) => {
                toggle_subslot!(as_number, inplace_and, number_binary_op_wrapper!(__iand__));
            }
            _ if name == identifier!(ctx, __xor__) => {
                toggle_subslot!(as_number, xor, number_binary_op_wrapper!(__xor__));
            }
            _ if name == identifier!(ctx, __rxor__) => {
                toggle_subslot!(
                    as_number,
                    right_xor,
                    number_binary_right_op_wrapper!(__rxor__)
                );
            }
            _ if name == identifier!(ctx, __ixor__) => {
                toggle_subslot!(as_number, inplace_xor, number_binary_op_wrapper!(__ixor__));
            }
            _ if name == identifier!(ctx, __or__) => {
                toggle_subslot!(as_number, or, number_binary_op_wrapper!(__or__));
            }
            _ if name == identifier!(ctx, __ror__) => {
                toggle_subslot!(
                    as_number,
                    right_or,
                    number_binary_right_op_wrapper!(__ror__)
                );
            }
            _ if name == identifier!(ctx, __ior__) => {
                toggle_subslot!(as_number, inplace_or, number_binary_op_wrapper!(__ior__));
            }
            _ if name == identifier!(ctx, __floordiv__) => {
                toggle_subslot!(
                    as_number,
                    floor_divide,
                    number_binary_op_wrapper!(__floordiv__)
                );
            }
            _ if name == identifier!(ctx, __rfloordiv__) => {
                toggle_subslot!(
                    as_number,
                    right_floor_divide,
                    number_binary_right_op_wrapper!(__rfloordiv__)
                );
            }
            _ if name == identifier!(ctx, __ifloordiv__) => {
                toggle_subslot!(
                    as_number,
                    inplace_floor_divide,
                    number_binary_op_wrapper!(__ifloordiv__)
                );
            }
            _ if name == identifier!(ctx, __truediv__) => {
                toggle_subslot!(
                    as_number,
                    true_divide,
                    number_binary_op_wrapper!(__truediv__)
                );
            }
            _ if name == identifier!(ctx, __rtruediv__) => {
                toggle_subslot!(
                    as_number,
                    right_true_divide,
                    number_binary_right_op_wrapper!(__rtruediv__)
                );
            }
            _ if name == identifier!(ctx, __itruediv__) => {
                toggle_subslot!(
                    as_number,
                    inplace_true_divide,
                    number_binary_op_wrapper!(__itruediv__)
                );
            }
            _ if name == identifier!(ctx, __matmul__) => {
                toggle_subslot!(
                    as_number,
                    matrix_multiply,
                    number_binary_op_wrapper!(__matmul__)
                );
            }
            _ if name == identifier!(ctx, __rmatmul__) => {
                toggle_subslot!(
                    as_number,
                    right_matrix_multiply,
                    number_binary_right_op_wrapper!(__rmatmul__)
                );
            }
            _ if name == identifier!(ctx, __imatmul__) => {
                toggle_subslot!(
                    as_number,
                    inplace_matrix_multiply,
                    number_binary_op_wrapper!(__imatmul__)
                );
            }
            _ => {}
        }
    }
}

#[pyclass]
pub trait Constructor: PyPayload {
    type Args: FromArgs;

    #[inline]
    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        Self::py_new(cls, args, vm)
    }

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult;
}

#[pyclass]
pub trait DefaultConstructor: PyPayload + Default {
    #[inline]
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::default().into_ref_with_type(vm, cls).map(Into::into)
    }
}

/// For types that cannot be instantiated through Python code.
pub trait Unconstructible: PyPayload {}

impl<T> Constructor for T
where
    T: Unconstructible,
{
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error(format!("cannot create {} instances", cls.slot_name())))
    }
}

#[pyclass]
pub trait Initializer: PyPayload {
    type Args: FromArgs;

    #[pyslot]
    #[inline]
    fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let zelf = zelf.try_into_value(vm)?;
        let args: Self::Args = args.bind(vm)?;
        Self::init(zelf, args, vm)
    }

    #[pymethod]
    #[inline]
    fn __init__(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
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
            .ok_or_else(|| vm.new_type_error("unexpected payload for __del__".to_owned()))?;
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
        let Some(zelf) = zelf.downcast_ref() else {
            let err = vm.new_downcast_type_error(Self::class(&vm.ctx), zelf);
            return Err(err);
        };
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
    #[pymethod(magic)]
    fn get(
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
        cls.as_ref().map_or(false, |cls| other.borrow().is(cls))
    }
}

#[pyclass]
pub trait Hashable: PyPayload {
    #[inline]
    #[pyslot]
    fn slot_hash(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __hash__".to_owned()))?;
        Self::hash(zelf, vm)
    }

    #[inline]
    #[pymethod]
    fn __hash__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::slot_hash(&zelf, vm)
    }

    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
}

#[pyclass]
pub trait Representable: PyPayload {
    #[inline]
    #[pyslot]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __repr__".to_owned()))?;
        Self::repr(zelf, vm)
    }

    #[inline]
    #[pymethod]
    fn __repr__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Self::slot_repr(&zelf, vm)
    }

    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
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
    #[pymethod(magic)]
    fn eq(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Eq, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn ne(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Ne, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn lt(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Lt, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn le(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Le, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn ge(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, &other, PyComparisonOp::Ge, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn gt(zelf: &Py<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyComparisonValue> {
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

    pub fn eval_ord(self, ord: Ordering) -> bool {
        let bit = match ord {
            Ordering::Less => Self::Lt,
            Ordering::Equal => Self::Eq,
            Ordering::Greater => Self::Gt,
        };
        self.0 as u8 & bit.0 as u8 != 0
    }

    pub fn swapped(self) -> Self {
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

    pub fn operator_token(self) -> &'static str {
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
        let zelf = obj.downcast_ref().ok_or_else(|| {
            vm.new_type_error("unexpected payload for __getattribute__".to_owned())
        })?;
        Self::getattro(zelf, name, vm)
    }

    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult;

    #[inline]
    #[pymethod(magic)]
    fn getattribute(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(&zelf, &name, vm)
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
            .ok_or_else(|| vm.new_type_error("unexpected payload for __setattr__".to_owned()))?;
        Self::setattro(zelf, name, value, vm)
    }

    fn setattro(
        zelf: &Py<Self>,
        name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()>;

    #[inline]
    #[pymethod(magic)]
    fn setattr(
        zelf: PyRef<Self>,
        name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::setattro(&zelf, &name, PySetterValue::Assign(value), vm)
    }

    #[inline]
    #[pymethod(magic)]
    fn delattr(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::setattro(&zelf, &name, PySetterValue::Delete, vm)
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
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer".to_owned()))?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer>;
}

#[pyclass]
pub trait AsMapping: PyPayload {
    #[pyslot]
    fn as_mapping() -> &'static PyMappingMethods;

    #[inline]
    fn mapping_downcast(mapping: PyMapping) -> &Py<Self> {
        unsafe { mapping.obj.downcast_unchecked_ref() }
    }
}

#[pyclass]
pub trait AsSequence: PyPayload {
    #[pyslot]
    fn as_sequence() -> &'static PySequenceMethods;

    #[inline]
    fn sequence_downcast(seq: PySequence) -> &Py<Self> {
        unsafe { seq.obj.downcast_unchecked_ref() }
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
    fn number_downcast(num: PyNumber) -> &Py<Self> {
        unsafe { num.obj().downcast_unchecked_ref() }
    }

    #[inline]
    fn number_downcast_exact(num: PyNumber, vm: &VirtualMachine) -> PyRef<Self> {
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
    #[pymethod(name = "__iter__")]
    fn slot_iter(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let zelf = zelf
            .downcast()
            .map_err(|_| vm.new_type_error("unexpected payload for __iter__".to_owned()))?;
        Self::iter(zelf, vm)
    }

    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult;
}

// `Iterator` fits better, but to avoid confusion with rust std::iter::Iterator
#[pyclass(with(Iterable))]
pub trait IterNext: PyPayload + Iterable {
    #[pyslot]
    fn slot_iternext(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __next__".to_owned()))?;
        Self::next(zelf, vm)
    }

    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn>;

    #[inline]
    #[pymethod]
    fn __next__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::slot_iternext(&zelf, vm).to_pyresult(vm)
    }
}

pub trait IterNextIterable: PyPayload {}

impl<T> Iterable for T
where
    T: IterNextIterable,
{
    #[inline]
    fn slot_iter(zelf: PyObjectRef, _vm: &VirtualMachine) -> PyResult {
        Ok(zelf)
    }

    #[cold]
    fn iter(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
        unreachable!("slot_iter is implemented");
    }
}

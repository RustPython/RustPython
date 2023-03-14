use crate::{
    builtins::{type_::PointerSlot, PyFloat, PyInt, PyStrInterned, PyStrRef, PyType, PyTypeRef},
    bytecode::ComparisonOperator,
    common::{hash::PyHash, lock::PyRwLock},
    convert::{ToPyObject, ToPyResult},
    function::{Either, FromArgs, FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
    identifier,
    protocol::{
        PyBuffer, PyIterReturn, PyMapping, PyMappingMethods, PyNumber, PyNumberBinaryFunc,
        PyNumberBinaryOp, PyNumberMethods, PyNumberUnaryFunc, PySequence, PySequenceMethods,
    },
    vm::Context,
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use std::{borrow::Borrow, cmp::Ordering};

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
    pub name: PyRwLock<Option<String>>, // tp_name, not class name

    pub basicsize: usize,
    // tp_itemsize

    // Methods to implement standard operations

    // Method suites for standard classes
    pub as_number: AtomicCell<Option<PointerSlot<PyNumberMethods>>>,
    pub as_sequence: AtomicCell<Option<PointerSlot<PySequenceMethods>>>,
    pub as_mapping: AtomicCell<Option<PointerSlot<PyMappingMethods>>>,

    // More standard operations (here for binary compatibility)
    pub hash: AtomicCell<Option<HashFunc>>,
    pub call: AtomicCell<Option<GenericMethod>>,
    // tp_str
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
    pub number: PyNumberSlots,
}

impl PyTypeSlots {
    pub fn from_flags(flags: PyTypeFlags) -> Self {
        Self {
            flags,
            ..Default::default()
        }
    }
}

impl std::fmt::Debug for PyTypeSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PyTypeSlots")
    }
}

#[derive(Default)]
pub struct PyNumberSlots {
    pub add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub power: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub negative: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub positive: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub absolute: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub boolean: AtomicCell<Option<PyNumberUnaryFunc<bool>>>,
    pub invert: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub or: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub int: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyInt>>>>,
    pub float: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyFloat>>>>,

    pub right_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_power: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_or: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub inplace_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_power: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_or: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub index: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyInt>>>>,

    pub matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
}

impl PyNumberSlots {
    pub fn left_binary_op(
        &self,
        op_slot: PyNumberBinaryOp,
    ) -> PyResult<Option<PyNumberBinaryFunc>> {
        use PyNumberBinaryOp::*;
        let binary_op = match op_slot {
            Add => self.add.load(),
            Subtract => self.subtract.load(),
            Multiply => self.multiply.load(),
            Remainder => self.remainder.load(),
            Divmod => self.divmod.load(),
            Power => self.power.load(),
            Lshift => self.lshift.load(),
            Rshift => self.rshift.load(),
            And => self.and.load(),
            Xor => self.xor.load(),
            Or => self.or.load(),
            InplaceAdd => self.inplace_add.load(),
            InplaceSubtract => self.inplace_subtract.load(),
            InplaceMultiply => self.inplace_multiply.load(),
            InplaceRemainder => self.inplace_remainder.load(),
            InplacePower => self.inplace_power.load(),
            InplaceLshift => self.inplace_lshift.load(),
            InplaceRshift => self.inplace_rshift.load(),
            InplaceAnd => self.inplace_and.load(),
            InplaceXor => self.inplace_xor.load(),
            InplaceOr => self.inplace_or.load(),
            FloorDivide => self.floor_divide.load(),
            TrueDivide => self.true_divide.load(),
            InplaceFloorDivide => self.inplace_floor_divide.load(),
            InplaceTrueDivide => self.inplace_true_divide.load(),
            MatrixMultiply => self.matrix_multiply.load(),
            InplaceMatrixMultiply => self.inplace_matrix_multiply.load(),
        };
        Ok(binary_op)
    }

    pub fn right_binary_op(
        &self,
        op_slot: PyNumberBinaryOp,
    ) -> PyResult<Option<PyNumberBinaryFunc>> {
        use PyNumberBinaryOp::*;
        let binary_op = match op_slot {
            Add => self.right_add.load(),
            Subtract => self.right_subtract.load(),
            Multiply => self.right_multiply.load(),
            Remainder => self.right_remainder.load(),
            Divmod => self.right_divmod.load(),
            Power => self.right_power.load(),
            Lshift => self.right_lshift.load(),
            Rshift => self.right_rshift.load(),
            And => self.right_and.load(),
            Xor => self.right_xor.load(),
            Or => self.right_or.load(),
            FloorDivide => self.right_floor_divide.load(),
            TrueDivide => self.right_true_divide.load(),
            MatrixMultiply => self.right_matrix_multiply.load(),
            _ => None,
        };
        Ok(binary_op)
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
pub(crate) type GetattroFunc = fn(&PyObject, PyStrRef, &VirtualMachine) -> PyResult;
pub(crate) type SetattroFunc =
    fn(&PyObject, PyStrRef, PySetterValue, &VirtualMachine) -> PyResult<()>;
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
    fn(PyObjectRef, PyObjectRef, PySetterValue, &VirtualMachine) -> PyResult<()>;
pub(crate) type NewFunc = fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type InitFunc = fn(PyObjectRef, FuncArgs, &VirtualMachine) -> PyResult<()>;
pub(crate) type DelFunc = fn(&PyObject, &VirtualMachine) -> PyResult<()>;

// slot_sq_length
pub(crate) fn len_wrapper(obj: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    let ret = vm.call_special_method(obj.to_owned(), identifier!(vm, __len__), ())?;
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

fn int_wrapper(num: PyNumber, vm: &VirtualMachine) -> PyResult<PyRef<PyInt>> {
    let ret = vm.call_special_method(num.obj.to_owned(), identifier!(vm, __int__), ())?;
    ret.downcast::<PyInt>().map_err(|obj| {
        vm.new_type_error(format!("__int__ returned non-int (type {})", obj.class()))
    })
}

fn index_wrapper(num: PyNumber, vm: &VirtualMachine) -> PyResult<PyRef<PyInt>> {
    let ret = vm.call_special_method(num.obj.to_owned(), identifier!(vm, __index__), ())?;
    ret.downcast::<PyInt>().map_err(|obj| {
        vm.new_type_error(format!("__index__ returned non-int (type {})", obj.class()))
    })
}

fn float_wrapper(num: PyNumber, vm: &VirtualMachine) -> PyResult<PyRef<PyFloat>> {
    let ret = vm.call_special_method(num.obj.to_owned(), identifier!(vm, __float__), ())?;
    ret.downcast::<PyFloat>().map_err(|obj| {
        vm.new_type_error(format!(
            "__float__ returned non-float (type {})",
            obj.class()
        ))
    })
}

macro_rules! number_binary_op_wrapper {
    ($name:ident) => {
        |num, other, vm| {
            vm.call_special_method(
                num.obj.to_owned(),
                identifier!(vm, $name),
                (other.to_owned(),),
            )
        }
    };
}

fn getitem_wrapper<K: ToPyObject>(obj: &PyObject, needle: K, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(obj.to_owned(), identifier!(vm, __getitem__), (needle,))
}

fn setitem_wrapper<K: ToPyObject>(
    obj: &PyObject,
    needle: K,
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match value {
        Some(value) => vm.call_special_method(
            obj.to_owned(),
            identifier!(vm, __setitem__),
            (needle, value),
        ),
        None => vm.call_special_method(obj.to_owned(), identifier!(vm, __delitem__), (needle,)),
    }
    .map(drop)
}

fn hash_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    let hash_obj = vm.call_special_method(zelf.to_owned(), identifier!(vm, __hash__), ())?;
    match hash_obj.payload_if_subclass::<PyInt>(vm) {
        Some(py_int) => Ok(rustpython_common::hash::hash_bigint(py_int.as_bigint())),
        None => Err(vm.new_type_error("__hash__ method should return an integer".to_owned())),
    }
}

/// Marks a type as unhashable. Similar to PyObject_HashNotImplemented in CPython
pub fn hash_not_implemented(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    Err(vm.new_type_error(format!("unhashable type: {}", zelf.class().name())))
}

fn call_wrapper(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf.to_owned(), identifier!(vm, __call__), args)
}

fn getattro_wrapper(zelf: &PyObject, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    let __getattribute__ = identifier!(vm, __getattribute__);
    let __getattr__ = identifier!(vm, __getattr__);
    match vm.call_special_method(zelf.to_owned(), __getattribute__, (name.clone(),)) {
        Ok(r) => Ok(r),
        Err(_) if zelf.class().has_attr(__getattr__) => {
            vm.call_special_method(zelf.to_owned(), __getattr__, (name,))
        }
        Err(e) => Err(e),
    }
}

fn setattro_wrapper(
    zelf: &PyObject,
    name: PyStrRef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let zelf = zelf.to_owned();
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
    vm.call_special_method(
        zelf.to_owned(),
        op.method_name(&vm.ctx),
        (other.to_owned(),),
    )
    .map(Either::A)
}

fn iter_wrapper(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf, identifier!(vm, __iter__), ())
}

fn iternext_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
    PyIterReturn::from_pyresult(
        vm.call_special_method(zelf.to_owned(), identifier!(vm, __next__), ()),
        vm,
    )
}

fn descr_get_wrapper(
    zelf: PyObjectRef,
    obj: Option<PyObjectRef>,
    cls: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    vm.call_special_method(zelf, identifier!(vm, __get__), (obj, cls))
}

fn descr_set_wrapper(
    zelf: PyObjectRef,
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
    let res = vm.call_special_method(obj, identifier!(vm, __init__), args)?;
    if !vm.is_none(&res) {
        return Err(vm.new_type_error("__init__ must return None".to_owned()));
    }
    Ok(())
}

// = slot_tp_new
pub(crate) fn new_wrapper(cls: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    let new = cls.get_attr(identifier!(vm, __new__)).unwrap();
    args.prepend_arg(cls.into());
    new.call(args, vm)
}

fn del_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
    vm.call_special_method(zelf.to_owned(), identifier!(vm, __del__), ())?;
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
                toggle_subslot!(number, int, int_wrapper);
            }
            _ if name == identifier!(ctx, __index__) => {
                toggle_subslot!(number, index, index_wrapper);
            }
            _ if name == identifier!(ctx, __float__) => {
                toggle_subslot!(number, float, float_wrapper);
            }
            _ if name == identifier!(ctx, __add__) => {
                toggle_subslot!(number, add, number_binary_op_wrapper!(__add__));
            }
            _ if name == identifier!(ctx, __radd__) => {
                toggle_subslot!(number, right_add, number_binary_op_wrapper!(__radd__));
            }
            _ if name == identifier!(ctx, __iadd__) => {
                toggle_subslot!(number, inplace_add, number_binary_op_wrapper!(__iadd__));
            }
            _ if name == identifier!(ctx, __sub__) => {
                toggle_subslot!(number, subtract, number_binary_op_wrapper!(__sub__));
            }
            _ if name == identifier!(ctx, __rsub__) => {
                toggle_subslot!(number, right_subtract, number_binary_op_wrapper!(__rsub__));
            }
            _ if name == identifier!(ctx, __isub__) => {
                toggle_subslot!(
                    number,
                    inplace_subtract,
                    number_binary_op_wrapper!(__isub__)
                );
            }
            _ if name == identifier!(ctx, __mul__) => {
                toggle_subslot!(number, multiply, number_binary_op_wrapper!(__mul__));
            }
            _ if name == identifier!(ctx, __rmul__) => {
                toggle_subslot!(number, right_multiply, number_binary_op_wrapper!(__rmul__));
            }
            _ if name == identifier!(ctx, __imul__) => {
                toggle_subslot!(
                    number,
                    inplace_multiply,
                    number_binary_op_wrapper!(__imul__)
                );
            }
            _ if name == identifier!(ctx, __mod__) => {
                toggle_subslot!(number, remainder, number_binary_op_wrapper!(__mod__));
            }
            _ if name == identifier!(ctx, __rmod__) => {
                toggle_subslot!(number, right_remainder, number_binary_op_wrapper!(__rmod__));
            }
            _ if name == identifier!(ctx, __imod__) => {
                toggle_subslot!(
                    number,
                    inplace_remainder,
                    number_binary_op_wrapper!(__imod__)
                );
            }
            _ if name == identifier!(ctx, __divmod__) => {
                toggle_subslot!(number, divmod, number_binary_op_wrapper!(__divmod__));
            }
            _ if name == identifier!(ctx, __rdivmod__) => {
                toggle_subslot!(number, right_divmod, number_binary_op_wrapper!(__rdivmod__));
            }
            _ if name == identifier!(ctx, __pow__) => {
                toggle_subslot!(number, power, number_binary_op_wrapper!(__pow__));
            }
            _ if name == identifier!(ctx, __rpow__) => {
                toggle_subslot!(number, right_power, number_binary_op_wrapper!(__rpow__));
            }
            _ if name == identifier!(ctx, __ipow__) => {
                toggle_subslot!(number, inplace_power, number_binary_op_wrapper!(__ipow__));
            }
            _ if name == identifier!(ctx, __lshift__) => {
                toggle_subslot!(number, lshift, number_binary_op_wrapper!(__lshift__));
            }
            _ if name == identifier!(ctx, __rlshift__) => {
                toggle_subslot!(number, right_lshift, number_binary_op_wrapper!(__rlshift__));
            }
            _ if name == identifier!(ctx, __ilshift__) => {
                toggle_subslot!(
                    number,
                    inplace_lshift,
                    number_binary_op_wrapper!(__ilshift__)
                );
            }
            _ if name == identifier!(ctx, __rshift__) => {
                toggle_subslot!(number, rshift, number_binary_op_wrapper!(__rshift__));
            }
            _ if name == identifier!(ctx, __rrshift__) => {
                toggle_subslot!(number, right_rshift, number_binary_op_wrapper!(__rrshift__));
            }
            _ if name == identifier!(ctx, __irshift__) => {
                toggle_subslot!(
                    number,
                    inplace_rshift,
                    number_binary_op_wrapper!(__irshift__)
                );
            }
            _ if name == identifier!(ctx, __and__) => {
                toggle_subslot!(number, and, number_binary_op_wrapper!(__and__));
            }
            _ if name == identifier!(ctx, __rand__) => {
                toggle_subslot!(number, right_and, number_binary_op_wrapper!(__rand__));
            }
            _ if name == identifier!(ctx, __iand__) => {
                toggle_subslot!(number, inplace_and, number_binary_op_wrapper!(__iand__));
            }
            _ if name == identifier!(ctx, __xor__) => {
                toggle_subslot!(number, xor, number_binary_op_wrapper!(__xor__));
            }
            _ if name == identifier!(ctx, __rxor__) => {
                toggle_subslot!(number, right_xor, number_binary_op_wrapper!(__rxor__));
            }
            _ if name == identifier!(ctx, __ixor__) => {
                toggle_subslot!(number, inplace_xor, number_binary_op_wrapper!(__ixor__));
            }
            _ if name == identifier!(ctx, __or__) => {
                toggle_subslot!(number, or, number_binary_op_wrapper!(__or__));
            }
            _ if name == identifier!(ctx, __ror__) => {
                toggle_subslot!(number, right_or, number_binary_op_wrapper!(__ror__));
            }
            _ if name == identifier!(ctx, __ior__) => {
                toggle_subslot!(number, inplace_or, number_binary_op_wrapper!(__ior__));
            }
            _ if name == identifier!(ctx, __floordiv__) => {
                toggle_subslot!(
                    number,
                    floor_divide,
                    number_binary_op_wrapper!(__floordiv__)
                );
            }
            _ if name == identifier!(ctx, __rfloordiv__) => {
                toggle_subslot!(
                    number,
                    right_floor_divide,
                    number_binary_op_wrapper!(__rfloordiv__)
                );
            }
            _ if name == identifier!(ctx, __ifloordiv__) => {
                toggle_subslot!(
                    number,
                    inplace_floor_divide,
                    number_binary_op_wrapper!(__ifloordiv__)
                );
            }
            _ if name == identifier!(ctx, __truediv__) => {
                toggle_subslot!(number, true_divide, number_binary_op_wrapper!(__truediv__));
            }
            _ if name == identifier!(ctx, __rtruediv__) => {
                toggle_subslot!(
                    number,
                    right_true_divide,
                    number_binary_op_wrapper!(__rtruediv__)
                );
            }
            _ if name == identifier!(ctx, __itruediv__) => {
                toggle_subslot!(
                    number,
                    inplace_true_divide,
                    number_binary_op_wrapper!(__itruediv__)
                );
            }
            _ if name == identifier!(ctx, __matmul__) => {
                toggle_subslot!(
                    number,
                    matrix_multiply,
                    number_binary_op_wrapper!(__matmul__)
                );
            }
            _ if name == identifier!(ctx, __rmatmul__) => {
                toggle_subslot!(
                    number,
                    right_matrix_multiply,
                    number_binary_op_wrapper!(__rmatmul__)
                );
            }
            _ if name == identifier!(ctx, __imatmul__) => {
                toggle_subslot!(
                    number,
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
        if let Some(zelf) = zelf.downcast_ref() {
            Self::del(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __del__".to_owned()))
        }
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
        if let Some(zelf) = zelf.downcast_ref() {
            Self::call(zelf, args.bind(vm)?, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __call__".to_owned()))
        }
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
    fn _zelf(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_into_value(vm)
    }

    #[inline]
    fn _unwrap(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyRef<Self>, PyObjectRef)> {
        let zelf = Self::_zelf(zelf, vm)?;
        let obj = vm.unwrap_or_none(obj);
        Ok((zelf, obj))
    }

    #[inline]
    fn _check(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> Result<(PyRef<Self>, PyObjectRef), PyResult> {
        // CPython descr_check
        if let Some(obj) = obj {
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
            Ok((Self::_zelf(zelf, vm).unwrap(), obj))
        // }
        } else {
            Err(Ok(zelf))
        }
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
        if let Some(zelf) = zelf.downcast_ref() {
            Self::hash(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __hash__".to_owned()))
        }
    }

    #[inline]
    #[pymethod]
    fn __hash__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::slot_hash(&zelf, vm)
    }

    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
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
        if let Some(zelf) = zelf.downcast_ref() {
            Self::cmp(zelf, other, op, vm).map(Either::B)
        } else {
            Err(vm.new_type_error(format!(
                "unexpected payload for {}",
                op.method_name(&vm.ctx).as_str()
            )))
        }
    }

    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue>;

    #[pymethod(magic)]
    fn eq(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Eq, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn ne(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ne, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn lt(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Lt, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn le(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Le, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn ge(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ge, vm)
    }
    #[inline]
    #[pymethod(magic)]
    fn gt(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Gt, vm)
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
        if f() {
            Some(eq)
        } else {
            None
        }
    }
}

#[pyclass]
pub trait GetAttr: PyPayload {
    #[pyslot]
    fn slot_getattro(obj: &PyObject, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Some(zelf) = obj.downcast_ref::<Self>() {
            Self::getattro(zelf, name, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __getattribute__".to_owned()))
        }
    }

    fn getattro(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult;

    #[inline]
    #[pymethod(magic)]
    fn getattribute(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(&zelf, name, vm)
    }
}

#[pyclass]
pub trait SetAttr: PyPayload {
    #[pyslot]
    #[inline]
    fn slot_setattro(
        obj: &PyObject,
        name: PyStrRef,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(zelf) = obj.downcast_ref::<Self>() {
            Self::setattro(zelf, name, value, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __setattr__".to_owned()))
        }
    }

    fn setattro(
        zelf: &Py<Self>,
        name: PyStrRef,
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
        Self::setattro(&zelf, name, PySetterValue::Assign(value), vm)
    }

    #[inline]
    #[pymethod(magic)]
    fn delattr(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::setattro(&zelf, name, PySetterValue::Delete, vm)
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

macro_rules! extend_number_slot {
    ($slots:ident, $methods:ident, $method:ident, $right_method:ident, $op_slot:ident) => {
        if $methods.$method.is_some() {
            $slots.number.$method.store($methods.$method);
            $slots.number.$right_method.store(Some(|num, other, vm| {
                num.methods.binary_op(PyNumberBinaryOp::$op_slot).unwrap()(
                    other.to_number(),
                    num.obj,
                    vm,
                )
            }));
        }
    };
    ($slots:ident, $methods:ident, $method:ident) => {
        if $methods.$method.is_some() {
            $slots.number.$method.store($methods.$method);
        }
    };
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
        unsafe { num.obj.downcast_unchecked_ref() }
    }

    #[inline]
    fn number_downcast_exact(number: PyNumber, vm: &VirtualMachine) -> PyRef<Self> {
        if let Some(zelf) = number.obj.downcast_ref_if_exact::<Self>(vm) {
            zelf.to_owned()
        } else {
            Self::clone_exact(Self::number_downcast(number), vm)
        }
    }

    fn extend_slots(slots: &mut PyTypeSlots) {
        let methods = Self::as_number();

        extend_number_slot!(slots, methods, add, right_add, Add);
        extend_number_slot!(slots, methods, subtract, right_subtract, Subtract);
        extend_number_slot!(slots, methods, multiply, right_multiply, Multiply);
        extend_number_slot!(slots, methods, remainder, right_remainder, Remainder);
        extend_number_slot!(slots, methods, divmod, right_divmod, Divmod);
        extend_number_slot!(slots, methods, power, right_power, Power);
        extend_number_slot!(slots, methods, lshift, right_lshift, Lshift);
        extend_number_slot!(slots, methods, rshift, right_rshift, Rshift);
        extend_number_slot!(slots, methods, and, right_and, And);
        extend_number_slot!(slots, methods, xor, right_xor, Xor);
        extend_number_slot!(slots, methods, or, right_or, Or);
        extend_number_slot!(
            slots,
            methods,
            floor_divide,
            right_floor_divide,
            FloorDivide
        );
        extend_number_slot!(slots, methods, true_divide, right_true_divide, TrueDivide);
        extend_number_slot!(
            slots,
            methods,
            matrix_multiply,
            right_matrix_multiply,
            MatrixMultiply
        );

        extend_number_slot!(slots, methods, negative);
        extend_number_slot!(slots, methods, positive);
        extend_number_slot!(slots, methods, absolute);
        extend_number_slot!(slots, methods, boolean);
        extend_number_slot!(slots, methods, invert);
        extend_number_slot!(slots, methods, int);
        extend_number_slot!(slots, methods, float);
        extend_number_slot!(slots, methods, index);

        extend_number_slot!(slots, methods, inplace_add);
        extend_number_slot!(slots, methods, inplace_subtract);
        extend_number_slot!(slots, methods, inplace_multiply);
        extend_number_slot!(slots, methods, inplace_remainder);
        extend_number_slot!(slots, methods, inplace_power);
        extend_number_slot!(slots, methods, inplace_lshift);
        extend_number_slot!(slots, methods, inplace_rshift);
        extend_number_slot!(slots, methods, inplace_and);
        extend_number_slot!(slots, methods, inplace_xor);
        extend_number_slot!(slots, methods, inplace_or);
        extend_number_slot!(slots, methods, inplace_floor_divide);
        extend_number_slot!(slots, methods, inplace_true_divide);
        extend_number_slot!(slots, methods, inplace_matrix_multiply);
    }
}

#[pyclass]
pub trait Iterable: PyPayload {
    #[pyslot]
    #[pymethod(name = "__iter__")]
    fn slot_iter(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(zelf) = zelf.downcast() {
            Self::iter(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __iter__".to_owned()))
        }
    }

    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult;
}

// `Iterator` fits better, but to avoid confusion with rust std::iter::Iterator
#[pyclass(with(Iterable))]
pub trait IterNext: PyPayload + Iterable {
    #[pyslot]
    fn slot_iternext(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::next(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __next__".to_owned()))
        }
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

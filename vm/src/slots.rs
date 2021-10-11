use crate::common::{hash::PyHash, lock::PyRwLock};
use crate::{
    builtins::{PyStrRef, PyTypeRef},
    function::{FromArgs, FuncArgs, IntoPyResult, OptionalArg},
    protocol::{PyBuffer, PyIterReturn, PyMappingMethods},
    utils::Either,
    IdProtocol, PyComparisonValue, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
    VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use std::cmp::Ordering;

bitflags! {
    pub struct PyTypeFlags: u64 {
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
    pub fn heap_type_flags() -> Self {
        Self::DEFAULT | Self::HEAPTYPE | Self::BASETYPE
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

pub(crate) type GenericMethod = fn(&PyObjectRef, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type NewFunc = fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type DelFunc = fn(&PyObjectRef, &VirtualMachine) -> PyResult<()>;
pub(crate) type DescrGetFunc =
    fn(PyObjectRef, Option<PyObjectRef>, Option<PyObjectRef>, &VirtualMachine) -> PyResult;
pub(crate) type DescrSetFunc =
    fn(PyObjectRef, PyObjectRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>;
pub(crate) type HashFunc = fn(&PyObjectRef, &VirtualMachine) -> PyResult<PyHash>;
pub(crate) type RichCompareFunc = fn(
    &PyObjectRef,
    &PyObjectRef,
    PyComparisonOp,
    &VirtualMachine,
) -> PyResult<Either<PyObjectRef, PyComparisonValue>>;
pub(crate) type GetattroFunc = fn(PyObjectRef, PyStrRef, &VirtualMachine) -> PyResult;
pub(crate) type SetattroFunc =
    fn(&PyObjectRef, PyStrRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>;
pub(crate) type BufferFunc = fn(&PyObjectRef, &VirtualMachine) -> PyResult<PyBuffer>;
pub(crate) type MappingFunc = fn(&PyObjectRef, &VirtualMachine) -> PyResult<PyMappingMethods>;
pub(crate) type IterFunc = fn(PyObjectRef, &VirtualMachine) -> PyResult;
pub(crate) type IterNextFunc = fn(&PyObjectRef, &VirtualMachine) -> PyResult<PyIterReturn>;

// The corresponding field in CPython is `tp_` prefixed.
// e.g. name -> tp_name
#[derive(Default)]
pub struct PyTypeSlots {
    pub name: PyRwLock<Option<String>>, // tp_name, not class name
    // tp_basicsize, tp_itemsize

    // Methods to implement standard operations

    // Method suites for standard classes
    // tp_as_number
    // tp_as_sequence
    pub as_mapping: AtomicCell<Option<MappingFunc>>,

    // More standard operations (here for binary compatibility)
    pub hash: AtomicCell<Option<HashFunc>>,
    pub call: AtomicCell<Option<GenericMethod>>,
    // tp_str
    pub getattro: AtomicCell<Option<GetattroFunc>>,
    pub setattro: AtomicCell<Option<SetattroFunc>>,

    // Functions to access object as input/output buffer
    pub as_buffer: Option<BufferFunc>,

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
    // tp_init
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

#[pyimpl]
pub trait SlotConstructor: PyValue {
    type Args: FromArgs;

    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        Self::py_new(cls, args, vm)
    }

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult;
}

#[pyimpl]
pub trait SlotDestructor: PyValue {
    #[inline] // for __del__
    #[pyslot]
    fn slot_del(zelf: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
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

    fn del(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<()>;
}

#[pyimpl]
pub trait Callable: PyValue {
    #[pyslot]
    fn slot_call(zelf: &PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::call(zelf, args, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __call__".to_owned()))
        }
    }
    #[pymethod]
    fn __call__(zelf: PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Self::call(&zelf, args, vm)
    }
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult;
}

#[pyimpl]
pub trait SlotDescriptor: PyValue {
    #[pyslot]
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult;

    #[pymethod(magic)]
    fn get(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Self::descr_get(zelf, Some(obj), cls.into_option(), vm)
    }

    fn _zelf(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_into_value(vm)
    }

    fn _unwrap(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyRef<Self>, PyObjectRef)> {
        let zelf = Self::_zelf(zelf, vm)?;
        let obj = vm.unwrap_or_none(obj);
        Ok((zelf, obj))
    }

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

    fn _cls_is<T>(cls: &Option<PyObjectRef>, other: &T) -> bool
    where
        T: IdProtocol,
    {
        cls.as_ref().map_or(false, |cls| other.is(cls))
    }
}

#[pyimpl]
pub trait Hashable: PyValue {
    #[pyslot]
    fn slot_hash(zelf: &PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::hash(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __hash__".to_owned()))
        }
    }

    #[pymethod]
    fn __hash__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::hash(&zelf, vm)
    }

    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
}

pub trait Unhashable: PyValue {}

impl<T> Hashable for T
where
    T: Unhashable,
{
    fn hash(_zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Err(vm.new_type_error(format!("unhashable type: '{}'", _zelf.class().name())))
    }
}

#[pyimpl]
pub trait Comparable: PyValue {
    #[pyslot]
    fn slot_richcompare(
        zelf: &PyObjectRef,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::cmp(zelf, other, op, vm).map(Either::B)
        } else {
            Err(vm.new_type_error(format!("unexpected payload for {}", op.method_name())))
        }
    }

    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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
    #[pymethod(magic)]
    fn ne(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ne, vm)
    }
    #[pymethod(magic)]
    fn lt(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Lt, vm)
    }
    #[pymethod(magic)]
    fn le(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Le, vm)
    }
    #[pymethod(magic)]
    fn ge(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ge, vm)
    }
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
pub enum PyComparisonOp {
    // be intentional with bits so that we can do eval_ord with just a bitwise and
    // bits: | Equal | Greater | Less |
    Lt = 0b001,
    Gt = 0b010,
    Ne = 0b011,
    Eq = 0b100,
    Le = 0b101,
    Ge = 0b110,
}

use PyComparisonOp::*;
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
            Ordering::Less => Lt,
            Ordering::Equal => Eq,
            Ordering::Greater => Gt,
        };
        self as u8 & bit as u8 != 0
    }

    pub fn swapped(self) -> Self {
        match self {
            Lt => Gt,
            Le => Ge,
            Eq => Eq,
            Ne => Ne,
            Ge => Le,
            Gt => Lt,
        }
    }

    pub fn method_name(self) -> &'static str {
        match self {
            Lt => "__lt__",
            Le => "__le__",
            Eq => "__eq__",
            Ne => "__ne__",
            Ge => "__ge__",
            Gt => "__gt__",
        }
    }

    pub fn operator_token(self) -> &'static str {
        match self {
            Lt => "<",
            Le => "<=",
            Eq => "==",
            Ne => "!=",
            Ge => ">=",
            Gt => ">",
        }
    }

    /// Returns an appropriate return value for the comparison when a and b are the same object, if an
    /// appropriate return value exists.
    pub fn identical_optimization(self, a: &impl IdProtocol, b: &impl IdProtocol) -> Option<bool> {
        self.map_eq(|| a.is(b))
    }

    /// Returns `Some(true)` when self is `Eq` and `f()` returns true. Returns `Some(false)` when self
    /// is `Ne` and `f()` returns true. Otherwise returns `None`.
    pub fn map_eq(self, f: impl FnOnce() -> bool) -> Option<bool> {
        match self {
            Self::Eq => {
                if f() {
                    Some(true)
                } else {
                    None
                }
            }
            Self::Ne => {
                if f() {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[pyimpl]
pub trait SlotGetattro: PyValue {
    #[pyslot]
    fn slot_getattro(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(zelf) = obj.downcast::<Self>() {
            Self::getattro(zelf, name, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __getattribute__".to_owned()))
        }
    }

    // TODO: make zelf: &PyRef<Self>
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult;

    #[pymethod]
    fn __getattribute__(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(zelf, name, vm)
    }
}

#[pyimpl]
pub trait SlotSetattro: PyValue {
    #[pyslot]
    fn slot_setattro(
        obj: &PyObjectRef,
        name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(zelf) = obj.downcast_ref::<Self>() {
            Self::setattro(zelf, name, value, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __setattr__".to_owned()))
        }
    }

    fn setattro(
        zelf: &PyRef<Self>,
        name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()>;

    #[pymethod]
    fn __setattr__(
        zelf: PyRef<Self>,
        name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::setattro(&zelf, name, Some(value), vm)
    }

    #[pymethod]
    fn __delattr__(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::setattro(&zelf, name, None, vm)
    }
}

#[pyimpl]
pub trait AsBuffer: PyValue {
    // TODO: `flags` parameter
    #[pyslot]
    fn slot_as_buffer(zelf: &PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer".to_owned()))?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer>;
}

#[pyimpl]
pub trait AsMapping: PyValue {
    #[pyslot]
    fn slot_as_mapping(zelf: &PyObjectRef, vm: &VirtualMachine) -> PyResult<PyMappingMethods> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_mapping".to_owned()))?;
        Self::as_mapping(zelf, vm)
    }

    fn downcast(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.downcast::<Self>().map_err(|obj| {
            vm.new_type_error(format!(
                "{} type is required, not {}",
                Self::class(vm),
                obj.class()
            ))
        })
    }

    fn downcast_ref<'a>(zelf: &'a PyObjectRef, vm: &VirtualMachine) -> PyResult<&'a PyRef<Self>> {
        zelf.downcast_ref::<Self>().ok_or_else(|| {
            vm.new_type_error(format!(
                "{} type is required, not {}",
                Self::class(vm),
                zelf.class()
            ))
        })
    }

    fn as_mapping(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyMappingMethods>;

    fn length(zelf: PyObjectRef, _vm: &VirtualMachine) -> PyResult<usize>;

    fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult;

    fn ass_subscript(
        zelf: PyObjectRef,
        needle: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()>;
}

#[pyimpl]
pub trait Iterable: PyValue {
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

#[pyimpl(with(Iterable))]
pub trait SlotIterator: PyValue + Iterable {
    #[pyslot]
    fn slot_iternext(zelf: &PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::next(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __next__".to_owned()))
        }
    }

    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn>;

    #[pymethod]
    fn __next__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::slot_iternext(&zelf, vm).into_pyresult(vm)
    }
}

pub trait IteratorIterable: PyValue {}

impl<T> Iterable for T
where
    T: IteratorIterable,
{
    fn slot_iter(zelf: PyObjectRef, _vm: &VirtualMachine) -> PyResult {
        Ok(zelf)
    }
    fn iter(_zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
        unreachable!("slot_iter is implemented");
    }
}

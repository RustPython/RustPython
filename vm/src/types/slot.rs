use crate::common::{hash::PyHash, lock::PyRwLock};
use crate::{
    builtins::{PyInt, PyStrRef, PyType, PyTypeRef},
    function::{FromArgs, FuncArgs, IntoPyResult, OptionalArg},
    protocol::{PyBuffer, PyIterReturn, PyMappingMethods},
    utils::Either,
    IdProtocol, PyComparisonValue, PyObject, PyObjectRef, PyObjectView, PyRef, PyResult, PyValue,
    TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use std::cmp::Ordering;

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
    pub as_mapping: AtomicCell<Option<AsMappingFunc>>,

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
pub(crate) type AsMappingFunc = fn(&PyObject, &VirtualMachine) -> PyMappingMethods;
pub(crate) type HashFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyHash>;
// CallFunc = GenericMethod
pub(crate) type GetattroFunc = fn(PyObjectRef, PyStrRef, &VirtualMachine) -> PyResult;
pub(crate) type SetattroFunc =
    fn(&PyObject, PyStrRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>;
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
    fn(PyObjectRef, PyObjectRef, Option<PyObjectRef>, &VirtualMachine) -> PyResult<()>;
pub(crate) type NewFunc = fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult;
pub(crate) type DelFunc = fn(&PyObject, &VirtualMachine) -> PyResult<()>;

pub use crate::builtins::object::{generic_getattr, generic_setattr};

fn as_mapping_wrapper(zelf: &PyObject, _vm: &VirtualMachine) -> PyMappingMethods {
    macro_rules! then_some_closure {
        ($cond:expr, $closure:expr) => {
            if $cond {
                Some($closure)
            } else {
                None
            }
        };
    }
    PyMappingMethods {
        length: then_some_closure!(zelf.has_class_attr("__len__"), |zelf, vm| {
            vm.call_special_method(zelf, "__len__", ()).map(|obj| {
                obj.payload_if_subclass::<PyInt>(vm)
                    .map(|length_obj| {
                        length_obj.as_bigint().to_usize().ok_or_else(|| {
                            vm.new_value_error("__len__() should return >= 0".to_owned())
                        })
                    })
                    .unwrap()
            })?
        }),
        subscript: then_some_closure!(
            zelf.has_class_attr("__getitem__"),
            |zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine| {
                vm.call_special_method(zelf, "__getitem__", (needle,))
            }
        ),
        ass_subscript: then_some_closure!(
            zelf.has_class_attr("__setitem__") | zelf.has_class_attr("__delitem__"),
            |zelf, needle, value, vm| match value {
                Some(value) => vm
                    .call_special_method(zelf, "__setitem__", (needle, value),)
                    .map(|_| Ok(()))?,
                None => vm
                    .call_special_method(zelf, "__delitem__", (needle,))
                    .map(|_| Ok(()))?,
            }
        ),
    }
}

fn hash_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    let hash_obj = vm.call_special_method(zelf.to_owned(), "__hash__", ())?;
    match hash_obj.payload_if_subclass::<PyInt>(vm) {
        Some(py_int) => Ok(rustpython_common::hash::hash_bigint(py_int.as_bigint())),
        None => Err(vm.new_type_error("__hash__ method should return an integer".to_owned())),
    }
}

fn call_wrapper(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf.to_owned(), "__call__", args)
}

fn getattro_wrapper(zelf: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf, "__getattribute__", (name,))
}

fn setattro_wrapper(
    zelf: &PyObject,
    name: PyStrRef,
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let zelf = zelf.to_owned();
    match value {
        Some(value) => {
            vm.call_special_method(zelf, "__setattr__", (name, value))?;
        }
        None => {
            vm.call_special_method(zelf, "__delattr__", (name,))?;
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
    vm.call_special_method(zelf.to_owned(), op.method_name(), (other.to_owned(),))
        .map(Either::A)
}

fn iter_wrapper(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf, "__iter__", ())
}

fn iternext_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
    PyIterReturn::from_pyresult(vm.call_special_method(zelf.to_owned(), "__next__", ()), vm)
}

fn descr_get_wrapper(
    zelf: PyObjectRef,
    obj: Option<PyObjectRef>,
    cls: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    vm.call_special_method(zelf, "__get__", (obj, cls))
}

fn descr_set_wrapper(
    zelf: PyObjectRef,
    obj: PyObjectRef,
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match value {
        Some(val) => vm.call_special_method(zelf, "__set__", (obj, val)),
        None => vm.call_special_method(zelf, "__delete__", (obj,)),
    }
    .map(drop)
}

fn new_wrapper(cls: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    let new = vm
        .get_attribute_opt(cls.as_object().to_owned(), "__new__")?
        .unwrap();
    args.prepend_arg(cls.into());
    vm.invoke(&new, args)
}

fn del_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
    vm.call_special_method(zelf.to_owned(), "__del__", ())?;
    Ok(())
}

impl PyType {
    pub(crate) fn update_slot(&self, name: &str, add: bool) {
        debug_assert!(name.starts_with("__"));
        debug_assert!(name.ends_with("__"));

        macro_rules! update_slot {
            ($name:ident, $func:expr) => {{
                self.slots.$name.store(if add { Some($func) } else { None });
            }};
        }
        match name {
            "__len__" | "__getitem__" | "__setitem__" | "__delitem__" => {
                update_slot!(as_mapping, as_mapping_wrapper);
                // TODO: need to update sequence protocol too
            }
            "__hash__" => {
                update_slot!(hash, hash_wrapper);
            }
            "__call__" => {
                update_slot!(call, call_wrapper);
            }
            "__getattribute__" => {
                update_slot!(getattro, getattro_wrapper);
            }
            "__setattr__" | "__delattr__" => {
                update_slot!(setattro, setattro_wrapper);
            }
            "__eq__" | "__ne__" | "__le__" | "__lt__" | "__ge__" | "__gt__" => {
                update_slot!(richcompare, richcompare_wrapper);
            }
            "__iter__" => {
                update_slot!(iter, iter_wrapper);
            }
            "__next__" => {
                update_slot!(iternext, iternext_wrapper);
            }
            "__get__" => {
                update_slot!(descr_get, descr_get_wrapper);
            }
            "__set__" | "__delete__" => {
                update_slot!(descr_set, descr_set_wrapper);
            }
            "__new__" => {
                update_slot!(new, new_wrapper);
            }
            "__del__" => {
                update_slot!(del, del_wrapper);
            }
            _ => {}
        }
    }
}

#[pyimpl]
pub trait Constructor: PyValue {
    type Args: FromArgs;

    #[inline]
    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let args: Self::Args = args.bind(vm)?;
        Self::py_new(cls, args, vm)
    }

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult;
}

/// For types that cannot be instantiated through Python code.
pub trait Unconstructible: PyValue {}

impl<T> Constructor for T
where
    T: Unconstructible,
{
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error(format!("cannot create {} instances", cls.slot_name())))
    }
}

#[pyimpl]
pub trait Destructor: PyValue {
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

    fn del(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<()>;
}

#[pyimpl]
pub trait Callable: PyValue {
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
    fn call(zelf: &PyObjectView<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult;
}

#[pyimpl]
pub trait GetDescriptor: PyValue {
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
    fn _cls_is<T>(cls: &Option<PyObjectRef>, other: &T) -> bool
    where
        T: IdProtocol,
    {
        cls.as_ref().map_or(false, |cls| other.is(cls))
    }
}

#[pyimpl]
pub trait Hashable: PyValue {
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

    fn hash(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
}

pub trait Unhashable: PyValue {}

impl<T> Hashable for T
where
    T: Unhashable,
{
    fn slot_hash(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
        Err(vm.new_type_error(format!("unhashable type: '{}'", zelf.class().name())))
    }

    #[cold]
    fn hash(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
        unreachable!("slot_hash is implemented for unhashable types");
    }
}

#[pyimpl]
pub trait Comparable: PyValue {
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
            Err(vm.new_type_error(format!("unexpected payload for {}", op.method_name())))
        }
    }

    fn cmp(
        zelf: &PyObjectView<Self>,
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
    #[inline]
    pub fn identical_optimization(self, a: &impl IdProtocol, b: &impl IdProtocol) -> Option<bool> {
        self.map_eq(|| a.is(b))
    }

    /// Returns `Some(true)` when self is `Eq` and `f()` returns true. Returns `Some(false)` when self
    /// is `Ne` and `f()` returns true. Otherwise returns `None`.
    #[inline]
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
pub trait GetAttr: PyValue {
    #[pyslot]
    fn slot_getattro(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(zelf) = obj.downcast::<Self>() {
            Self::getattro(zelf, name, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __getattribute__".to_owned()))
        }
    }

    // TODO: make zelf: &PyObjectView<Self>
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult;

    #[inline]
    #[pymethod(magic)]
    fn getattribute(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(zelf, name, vm)
    }
}

#[pyimpl]
pub trait SetAttr: PyValue {
    #[pyslot]
    #[inline]
    fn slot_setattro(
        obj: &PyObject,
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
        zelf: &PyObjectView<Self>,
        name: PyStrRef,
        value: Option<PyObjectRef>,
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
        Self::setattro(&zelf, name, Some(value), vm)
    }

    #[inline]
    #[pymethod(magic)]
    fn delattr(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::setattro(&zelf, name, None, vm)
    }
}

#[pyimpl]
pub trait AsBuffer: PyValue {
    // TODO: `flags` parameter
    #[inline]
    #[pyslot]
    fn slot_as_buffer(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer".to_owned()))?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer>;
}

#[pyimpl]
pub trait AsMapping: PyValue {
    #[inline]
    #[pyslot]
    fn slot_as_mapping(zelf: &PyObject, vm: &VirtualMachine) -> PyMappingMethods {
        let zelf = unsafe { zelf.downcast_unchecked_ref::<Self>() };
        Self::as_mapping(zelf, vm)
    }

    #[inline]
    fn downcast(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.downcast::<Self>().map_err(|obj| {
            vm.new_type_error(format!(
                "{} type is required, not {}",
                Self::class(vm),
                obj.class()
            ))
        })
    }

    #[inline]
    fn downcast_ref<'a>(
        zelf: &'a PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<&'a PyObjectView<Self>> {
        zelf.downcast_ref::<Self>().ok_or_else(|| {
            vm.new_type_error(format!(
                "{} type is required, not {}",
                Self::class(vm),
                zelf.class()
            ))
        })
    }

    fn as_mapping(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyMappingMethods;

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

// `Iterator` fits better, but to avoid confusion with rust std::iter::Iterator
#[pyimpl(with(Iterable))]
pub trait IterNext: PyValue + Iterable {
    #[pyslot]
    fn slot_iternext(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if let Some(zelf) = zelf.downcast_ref() {
            Self::next(zelf, vm)
        } else {
            Err(vm.new_type_error("unexpected payload for __next__".to_owned()))
        }
    }

    fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn>;

    #[inline]
    #[pymethod]
    fn __next__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::slot_iternext(&zelf, vm).into_pyresult(vm)
    }
}

pub trait IterNextIterable: PyValue {}

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

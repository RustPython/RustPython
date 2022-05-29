use crate::common::{hash::PyHash, lock::PyRwLock};
use crate::{
    builtins::{PyFloat, PyInt, PyStrInterned, PyStrRef, PyType, PyTypeRef},
    bytecode::ComparisonOperator,
    convert::{ToPyObject, ToPyResult},
    function::Either,
    function::{FromArgs, FuncArgs, OptionalArg, PyComparisonValue},
    identifier,
    protocol::{
        PyBuffer, PyIterReturn, PyMapping, PyMappingMethods, PyNumber, PyNumberMethods, PySequence,
        PySequenceMethods,
    },
    vm::Context,
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};
use std::{borrow::Borrow, cmp::Ordering};

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
    pub as_number: AtomicCell<Option<AsNumberFunc>>,
    pub as_sequence: AtomicCell<Option<AsSequenceFunc>>,
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
    #[non_exhaustive]
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
pub(crate) type AsMappingFunc = fn(&PyObject, &VirtualMachine) -> &'static PyMappingMethods;
pub(crate) type AsNumberFunc = fn(&PyObject, &VirtualMachine) -> Cow<'static, PyNumberMethods>;
pub(crate) type HashFunc = fn(&PyObject, &VirtualMachine) -> PyResult<PyHash>;
// CallFunc = GenericMethod
pub(crate) type GetattroFunc = fn(&PyObject, PyStrRef, &VirtualMachine) -> PyResult;
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
pub(crate) type InitFunc = fn(PyObjectRef, FuncArgs, &VirtualMachine) -> PyResult<()>;
pub(crate) type DelFunc = fn(&PyObject, &VirtualMachine) -> PyResult<()>;
pub(crate) type AsSequenceFunc = fn(&PyObject, &VirtualMachine) -> &'static PySequenceMethods;

fn length_wrapper(obj: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    let ret = vm.call_special_method(obj.to_owned(), identifier!(vm, __len__), ())?;
    let len = ret.payload::<PyInt>().ok_or_else(|| {
        vm.new_type_error(format!(
            "'{}' object cannot be interpreted as an integer",
            ret.class().name()
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

const fn bool_int(v: bool) -> usize {
    if v {
        1
    } else {
        0
    }
}

pub(crate) fn static_as_mapping_generic(
    has_length: bool,
    has_subscript: bool,
    has_ass_subscript: bool,
) -> &'static PyMappingMethods {
    static METHODS: &[PyMappingMethods] = &[
        new_generic(false, false, false),
        new_generic(true, false, false),
        new_generic(false, true, false),
        new_generic(true, true, false),
        new_generic(false, false, true),
        new_generic(true, false, true),
        new_generic(false, true, true),
        new_generic(true, true, true),
    ];

    fn length(mapping: &PyMapping, vm: &VirtualMachine) -> PyResult<usize> {
        length_wrapper(mapping.obj, vm)
    }
    fn subscript(mapping: &PyMapping, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        vm.call_special_method(
            mapping.obj.to_owned(),
            identifier!(vm, __getitem__),
            (needle.to_owned(),),
        )
    }
    fn ass_subscript(
        mapping: &PyMapping,
        needle: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            Some(value) => vm
                .call_special_method(
                    mapping.obj.to_owned(),
                    identifier!(vm, __setitem__),
                    (needle.to_owned(), value),
                )
                .map(|_| Ok(()))?,
            None => vm
                .call_special_method(
                    mapping.obj.to_owned(),
                    identifier!(vm, __delitem__),
                    (needle.to_owned(),),
                )
                .map(|_| Ok(()))?,
        }
    }

    const fn new_generic(
        has_length: bool,
        has_subscript: bool,
        has_ass_subscript: bool,
    ) -> PyMappingMethods {
        PyMappingMethods {
            length: if has_length { Some(length) } else { None },
            subscript: if has_subscript { Some(subscript) } else { None },
            ass_subscript: if has_ass_subscript {
                Some(ass_subscript)
            } else {
                None
            },
        }
    }

    let key =
        bool_int(has_length) | (bool_int(has_subscript) << 1) | (bool_int(has_ass_subscript) << 2);

    &METHODS[key]
}

fn as_mapping_generic(zelf: &PyObject, vm: &VirtualMachine) -> &'static PyMappingMethods {
    let (has_length, has_subscript, has_ass_subscript) = (
        zelf.class().has_attr(identifier!(vm, __len__)),
        zelf.class().has_attr(identifier!(vm, __getitem__)),
        zelf.class().has_attr(identifier!(vm, __setitem__))
            | zelf.class().has_attr(identifier!(vm, __delitem__)),
    );
    static_as_mapping_generic(has_length, has_subscript, has_ass_subscript)
}

pub(crate) fn static_as_sequence_generic(
    has_length: bool,
    has_ass_item: bool,
) -> &'static PySequenceMethods {
    static METHODS: &[PySequenceMethods] = &[
        new_generic(false, false),
        new_generic(true, false),
        new_generic(false, true),
        new_generic(true, true),
    ];

    fn length(seq: &PySequence, vm: &VirtualMachine) -> PyResult<usize> {
        length_wrapper(seq.obj, vm)
    }
    fn item(seq: &PySequence, i: isize, vm: &VirtualMachine) -> PyResult {
        vm.call_special_method(seq.obj.to_owned(), identifier!(vm, __getitem__), (i,))
    }
    fn ass_item(
        seq: &PySequence,
        i: isize,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            Some(value) => vm
                .call_special_method(
                    seq.obj.to_owned(),
                    identifier!(vm, __setitem__),
                    (i.to_pyobject(vm), value),
                )
                .map(|_| Ok(()))?,
            None => vm
                .call_special_method(
                    seq.obj.to_owned(),
                    identifier!(vm, __delitem__),
                    (i.to_pyobject(vm),),
                )
                .map(|_| Ok(()))?,
        }
    }

    const fn new_generic(has_length: bool, has_ass_item: bool) -> PySequenceMethods {
        PySequenceMethods {
            length: if has_length { Some(length) } else { None },
            item: Some(item),
            ass_item: if has_ass_item { Some(ass_item) } else { None },
            ..PySequenceMethods::NOT_IMPLEMENTED
        }
    }

    let key = bool_int(has_length) | (bool_int(has_ass_item) << 1);

    &METHODS[key]
}

fn as_sequence_generic(zelf: &PyObject, vm: &VirtualMachine) -> &'static PySequenceMethods {
    if !zelf.class().has_attr(identifier!(vm, __getitem__)) {
        return &PySequenceMethods::NOT_IMPLEMENTED;
    }

    let (has_length, has_ass_item) = (
        zelf.class().has_attr(identifier!(vm, __len__)),
        zelf.class().has_attr(identifier!(vm, __setitem__))
            | zelf.class().has_attr(identifier!(vm, __delitem__)),
    );

    static_as_sequence_generic(has_length, has_ass_item)
}

fn as_number_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> Cow<'static, PyNumberMethods> {
    Cow::Owned(PyNumberMethods {
        int: then_some_closure!(
            zelf.class().has_attr(identifier!(vm, __int__)),
            |num, vm| {
                let ret =
                    vm.call_special_method(num.obj.to_owned(), identifier!(vm, __int__), ())?;
                ret.downcast::<PyInt>().map_err(|obj| {
                    vm.new_type_error(format!("__int__ returned non-int (type {})", obj.class()))
                })
            }
        ),
        float: then_some_closure!(
            zelf.class().has_attr(identifier!(vm, __float__)),
            |num, vm| {
                let ret =
                    vm.call_special_method(num.obj.to_owned(), identifier!(vm, __float__), ())?;
                ret.downcast::<PyFloat>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "__float__ returned non-float (type {})",
                        obj.class()
                    ))
                })
            }
        ),
        index: then_some_closure!(
            zelf.class().has_attr(identifier!(vm, __index__)),
            |num, vm| {
                let ret =
                    vm.call_special_method(num.obj.to_owned(), identifier!(vm, __index__), ())?;
                ret.downcast::<PyInt>().map_err(|obj| {
                    vm.new_type_error(format!("__index__ returned non-int (type {})", obj.class()))
                })
            }
        ),
        ..PyNumberMethods::NOT_IMPLEMENTED
    })
}

fn hash_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
    let hash_obj = vm.call_special_method(zelf.to_owned(), identifier!(vm, __hash__), ())?;
    match hash_obj.payload_if_subclass::<PyInt>(vm) {
        Some(py_int) => Ok(rustpython_common::hash::hash_bigint(py_int.as_bigint())),
        None => Err(vm.new_type_error("__hash__ method should return an integer".to_owned())),
    }
}

fn call_wrapper(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf.to_owned(), identifier!(vm, __call__), args)
}

fn getattro_wrapper(zelf: &PyObject, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    vm.call_special_method(zelf.to_owned(), identifier!(vm, __getattribute__), (name,))
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
            vm.call_special_method(zelf, identifier!(vm, __setattr__), (name, value))?;
        }
        None => {
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
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match value {
        Some(val) => vm.call_special_method(zelf, identifier!(vm, __set__), (obj, val)),
        None => vm.call_special_method(zelf, identifier!(vm, __delete__), (obj,)),
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

fn new_wrapper(cls: PyTypeRef, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    let new = cls.get_attr(identifier!(vm, __new__)).unwrap();
    args.prepend_arg(cls.into());
    vm.invoke(&new, args)
}

fn del_wrapper(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
    vm.call_special_method(zelf.to_owned(), identifier!(vm, __del__), ())?;
    Ok(())
}

impl PyType {
    pub(crate) fn update_slot(&self, name: &'static PyStrInterned, add: bool) {
        debug_assert!(name.as_str().starts_with("__"));
        debug_assert!(name.as_str().ends_with("__"));

        macro_rules! update_slot {
            ($name:ident, $func:expr) => {{
                self.slots.$name.store(if add { Some($func) } else { None });
            }};
        }
        match name.as_str() {
            "__len__" | "__getitem__" | "__setitem__" | "__delitem__" => {
                update_slot!(as_mapping, as_mapping_generic);
                update_slot!(as_sequence, as_sequence_generic);
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
            "__init__" => {
                update_slot!(init, init_wrapper);
            }
            "__new__" => {
                update_slot!(new, new_wrapper);
            }
            "__del__" => {
                update_slot!(del, del_wrapper);
            }
            "__int__" | "__index__" | "__float__" => {
                update_slot!(as_number, as_number_wrapper);
            }
            _ => {}
        }
    }
}

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
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

pub trait Unhashable: PyPayload {}

impl<T> Hashable for T
where
    T: Unhashable,
{
    fn slot_hash(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyHash> {
        Err(vm.new_type_error(format!("unhashable type: '{}'", zelf.class().name())))
    }

    #[cold]
    fn hash(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
        unreachable!("slot_hash is implemented for unhashable types");
    }
}

#[pyimpl]
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

#[pyimpl]
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

#[pyimpl]
pub trait SetAttr: PyPayload {
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
        zelf: &Py<Self>,
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

#[pyimpl]
pub trait AsMapping: PyPayload {
    const AS_MAPPING: PyMappingMethods;

    #[inline]
    #[pyslot]
    fn as_mapping(_zelf: &PyObject, _vm: &VirtualMachine) -> &'static PyMappingMethods {
        &Self::AS_MAPPING
    }

    #[inline]
    fn mapping_downcast<'a>(mapping: &'a PyMapping) -> &'a Py<Self> {
        unsafe { mapping.obj.downcast_unchecked_ref() }
    }
}

#[pyimpl]
pub trait AsSequence: PyPayload {
    const AS_SEQUENCE: PySequenceMethods;

    #[inline]
    #[pyslot]
    fn as_sequence(_zelf: &PyObject, _vm: &VirtualMachine) -> &'static PySequenceMethods {
        &Self::AS_SEQUENCE
    }

    fn sequence_downcast<'a>(seq: &'a PySequence) -> &'a Py<Self> {
        unsafe { seq.obj.downcast_unchecked_ref() }
    }
}

#[pyimpl]
pub trait AsNumber: PyPayload {
    const AS_NUMBER: PyNumberMethods;

    #[inline]
    #[pyslot]
    fn as_number(_zelf: &PyObject, _vm: &VirtualMachine) -> Cow<'static, PyNumberMethods> {
        Cow::Borrowed(&Self::AS_NUMBER)
    }

    fn number_downcast<'a>(number: &'a PyNumber) -> &'a Py<Self> {
        unsafe { number.obj.downcast_unchecked_ref() }
    }
}

#[pyimpl]
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
#[pyimpl(with(Iterable))]
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

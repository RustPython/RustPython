use super::{
    PyAnyDictRef, PyDict, PyGenericAlias, PyStrRef, PyTupleRef, PyType, PyTypeRef,
    dict::{
        DictIter, PyDictItems, PyDictKeyIterator, PyDictKeys, PyDictReverseKeyIterator,
        PyDictValues,
    },
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine, atomic_func,
    class::PyClassImpl,
    common::{
        hash::{self, PyHash},
        lock::LazyLock,
        wtf8::Wtf8Buf,
    },
    dict_inner::DictKey,
    function::{FuncArgs, KwArgs, OptionalArg, PyComparisonValue},
    object::{Traverse, TraverseFn},
    protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    recursion::ReprGuard,
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, Hashable, Iterable,
        PyComparisonOp, Representable,
    },
};
use rustpython_common::atomic::{Ordering, PyAtomic, Radium};

#[doc = r"frozendict() -> new empty immutable dictionary
frozendict(mapping) -> new immutable dictionary initialized from a mapping
    object's (key, value) pairs
frozendict(iterable) -> new immutable dictionary initialized as if via:
    d = {}
    for k, v in iterable:
        d[k] = v
    d = frozendict(d)
frozendict(**kwargs) -> new immutable dictionary initialized with the name=value
    pairs in the keyword argument list.  For example:  frozendict(one=1, two=2)"]
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

    pub fn from_object(input: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if let Some(input) = &input
            && let Some(frozen) = input.downcast_ref_if_exact::<Self>(vm)
        {
            return Ok(frozen.to_owned());
        }
        let dict = PyDict::default();
        if let Some(input) = input {
            Self::merge_into(&dict, input, vm)?;
        }
        Ok(Self::from_dict(dict).into_ref(&vm.ctx))
    }

    /// Hash a dictionary key, adding the frozendict context to an exact TypeError.
    pub fn key_hash<K: DictKey + ?Sized>(key: &K, vm: &VirtualMachine) -> PyResult<PyHash> {
        super::dict::key_hash(key, "frozendict", vm)
    }

    pub fn inner_getitem_opt<K: DictKey + ?Sized>(
        &self,
        key: &K,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let hash = Self::key_hash(key, vm)?;
        self.dict.entries.get(vm, key, hash)
    }

    fn insert(
        dict: &PyDict,
        key: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let hash = Self::key_hash(key, vm)?;
        dict.entries.insert(vm, key, hash, value)
    }

    // This table is unpublished until construction/union/fromkeys finishes. Python callbacks
    // cannot find and mutate a partially initialized frozendict through the GC.
    pub(crate) fn merge_into(
        dict: &PyDict,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(other_dict) = PyAnyDictRef::from_object(&other)
            && other_dict.uses_builtin_iter(vm)
        {
            return dict.merge_dict(other_dict.as_dict(), true, vm);
        }
        match other.get_attr(vm.ctx.intern_str("keys"), vm) {
            Ok(keys) => {
                let keys = PyIter::try_from_object(vm, keys.call((), vm)?)?;
                while let PyIterReturn::Return(key) = keys.next(vm)? {
                    let value = other.get_item(&*key, vm)?;
                    Self::insert(dict, &key, value, vm)?;
                }
            }
            Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                for (index, pair) in other.get_iter(vm)?.iter::<PyObjectRef>(vm)?.enumerate() {
                    let (key, value) = PyDict::update_sequence_pair(pair?, index, vm)?;
                    Self::insert(dict, &key, value, vm)?;
                }
            }
            Err(exc) => return Err(exc),
        }
        Ok(())
    }

    pub(crate) fn union(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
        let (Some(left), Some(right)) =
            (PyAnyDictRef::from_object(a), PyAnyDictRef::from_object(b))
        else {
            return Ok(vm.ctx.not_implemented());
        };
        if a.class().is(vm.ctx.types.frozendict_type) {
            if left.is_empty() && b.class().is(vm.ctx.types.frozendict_type) {
                return Ok(b.to_owned());
            }
            if right.is_empty()
                && (b.class().is(vm.ctx.types.dict_type)
                    || b.class().is(vm.ctx.types.frozendict_type))
            {
                return Ok(a.to_owned());
            }
        }
        let dict = PyDict::default();
        if left.is_frozen() {
            if !left.is_empty() {
                Self::merge_into(&dict, a.to_owned(), vm)?;
            }
            Self::merge_into(&dict, b.to_owned(), vm)?;
            Ok(Self::from_dict(dict).into_pyobject(vm))
        } else {
            if !left.is_empty() {
                dict.merge_object(a.to_owned(), vm)?;
            }
            dict.merge_object(b.to_owned(), vm)?;
            Ok(dict.into_pyobject(vm))
        }
    }
}

impl Constructor for PyFrozenDict {
    type Args = (OptionalArg<PyObjectRef>, KwArgs);
    fn py_new(
        _cls: &Py<PyType>,
        (input, kwargs): Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let dict = PyDict::default();
        if let OptionalArg::Present(input) = input {
            Self::merge_into(&dict, input, vm)?;
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
        key: PyObjectRef,
        default: OptionalArg<PyObjectRef>,
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
        iterable: PyObjectRef,
        value: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        super::dict::dict_fromkeys(cls, iterable, value.unwrap_or_else(|| vm.ctx.none()), vm)
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
                PyFrozenDict::merge_into(&dict, self.to_owned().into(), vm)?;
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
            or: Some(PyFrozenDict::union),
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
        op.eq_only(|| match PyAnyDictRef::from_object(other) {
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

fn vectorcall_frozendict(
    zelf: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    if nargs == 1
        && kwnames.is_none_or(<[_]>::is_empty)
        && args[0].class().is(vm.ctx.types.frozendict_type)
    {
        return Ok(args[0].clone());
    }
    PyFrozenDict::slot_new(
        zelf.downcast_ref::<PyType>().unwrap().to_owned(),
        FuncArgs::from_vectorcall_owned(args, nargs, kwnames),
        vm,
    )
}

pub(crate) fn init(ctx: &'static Context) {
    PyFrozenDict::extend_class(ctx, ctx.types.frozendict_type);
    ctx.types
        .frozendict_type
        .slots
        .vectorcall
        .store(Some(vectorcall_frozendict));
}

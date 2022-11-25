use once_cell::sync::Lazy;

/*
 * Builtin set type with a sequence of unique items.
 */
use super::{
    builtins_iter, IterStatus, PositionIterInternal, PyDict, PyDictRef, PyGenericAlias, PyTupleRef,
    PyType, PyTypeRef,
};
use crate::atomic_func;
use crate::common::{ascii, hash::PyHash, lock::PyMutex, rc::PyRc};
use crate::{
    class::PyClassImpl,
    dictdatatype::{self, DictSize},
    function::{ArgIterable, FuncArgs, OptionalArg, PosArgs, PyArithmeticValue, PyComparisonValue},
    protocol::{PyIterReturn, PySequenceMethods},
    recursion::ReprGuard,
    types::{
        AsSequence, Comparable, Constructor, Hashable, Initializer, IterNext, IterNextIterable,
        Iterable, PyComparisonOp, Unconstructible, Unhashable,
    },
    utils::collection_repr,
    vm::VirtualMachine,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
};
use std::{fmt, ops::Deref};

pub type SetContentType = dictdatatype::Dict<()>;

#[pyclass(module = false, name = "set")]
#[derive(Default)]
pub struct PySet {
    pub(super) inner: PySetInner,
}

impl PySet {
    pub fn new_ref(ctx: &Context) -> PyRef<Self> {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the set code itself.
        PyRef::new_ref(Self::default(), ctx.types.set_type.to_owned(), None)
    }

    pub fn elements(&self) -> Vec<PyObjectRef> {
        self.inner.elements()
    }

    fn fold_op(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        op: fn(&PySetInner, ArgIterable, &VirtualMachine) -> PyResult<PySetInner>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.fold_op(others, op, vm)?,
        })
    }

    fn op(
        &self,
        other: AnySet,
        op: fn(&PySetInner, ArgIterable, &VirtualMachine) -> PyResult<PySetInner>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .fold_op(std::iter::once(other.into_iterable(vm)?), op, vm)?,
        })
    }
}

#[pyclass(module = false, name = "frozenset")]
#[derive(Default)]
pub struct PyFrozenSet {
    inner: PySetInner,
}

impl PyFrozenSet {
    // Also used by ssl.rs windows.
    pub fn from_iter(
        vm: &VirtualMachine,
        it: impl IntoIterator<Item = PyObjectRef>,
    ) -> PyResult<Self> {
        let inner = PySetInner::default();
        for elem in it {
            inner.add(elem, vm)?;
        }
        // FIXME: empty set check
        Ok(Self { inner })
    }

    pub fn elements(&self) -> Vec<PyObjectRef> {
        self.inner.elements()
    }

    fn fold_op(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        op: fn(&PySetInner, ArgIterable, &VirtualMachine) -> PyResult<PySetInner>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.fold_op(others, op, vm)?,
        })
    }

    fn op(
        &self,
        other: AnySet,
        op: fn(&PySetInner, ArgIterable, &VirtualMachine) -> PyResult<PySetInner>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: self
                .inner
                .fold_op(std::iter::once(other.into_iterable(vm)?), op, vm)?,
        })
    }
}

impl fmt::Debug for PySet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set")
    }
}

impl fmt::Debug for PyFrozenSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("PyFrozenSet ")?;
        f.debug_set().entries(self.elements().iter()).finish()
    }
}

impl PyPayload for PySet {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.set_type
    }
}

impl PyPayload for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.frozenset_type
    }
}

#[derive(Default, Clone)]
pub(super) struct PySetInner {
    content: PyRc<SetContentType>,
}

impl PySetInner {
    pub(super) fn from_iter<T>(iter: T, vm: &VirtualMachine) -> PyResult<Self>
    where
        T: IntoIterator<Item = PyResult<PyObjectRef>>,
    {
        let set = PySetInner::default();
        for item in iter {
            set.add(item?, vm)?;
        }
        Ok(set)
    }

    fn fold_op<O>(
        &self,
        others: impl std::iter::Iterator<Item = O>,
        op: fn(&Self, O, &VirtualMachine) -> PyResult<Self>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let mut res = self.copy();
        for other in others {
            res = op(&res, other, vm)?;
        }
        Ok(res)
    }

    fn len(&self) -> usize {
        self.content.len()
    }

    fn sizeof(&self) -> usize {
        self.content.sizeof()
    }

    fn copy(&self) -> PySetInner {
        PySetInner {
            content: PyRc::new((*self.content).clone()),
        }
    }

    fn contains(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        self.retry_op_with_frozenset(needle, vm, |needle, vm| self.content.contains(vm, needle))
    }

    fn compare(
        &self,
        other: &PySetInner,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if op == PyComparisonOp::Ne {
            return self.compare(other, PyComparisonOp::Eq, vm).map(|eq| !eq);
        }
        if !op.eval_ord(self.len().cmp(&other.len())) {
            return Ok(false);
        }
        let (superset, subset) = if matches!(op, PyComparisonOp::Lt | PyComparisonOp::Le) {
            (other, self)
        } else {
            (self, other)
        };
        for key in subset.elements() {
            if !superset.contains(&key, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn union(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = self.clone();
        for item in other.iter(vm)? {
            set.add(item?, vm)?;
        }

        Ok(set)
    }

    pub(super) fn intersection(
        &self,
        other: ArgIterable,
        vm: &VirtualMachine,
    ) -> PyResult<PySetInner> {
        let set = PySetInner::default();
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(&obj, vm)? {
                set.add(obj, vm)?;
            }
        }
        Ok(set)
    }

    pub(super) fn difference(
        &self,
        other: ArgIterable,
        vm: &VirtualMachine,
    ) -> PyResult<PySetInner> {
        let set = self.copy();
        for item in other.iter(vm)? {
            set.content.delete_if_exists(vm, &*item?)?;
        }
        Ok(set)
    }

    pub(super) fn symmetric_difference(
        &self,
        other: ArgIterable,
        vm: &VirtualMachine,
    ) -> PyResult<PySetInner> {
        let new_inner = self.clone();

        // We want to remove duplicates in other
        let other_set = Self::from_iter(other.iter(vm)?, vm)?;

        for item in other_set.elements() {
            new_inner.content.delete_or_insert(vm, &item, ())?
        }

        Ok(new_inner)
    }

    fn issuperset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if !self.contains(&*item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn issubset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        let other_set = PySetInner::from_iter(other.iter(vm)?, vm)?;
        self.compare(&other_set, PyComparisonOp::Le, vm)
    }

    pub(super) fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if self.contains(&*item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn iter(&self) -> PySetIterator {
        PySetIterator {
            size: self.content.size(),
            internal: PyMutex::new(PositionIterInternal::new(self.content.clone(), 0)),
        }
    }

    fn repr(&self, class_name: Option<&str>, vm: &VirtualMachine) -> PyResult<String> {
        collection_repr(class_name, "{", "}", self.elements().iter(), vm)
    }

    fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.content.insert(vm, &*item, ())
    }

    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.retry_op_with_frozenset(&item, vm, |item, vm| self.content.delete(vm, item))
    }

    fn discard(&self, item: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        self.retry_op_with_frozenset(item, vm, |item, vm| self.content.delete_if_exists(vm, item))
    }

    fn clear(&self) {
        self.content.clear()
    }

    fn elements(&self) -> Vec<PyObjectRef> {
        self.content.keys()
    }

    fn pop(&self, vm: &VirtualMachine) -> PyResult {
        // TODO: should be pop_front, but that requires rearranging every index
        if let Some((key, _)) = self.content.pop_back() {
            Ok(key)
        } else {
            let err_msg = vm.ctx.new_str(ascii!("pop from an empty set")).into();
            Err(vm.new_key_error(err_msg))
        }
    }

    fn update(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.add(item?, vm)?;
            }
        }
        Ok(())
    }

    fn update_internal(&self, iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // check AnySet
        if let Ok(any_set) = AnySet::try_from_object(vm, iterable.to_owned()) {
            self.merge_set(any_set, vm)
        // check Dict
        } else if let Ok(dict) = iterable.to_owned().downcast_exact::<PyDict>(vm) {
            self.merge_dict(dict, vm)
        } else {
            // add iterable that is not AnySet or Dict
            for item in iterable.try_into_value::<ArgIterable>(vm)?.iter(vm)? {
                self.add(item?, vm)?;
            }
            Ok(())
        }
    }

    fn merge_set(&self, any_set: AnySet, vm: &VirtualMachine) -> PyResult<()> {
        for item in any_set.as_inner().elements() {
            self.add(item, vm)?;
        }
        Ok(())
    }

    fn merge_dict(&self, dict: PyDictRef, vm: &VirtualMachine) -> PyResult<()> {
        for (key, _value) in dict {
            self.add(key, vm)?;
        }
        Ok(())
    }

    fn intersection_update(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut temp_inner = self.copy();
        self.clear();
        for iterable in others {
            for item in iterable.iter(vm)? {
                let obj = item?;
                if temp_inner.contains(&obj, vm)? {
                    self.add(obj, vm)?;
                }
            }
            temp_inner = self.copy()
        }
        Ok(())
    }

    fn difference_update(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.content.delete_if_exists(vm, &*item?)?;
            }
        }
        Ok(())
    }

    fn symmetric_difference_update(
        &self,
        others: impl std::iter::Iterator<Item = ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for iterable in others {
            // We want to remove duplicates in iterable
            let iterable_set = Self::from_iter(iterable.iter(vm)?, vm)?;
            for item in iterable_set.elements() {
                self.content.delete_or_insert(vm, &item, ())?;
            }
        }
        Ok(())
    }

    fn hash(&self, vm: &VirtualMachine) -> PyResult<PyHash> {
        crate::utils::hash_iter_unordered(self.elements().iter(), vm)
    }

    // Run operation, on failure, if item is a set/set subclass, convert it
    // into a frozenset and try the operation again. Propagates original error
    // on failure to convert and restores item in KeyError on failure (remove).
    fn retry_op_with_frozenset<T, F>(
        &self,
        item: &PyObject,
        vm: &VirtualMachine,
        op: F,
    ) -> PyResult<T>
    where
        F: Fn(&PyObject, &VirtualMachine) -> PyResult<T>,
    {
        op(item, vm).or_else(|original_err| {
            item.payload_if_subclass::<PySet>(vm)
                // Keep original error around.
                .ok_or(original_err)
                .and_then(|set| {
                    op(
                        &PyFrozenSet {
                            inner: set.inner.copy(),
                        }
                        .into_pyobject(vm),
                        vm,
                    )
                    // If operation raised KeyError, report original set (set.remove)
                    .map_err(|op_err| {
                        if op_err.fast_isinstance(vm.ctx.exceptions.key_error) {
                            vm.new_key_error(item.to_owned())
                        } else {
                            op_err
                        }
                    })
                })
        })
    }
}

fn extract_set(obj: &PyObject) -> Option<&PySetInner> {
    match_class!(match obj {
        ref set @ PySet => Some(&set.inner),
        ref frozen @ PyFrozenSet => Some(&frozen.inner),
        _ => None,
    })
}

fn reduce_set(
    zelf: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(PyTypeRef, PyTupleRef, Option<PyDictRef>)> {
    Ok((
        zelf.class().to_owned(),
        vm.new_tuple((extract_set(zelf)
            .unwrap_or(&PySetInner::default())
            .elements(),)),
        zelf.dict(),
    ))
}

#[pyclass(
    with(Constructor, Initializer, AsSequence, Hashable, Comparable, Iterable),
    flags(BASETYPE)
)]
impl PySet {
    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner.sizeof()
    }

    #[pymethod]
    fn copy(&self) -> Self {
        Self {
            inner: self.inner.copy(),
        }
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.contains(&needle, vm)
    }

    #[pymethod]
    fn union(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::union, vm)
    }

    #[pymethod]
    fn intersection(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::intersection, vm)
    }

    #[pymethod]
    fn difference(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::difference, vm)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::symmetric_difference, vm)
    }

    #[pymethod]
    fn issubset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::union,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::intersection,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::difference,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::symmetric_difference,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = zelf.class();
        let borrowed_name = class.name();
        let class_name = borrowed_name.deref();
        let s = if zelf.inner.len() == 0 {
            format!("{}()", class_name)
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let name = if class_name != "set" {
                Some(class_name)
            } else {
                None
            };
            zelf.inner.repr(name, vm)?
        } else {
            format!("{}(...)", class_name)
        };
        Ok(s)
    }

    #[pymethod]
    pub fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.add(item, vm)?;
        Ok(())
    }

    #[pymethod]
    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.remove(item, vm)
    }

    #[pymethod]
    fn discard(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.discard(&item, vm)?;
        Ok(())
    }

    #[pymethod]
    fn clear(&self) {
        self.inner.clear()
    }

    #[pymethod]
    fn pop(&self, vm: &VirtualMachine) -> PyResult {
        self.inner.pop(vm)
    }

    #[pymethod(magic)]
    fn ior(zelf: PyRef<Self>, set: AnySet, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.inner.update(set.into_iterable_iter(vm)?, vm)?;
        Ok(zelf)
    }

    #[pymethod]
    fn update(&self, others: PosArgs<PyObjectRef>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            self.inner.update_internal(iterable, vm)?;
        }
        Ok(())
    }

    #[pymethod]
    fn intersection_update(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner.intersection_update(others.into_iter(), vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn iand(zelf: PyRef<Self>, set: AnySet, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.inner
            .intersection_update(std::iter::once(set.into_iterable(vm)?), vm)?;
        Ok(zelf)
    }

    #[pymethod]
    fn difference_update(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.difference_update(others.into_iter(), vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn isub(zelf: PyRef<Self>, set: AnySet, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.inner
            .difference_update(set.into_iterable_iter(vm)?, vm)?;
        Ok(zelf)
    }

    #[pymethod]
    fn symmetric_difference_update(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner
            .symmetric_difference_update(others.into_iter(), vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn ixor(zelf: PyRef<Self>, set: AnySet, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.inner
            .symmetric_difference_update(set.into_iterable_iter(vm)?, vm)?;
        Ok(zelf)
    }

    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyTypeRef, PyTupleRef, Option<PyDictRef>)> {
        reduce_set(zelf.as_ref(), vm)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl Constructor for PySet {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        PySet::default().into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl Initializer for PySet {
    type Args = OptionalArg<PyObjectRef>;

    fn init(zelf: PyRef<Self>, iterable: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        if zelf.len() > 0 {
            zelf.clear();
        }
        if let OptionalArg::Present(it) = iterable {
            zelf.update(PosArgs::new(vec![it]), vm)?;
        }
        Ok(())
    }
}

impl AsSequence for PySet {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PySet::sequence_downcast(seq).len())),
            contains: atomic_func!(|seq, needle, vm| PySet::sequence_downcast(seq)
                .inner
                .contains(needle, vm)),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Comparable for PySet {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        extract_set(other).map_or(Ok(PyComparisonValue::NotImplemented), |other| {
            Ok(zelf.inner.compare(other, op, vm)?.into())
        })
    }
}

impl Unhashable for PySet {}

impl Iterable for PySet {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(zelf.inner.iter().into_pyobject(vm))
    }
}

impl Constructor for PyFrozenSet {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(vm.ctx.types.frozenset_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(fs) => return Ok(fs.into()),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            iterable.try_to_value(vm)?
        } else {
            vec![]
        };

        // Return empty fs if iterable passed is empty and only for exact fs types.
        if elements.is_empty() && cls.is(vm.ctx.types.frozenset_type) {
            Ok(vm.ctx.empty_frozenset.clone().into())
        } else {
            Self::from_iter(vm, elements)
                .and_then(|o| o.into_ref_with_type(vm, cls).map(Into::into))
        }
    }
}

#[pyclass(
    flags(BASETYPE),
    with(Constructor, AsSequence, Hashable, Comparable, Iterable)
)]
impl PyFrozenSet {
    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner.sizeof()
    }

    #[pymethod]
    fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        if zelf.class().is(vm.ctx.types.frozenset_type) {
            zelf
        } else {
            Self {
                inner: zelf.inner.copy(),
            }
            .into_ref(vm)
        }
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.contains(&needle, vm)
    }

    #[pymethod]
    fn union(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::union, vm)
    }

    #[pymethod]
    fn intersection(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::intersection, vm)
    }

    #[pymethod]
    fn difference(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::difference, vm)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        self.fold_op(others.into_iter(), PySetInner::symmetric_difference, vm)
    }

    #[pymethod]
    fn issubset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(set) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                set,
                PySetInner::union,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::intersection,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::difference,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(other) = AnySet::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(self.op(
                other,
                PySetInner::symmetric_difference,
                vm,
            )?))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let inner = &zelf.inner;
        let class = zelf.class();
        let class_name = class.name();
        let s = if inner.len() == 0 {
            format!("{}()", class_name)
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            inner.repr(Some(&class_name), vm)?
        } else {
            format!("{}(...)", class_name)
        };
        Ok(s)
    }

    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyTypeRef, PyTupleRef, Option<PyDictRef>)> {
        reduce_set(zelf.as_ref(), vm)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl AsSequence for PyFrozenSet {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyFrozenSet::sequence_downcast(seq).len())),
            contains: atomic_func!(|seq, needle, vm| PyFrozenSet::sequence_downcast(seq)
                .inner
                .contains(needle, vm)),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Hashable for PyFrozenSet {
    #[inline]
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.inner.hash(vm)
    }
}

impl Comparable for PyFrozenSet {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        extract_set(other).map_or(Ok(PyComparisonValue::NotImplemented), |other| {
            Ok(zelf.inner.compare(other, op, vm)?.into())
        })
    }
}

impl Iterable for PyFrozenSet {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(zelf.inner.iter().into_pyobject(vm))
    }
}

struct AnySet {
    object: PyObjectRef,
}

impl AnySet {
    fn into_iterable(self, vm: &VirtualMachine) -> PyResult<ArgIterable> {
        self.object.try_into_value(vm)
    }

    fn into_iterable_iter(
        self,
        vm: &VirtualMachine,
    ) -> PyResult<impl std::iter::Iterator<Item = ArgIterable>> {
        Ok(std::iter::once(self.into_iterable(vm)?))
    }

    fn as_inner(&self) -> &PySetInner {
        match_class!(match self.object.as_object() {
            ref set @ PySet => &set.inner,
            ref frozen @ PyFrozenSet => &frozen.inner,
            _ => unreachable!("AnySet is always PySet or PyFrozenSet"), // should not be called.
        })
    }
}

impl TryFromObject for AnySet {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = obj.class();
        if class.fast_issubclass(vm.ctx.types.set_type)
            || class.fast_issubclass(vm.ctx.types.frozenset_type)
        {
            Ok(AnySet { object: obj })
        } else {
            Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", class)))
        }
    }
}

#[pyclass(module = false, name = "set_iterator")]
pub(crate) struct PySetIterator {
    size: DictSize,
    internal: PyMutex<PositionIterInternal<PyRc<SetContentType>>>,
}

impl fmt::Debug for PySetIterator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set_iterator")
    }
}

impl PyPayload for PySetIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.set_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PySetIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().length_hint(|_| self.size.entries_size)
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<(PyObjectRef, (PyObjectRef,))> {
        let internal = zelf.internal.lock();
        Ok((
            builtins_iter(vm).to_owned(),
            (vm.ctx
                .new_list(match &internal.status {
                    IterStatus::Exhausted => vec![],
                    IterStatus::Active(dict) => {
                        dict.keys().into_iter().skip(internal.position).collect()
                    }
                })
                .into(),),
        ))
    }
}
impl Unconstructible for PySetIterator {}

impl IterNextIterable for PySetIterator {}
impl IterNext for PySetIterator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut internal = zelf.internal.lock();
        let next = if let IterStatus::Active(dict) = &internal.status {
            if dict.has_changed_size(&zelf.size) {
                internal.status = IterStatus::Exhausted;
                return Err(vm.new_runtime_error("set changed size during iteration".to_owned()));
            }
            match dict.next_entry(internal.position) {
                Some((position, key, _)) => {
                    internal.position = position;
                    PyIterReturn::Return(key)
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

pub fn init(context: &Context) {
    PySet::extend_class(context, context.types.set_type);
    PyFrozenSet::extend_class(context, context.types.frozenset_type);
    PySetIterator::extend_class(context, context.types.set_iterator_type);
}

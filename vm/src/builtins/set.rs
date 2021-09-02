/*
 * Builtin set type with a sequence of unique items.
 */
use super::{pytype::PyTypeRef, IterStatus, PyDictRef};
use crate::common::hash::PyHash;
use crate::common::rc::PyRc;
use crate::dictdatatype;
use crate::dictdatatype::DictSize;
use crate::function::{Args, FuncArgs, OptionalArg};
use crate::slots::{
    Comparable, Hashable, Iterable, PyComparisonOp, PyIter, SlotConstructor, Unhashable,
};
use crate::vm::{ReprGuard, VirtualMachine};
use crate::{
    IdProtocol, PyClassImpl, PyComparisonValue, PyContext, PyIterable, PyObjectRef, PyRef,
    PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crossbeam_utils::atomic::AtomicCell;
use std::fmt;

pub type SetContentType = dictdatatype::Dict<()>;

/// set() -> new empty set object
/// set(iterable) -> new set object
///
/// Build an unordered collection of unique elements.
#[pyclass(module = false, name = "set")]
#[derive(Default)]
pub struct PySet {
    inner: PySetInner,
}
pub type PySetRef = PyRef<PySet>;

/// frozenset() -> empty frozenset object
/// frozenset(iterable) -> frozenset object
///
/// Build an immutable unordered collection of unique elements.
#[pyclass(module = false, name = "frozenset")]
#[derive(Default)]
pub struct PyFrozenSet {
    inner: PySetInner,
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
        f.write_str("frozenset")
    }
}

impl PyValue for PySet {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.set_type
    }
}

impl PyValue for PyFrozenSet {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.frozenset_type
    }
}

#[derive(Default, Clone)]
struct PySetInner {
    content: PyRc<SetContentType>,
}

impl PySetInner {
    fn new(iterable: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = PySetInner::default();
        for item in iterable.iter(vm)? {
            set.add(item?, vm)?;
        }
        Ok(set)
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

    fn contains(&self, needle: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
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

    fn union(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = self.clone();
        for item in other.iter(vm)? {
            set.add(item?, vm)?;
        }

        Ok(set)
    }

    fn intersection(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = PySetInner::default();
        for item in other.iter(vm)? {
            let obj = item?;
            if self.contains(&obj, vm)? {
                set.add(obj, vm)?;
            }
        }
        Ok(set)
    }

    fn difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let set = self.copy();
        for item in other.iter(vm)? {
            set.content.delete_if_exists(vm, &item?)?;
        }
        Ok(set)
    }

    fn symmetric_difference(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<PySetInner> {
        let new_inner = self.clone();

        // We want to remove duplicates in other
        let other_set = Self::new(other, vm)?;

        for item in other_set.elements() {
            new_inner.content.delete_or_insert(vm, &item, ())?
        }

        Ok(new_inner)
    }

    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if !self.contains(&item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        let other_set = PySetInner::new(other, vm)?;
        self.compare(&other_set, PyComparisonOp::Le, vm)
    }

    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        for item in other.iter(vm)? {
            if self.contains(&item?, vm)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn iter(&self) -> PySetIterator {
        PySetIterator {
            dict: PyRc::clone(&self.content),
            size: self.content.size(),
            position: AtomicCell::new(0),
            status: AtomicCell::new(IterStatus::Active),
        }
    }

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let mut str_parts = Vec::with_capacity(self.content.len());
        for key in self.elements() {
            let part = vm.to_repr(&key)?;
            str_parts.push(part.as_str().to_owned());
        }

        Ok(format!("{{{}}}", str_parts.join(", ")))
    }

    fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.content.insert(vm, item, ())
    }

    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.retry_op_with_frozenset(&item, vm, |item, vm| self.content.delete(vm, item.clone()))
    }

    fn discard(&self, item: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
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
            let err_msg = vm.ctx.new_str("pop from an empty set");
            Err(vm.new_key_error(err_msg))
        }
    }

    fn update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.add(item?, vm)?;
            }
        }
        Ok(())
    }

    fn intersection_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
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

    fn difference_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.content.delete_if_exists(vm, &item?)?;
            }
        }
        Ok(())
    }

    fn symmetric_difference_update(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for iterable in others {
            // We want to remove duplicates in iterable
            let iterable_set = Self::new(iterable, vm)?;
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
        item: &PyObjectRef,
        vm: &VirtualMachine,
        op: F,
    ) -> PyResult<T>
    where
        F: Fn(&PyObjectRef, &VirtualMachine) -> PyResult<T>,
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
                        .into_object(vm),
                        vm,
                    )
                    // If operation raised KeyError, report original set (set.remove)
                    .map_err(|op_err| {
                        if op_err.isinstance(&vm.ctx.exceptions.key_error) {
                            vm.new_key_error(item.clone())
                        } else {
                            op_err
                        }
                    })
                })
        })
    }
}

fn extract_set(obj: &PyObjectRef) -> Option<&PySetInner> {
    match_class!(match obj {
        ref set @ PySet => Some(&set.inner),
        ref frozen @ PyFrozenSet => Some(&frozen.inner),
        _ => None,
    })
}

fn reduce_set(
    zelf: &PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyTypeRef, PyObjectRef, Option<PyDictRef>)> {
    Ok((
        zelf.clone_class(),
        vm.ctx.new_tuple(vec![vm.ctx.new_list(
            extract_set(zelf)
                .unwrap_or(&PySetInner::default())
                .elements(),
        )]),
        zelf.dict(),
    ))
}

macro_rules! multi_args_set {
    ($vm:expr, $others:expr, $zelf:expr, $op:tt) => {{
        let mut res = $zelf.inner.copy();
        for other in $others {
            res = res.$op(other, $vm)?
        }
        Ok(Self { inner: res })
    }};
}

#[pyimpl(with(Hashable, Comparable, Iterable), flags(BASETYPE))]
impl PySet {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PySet::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(&self, iterable: OptionalArg<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        if self.len() > 0 {
            self.clear();
        }
        if let OptionalArg::Present(it) = iterable {
            self.update(Args::new(vec![it]), vm)?;
        }
        Ok(())
    }

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
    fn union(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_set!(vm, others, self, symmetric_difference)
    }

    #[pymethod]
    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.union(other.iterable, vm)
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.intersection(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn sub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.difference(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn rsub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.symmetric_difference(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let s = if zelf.inner.len() == 0 {
            "set()".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            zelf.inner.repr(vm)?
        } else {
            "set(...)".to_owned()
        };
        Ok(vm.ctx.new_str(s))
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
    fn ior(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.update(others, vm)?;
        Ok(())
    }

    #[pymethod]
    fn intersection_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.intersection_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn iand(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.intersection_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn difference_update(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn isub(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod]
    fn symmetric_difference_update(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner.symmetric_difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn ixor(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner
            .symmetric_difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().clone())
    }

    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyTypeRef, PyObjectRef, Option<PyDictRef>)> {
        reduce_set(&zelf.into_object(), vm)
    }
}

impl Comparable for PySet {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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
        Ok(zelf.inner.iter().into_object(vm))
    }
}

macro_rules! multi_args_frozenset {
    ($vm:expr, $others:expr, $zelf:expr, $op:tt) => {{
        let mut res = $zelf.inner.copy();
        for other in $others {
            res = res.$op(other, $vm)?
        }
        Ok(Self { inner: res })
    }};
}

impl SlotConstructor for PyFrozenSet {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(&vm.ctx.types.frozenset_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(fs) => return Ok(fs.into_object()),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        };

        // Return empty fs if iterable passed is empty and only for exact fs types.
        if elements.is_empty() && cls.is(&vm.ctx.types.frozenset_type) {
            Ok(vm.ctx.empty_frozenset.clone().into_object())
        } else {
            Self::from_iter(vm, elements).and_then(|o| o.into_pyresult_with_type(vm, cls))
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable, SlotConstructor))]
impl PyFrozenSet {
    // Also used by ssl.rs windows.
    pub(crate) fn from_iter(
        vm: &VirtualMachine,
        it: impl IntoIterator<Item = PyObjectRef>,
    ) -> PyResult<Self> {
        let inner = PySetInner::default();
        for elem in it {
            inner.add(elem, vm)?;
        }
        Ok(Self { inner })
    }

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
        if zelf.class().is(&vm.ctx.types.frozenset_type) {
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
    fn union(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: Args<PyIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: Args<PyIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, symmetric_difference)
    }

    #[pymethod]
    fn issubset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issubset(other, vm)
    }

    #[pymethod]
    fn issuperset(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.issuperset(other, vm)
    }

    #[pymethod]
    fn isdisjoint(&self, other: PyIterable, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.isdisjoint(other, vm)
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.union(other.iterable, vm)
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.intersection(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn sub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.difference(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn rsub(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.sub(other, vm)
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(&self, other: SetIterable, vm: &VirtualMachine) -> PyResult<Self> {
        self.symmetric_difference(other.iterable, vm)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let inner = &zelf.inner;
        let s = if inner.len() == 0 {
            "frozenset()".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            format!("frozenset({})", inner.repr(vm)?)
        } else {
            "frozenset(...)".to_owned()
        };
        Ok(vm.ctx.new_str(s))
    }

    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyTypeRef, PyObjectRef, Option<PyDictRef>)> {
        reduce_set(&zelf.into_object(), vm)
    }
}

impl Hashable for PyFrozenSet {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.inner.hash(vm)
    }
}

impl Comparable for PyFrozenSet {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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
        Ok(zelf.inner.iter().into_object(vm))
    }
}

struct SetIterable {
    iterable: Args<PyIterable>,
}

impl TryFromObject for SetIterable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = obj.class();
        if class.issubclass(&vm.ctx.types.set_type)
            || class.issubclass(&vm.ctx.types.frozenset_type)
        {
            // the class lease needs to be drop to be able to return the object
            drop(class);
            Ok(SetIterable {
                iterable: Args::new(vec![PyIterable::try_from_object(vm, obj)?]),
            })
        } else {
            Err(vm.new_type_error(format!("{} is not a subtype of set or frozenset", class)))
        }
    }
}

#[pyclass(module = false, name = "set_iterator")]
pub(crate) struct PySetIterator {
    dict: PyRc<SetContentType>,
    size: DictSize,
    position: AtomicCell<usize>,
    status: AtomicCell<IterStatus>,
}

impl fmt::Debug for PySetIterator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("set_iterator")
    }
}

impl PyValue for PySetIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.set_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PySetIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        if let IterStatus::Exhausted = self.status.load() {
            0
        } else {
            self.dict.len_from_entry_index(self.position.load())
        }
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<(PyObjectRef, (PyObjectRef,))> {
        Ok((
            vm.get_attribute(vm.builtins.clone(), "iter")?,
            (vm.ctx.new_list(match zelf.status.load() {
                IterStatus::Exhausted => vec![],
                IterStatus::Active => zelf
                    .dict
                    .keys()
                    .into_iter()
                    .skip(zelf.position.load())
                    .collect(),
            }),),
        ))
    }
}

impl PyIter for PySetIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        match zelf.status.load() {
            IterStatus::Exhausted => Err(vm.new_stop_iteration()),
            IterStatus::Active => {
                if zelf.dict.has_changed_size(&zelf.size) {
                    zelf.status.store(IterStatus::Exhausted);
                    return Err(
                        vm.new_runtime_error("set changed size during iteration".to_owned())
                    );
                }
                match zelf.dict.next_entry_atomic(&zelf.position) {
                    Some((key, _)) => Ok(key),
                    None => {
                        zelf.status.store(IterStatus::Exhausted);
                        Err(vm.new_stop_iteration())
                    }
                }
            }
        }
    }
}

pub fn init(context: &PyContext) {
    PySet::extend_class(context, &context.types.set_type);
    PyFrozenSet::extend_class(context, &context.types.frozenset_type);
    PySetIterator::extend_class(context, &context.types.set_iterator_type);
}

/*
 * Builtin set type with a sequence of unique items.
 */
use super::{
    builtins_iter, IterStatus, PositionIterInternal, PyDictRef, PyGenericAlias, PyTupleRef,
    PyTypeRef,
};
use crate::common::{ascii, hash::PyHash, lock::PyMutex, rc::PyRc};
use crate::{
    dictdatatype::{self, DictSize},
    function::{ArgIterable, FuncArgs, OptionalArg, PosArgs},
    protocol::PyIterReturn,
    types::{
        Comparable, Constructor, Hashable, IterNext, IterNextIterable, Iterable, PyComparisonOp,
        Unconstructible, Unhashable,
    },
    vm::{ReprGuard, VirtualMachine},
    IdProtocol, PyArithmeticValue, PyClassImpl, PyComparisonValue, PyContext, PyObject,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use std::{fmt, ops::Deref};

pub type SetContentType = dictdatatype::Dict<()>;

/// set() -> new empty set object
/// set(iterable) -> new set object
///
/// Build an unordered collection of unique elements.
#[pyclass(module = false, name = "set")]
#[derive(Default)]
pub struct PySet {
    pub(super) inner: PySetInner,
}

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
            set.content.delete_if_exists(vm, &item?)?;
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

    fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
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
        let mut repr = String::new();
        if let Some(name) = class_name {
            repr.push_str(name);
            repr.push('(');
        }
        repr.push('{');
        {
            let mut parts_iter = self.elements().into_iter().map(|o| o.repr(vm));
            repr.push_str(
                parts_iter
                    .next()
                    .transpose()?
                    .expect("this is not called for empty set")
                    .as_str(),
            );
            for part in parts_iter {
                repr.push_str(", ");
                repr.push_str(part?.as_str());
            }
        }
        repr.push('}');
        if class_name.is_some() {
            repr.push(')');
        }

        Ok(repr)
    }

    fn add(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.content.insert(vm, item, ())
    }

    fn remove(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.retry_op_with_frozenset(&item, vm, |item, vm| {
            self.content.delete(vm, item.to_owned())
        })
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

    fn update(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.add(item?, vm)?;
            }
        }
        Ok(())
    }

    fn intersection_update(
        &self,
        others: PosArgs<ArgIterable>,
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

    fn difference_update(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        for iterable in others {
            for item in iterable.iter(vm)? {
                self.content.delete_if_exists(vm, &item?)?;
            }
        }
        Ok(())
    }

    fn symmetric_difference_update(
        &self,
        others: PosArgs<ArgIterable>,
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
                        .into_object(vm),
                        vm,
                    )
                    // If operation raised KeyError, report original set (set.remove)
                    .map_err(|op_err| {
                        if op_err.isinstance(&vm.ctx.exceptions.key_error) {
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
        zelf.clone_class(),
        vm.new_tuple((extract_set(zelf)
            .unwrap_or(&PySetInner::default())
            .elements(),)),
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

impl PySet {
    pub fn new_ref(ctx: &PyContext) -> PyRef<Self> {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the set code itself.
        PyRef::new_ref(Self::default(), ctx.types.set_type.clone(), None)
    }
}

#[pyimpl(with(Hashable, Comparable, Iterable), flags(BASETYPE))]
impl PySet {
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PySet::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(&self, iterable: OptionalArg<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        if self.len() > 0 {
            self.clear();
        }
        if let OptionalArg::Present(it) = iterable {
            self.update(PosArgs::new(vec![it]), vm)?;
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
    fn union(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_set!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_set!(vm, others, self, symmetric_difference)
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
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.union(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.intersection(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.difference(set_iter.iterable, vm)?,
            ))
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
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.symmetric_difference(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
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
        Ok(vm.ctx.new_str(s).into())
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
        Ok(zelf.as_object().to_owned())
    }

    #[pymethod]
    fn update(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.update(others, vm)?;
        Ok(())
    }

    #[pymethod]
    fn intersection_update(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner.intersection_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn iand(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.intersection_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().to_owned())
    }

    #[pymethod]
    fn difference_update(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn isub(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner.difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().to_owned())
    }

    #[pymethod]
    fn symmetric_difference_update(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.inner.symmetric_difference_update(others, vm)?;
        Ok(())
    }

    #[pymethod(magic)]
    fn ixor(zelf: PyRef<Self>, iterable: SetIterable, vm: &VirtualMachine) -> PyResult {
        zelf.inner
            .symmetric_difference_update(iterable.iterable, vm)?;
        Ok(zelf.as_object().to_owned())
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

impl Comparable for PySet {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
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

impl Constructor for PyFrozenSet {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(&vm.ctx.types.frozenset_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(fs) => return Ok(fs.into()),
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
            Ok(vm.ctx.empty_frozenset.clone().into())
        } else {
            Self::from_iter(vm, elements).and_then(|o| o.into_pyresult_with_type(vm, cls))
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable, Constructor))]
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
    fn union(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, union)
    }

    #[pymethod]
    fn intersection(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, intersection)
    }

    #[pymethod]
    fn difference(&self, others: PosArgs<ArgIterable>, vm: &VirtualMachine) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, difference)
    }

    #[pymethod]
    fn symmetric_difference(
        &self,
        others: PosArgs<ArgIterable>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        multi_args_frozenset!(vm, others, self, symmetric_difference)
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
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.union(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.intersection(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmeticValue<Self>> {
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.difference(set_iter.iterable, vm)?,
            ))
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
        if let Ok(set_iter) = SetIterable::try_from_object(vm, other) {
            Ok(PyArithmeticValue::Implemented(
                self.symmetric_difference(set_iter.iterable, vm)?,
            ))
        } else {
            Ok(PyArithmeticValue::NotImplemented)
        }
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
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
        Ok(vm.ctx.new_str(s).into())
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

impl Hashable for PyFrozenSet {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.inner.hash(vm)
    }
}

impl Comparable for PyFrozenSet {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
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
        Ok(zelf.inner.iter().into_object(vm))
    }
}

struct SetIterable {
    iterable: PosArgs<ArgIterable>,
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
                iterable: PosArgs::new(vec![ArgIterable::try_from_object(vm, obj)?]),
            })
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

impl PyValue for PySetIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.set_iterator_type
    }
}

#[pyimpl(with(Constructor, IterNext))]
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
    fn next(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

pub fn init(context: &PyContext) {
    PySet::extend_class(context, &context.types.set_type);
    PyFrozenSet::extend_class(context, &context.types.frozenset_type);
    PySetIterator::extend_class(context, &context.types.set_iterator_type);
}

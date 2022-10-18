use super::{PositionIterInternal, PyGenericAlias, PyTupleRef, PyType, PyTypeRef};
use crate::atomic_func;
use crate::common::lock::{
    PyMappedRwLockReadGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
};
use crate::{
    class::PyClassImpl,
    convert::ToPyObject,
    function::{FuncArgs, OptionalArg, PyComparisonValue},
    iter::PyExactSizeIterator,
    protocol::{PyIterReturn, PyMappingMethods, PySequenceMethods},
    recursion::ReprGuard,
    sequence::{MutObjectSequenceOp, OptionalRangeArgs, SequenceExt, SequenceMutExt},
    sliceable::{SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsMapping, AsSequence, Comparable, Constructor, Hashable, Initializer, IterNext,
        IterNextIterable, Iterable, PyComparisonOp, Unconstructible, Unhashable,
    },
    utils::collection_repr,
    vm::VirtualMachine,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
};
use std::{fmt, ops::DerefMut};

#[pyclass(module = false, name = "list")]
#[derive(Default)]
pub struct PyList {
    elements: PyRwLock<Vec<PyObjectRef>>,
}

impl fmt::Debug for PyList {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("list")
    }
}

impl From<Vec<PyObjectRef>> for PyList {
    fn from(elements: Vec<PyObjectRef>) -> Self {
        PyList {
            elements: PyRwLock::new(elements),
        }
    }
}

impl FromIterator<PyObjectRef> for PyList {
    fn from_iter<T: IntoIterator<Item = PyObjectRef>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}

impl PyPayload for PyList {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.list_type
    }
}

impl ToPyObject for Vec<PyObjectRef> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyList::new_ref(self, &vm.ctx).into()
    }
}

impl PyList {
    pub fn new_ref(elements: Vec<PyObjectRef>, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::from(elements), ctx.types.list_type.to_owned(), None)
    }

    pub fn borrow_vec(&self) -> PyMappedRwLockReadGuard<'_, [PyObjectRef]> {
        PyRwLockReadGuard::map(self.elements.read(), |v| &**v)
    }

    pub fn borrow_vec_mut(&self) -> PyRwLockWriteGuard<'_, Vec<PyObjectRef>> {
        self.elements.write()
    }
}

#[derive(FromArgs, Default)]
pub(crate) struct SortOptions {
    #[pyarg(named, default)]
    key: Option<PyObjectRef>,
    #[pyarg(named, default = "false")]
    reverse: bool,
}

pub type PyListRef = PyRef<PyList>;

#[pyclass(
    with(
        Constructor,
        Initializer,
        AsMapping,
        Iterable,
        Hashable,
        Comparable,
        AsSequence
    ),
    flags(BASETYPE)
)]
impl PyList {
    #[pymethod]
    pub(crate) fn append(&self, x: PyObjectRef) {
        self.borrow_vec_mut().push(x);
    }

    #[pymethod]
    pub(crate) fn extend(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut new_elements = x.try_to_value(vm)?;
        self.borrow_vec_mut().append(&mut new_elements);
        Ok(())
    }

    #[pymethod]
    pub(crate) fn insert(&self, position: isize, element: PyObjectRef) {
        let mut elements = self.borrow_vec_mut();
        let position = elements.saturate_index(position);
        elements.insert(position, element);
    }

    fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let other = other.payload_if_subclass::<PyList>(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "Cannot add {} and {}",
                Self::class(vm).name(),
                other.class().name()
            ))
        })?;
        let mut elements = self.borrow_vec().to_vec();
        elements.extend(other.borrow_vec().iter().cloned());
        Ok(Self::new_ref(elements, &vm.ctx))
    }

    #[pymethod(magic)]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.concat(&other, vm)
    }

    fn inplace_concat(
        zelf: &Py<Self>,
        other: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut seq = extract_cloned(other, Ok, vm)?;
        zelf.borrow_vec_mut().append(&mut seq);
        Ok(zelf.to_owned().into())
    }

    #[pymethod(magic)]
    fn iadd(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let mut seq = extract_cloned(&other, Ok, vm)?;
        zelf.borrow_vec_mut().append(&mut seq);
        Ok(zelf)
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.borrow_vec().is_empty()
    }

    #[pymethod]
    fn clear(&self) {
        let _removed = std::mem::take(self.borrow_vec_mut().deref_mut());
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyRef<Self> {
        Self::new_ref(self.borrow_vec().to_vec(), &vm.ctx)
    }

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.borrow_vec().len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.elements.read().capacity() * std::mem::size_of::<PyObjectRef>()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_vec_mut().reverse();
    }

    #[pymethod(magic)]
    fn reversed(zelf: PyRef<Self>) -> PyListReverseIterator {
        let position = zelf.len().saturating_sub(1);
        PyListReverseIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, position)),
        }
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(i) => self.borrow_vec().getitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => self
                .borrow_vec()
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_list(x).into()),
        }
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    fn _setitem(&self, needle: &PyObject, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(index) => self.borrow_vec_mut().setitem_by_index(vm, index, value),
            SequenceIndex::Slice(slice) => {
                let sec = extract_cloned(&value, Ok, vm)?;
                self.borrow_vec_mut().setitem_by_slice(vm, slice, &sec)
            }
        }
    }

    #[pymethod(magic)]
    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self._setitem(&needle, value, vm)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if zelf.len() == 0 {
            "[]".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            collection_repr(None, "[", "]", zelf.borrow_vec().iter(), vm)?
        } else {
            "[...]".to_owned()
        };
        Ok(s)
    }

    #[pymethod(magic)]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let elements = &*self.borrow_vec();
        let v = elements.mul(vm, n)?;
        Ok(Self::new_ref(v, &vm.ctx))
    }

    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, n: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.borrow_vec_mut().imul(vm, n)?;
        Ok(zelf)
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.mut_count(vm, &needle)
    }

    #[pymethod(magic)]
    pub(crate) fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.mut_contains(vm, &needle)
    }

    #[pymethod]
    fn index(
        &self,
        needle: PyObjectRef,
        range: OptionalRangeArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (start, stop) = range.saturate(self.len(), vm)?;
        let index = self.mut_index_range(vm, &needle, start..stop)?;
        if let Some(index) = index.into() {
            Ok(index)
        } else {
            Err(vm.new_value_error(format!("'{}' is not in list", needle.str(vm)?)))
        }
    }

    #[pymethod]
    fn pop(&self, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
        let mut i = i.into_option().unwrap_or(-1);
        let mut elements = self.borrow_vec_mut();
        if i < 0 {
            i += elements.len() as isize;
        }
        if elements.is_empty() {
            Err(vm.new_index_error("pop from empty list".to_owned()))
        } else if i < 0 || i as usize >= elements.len() {
            Err(vm.new_index_error("pop index out of range".to_owned()))
        } else {
            Ok(elements.remove(i as usize))
        }
    }

    #[pymethod]
    fn remove(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let index = self.mut_index(vm, &needle)?;

        if let Some(index) = index.into() {
            // defer delete out of borrow
            Ok(self.borrow_vec_mut().remove(index))
        } else {
            Err(vm.new_value_error(format!("'{}' is not in list", needle.str(vm)?)))
        }
        .map(drop)
    }

    fn _delitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "list")? {
            SequenceIndex::Int(i) => self.borrow_vec_mut().del_item_by_index(vm, i),
            SequenceIndex::Slice(slice) => self.borrow_vec_mut().del_item_by_slice(vm, slice),
        }
    }

    #[pymethod(magic)]
    fn delitem(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(&subscript, vm)
    }

    #[pymethod]
    pub(crate) fn sort(&self, options: SortOptions, vm: &VirtualMachine) -> PyResult<()> {
        // replace list contents with [] for duration of sort.
        // this prevents keyfunc from messing with the list and makes it easy to
        // check if it tries to append elements to it.
        let mut elements = std::mem::take(self.borrow_vec_mut().deref_mut());
        let res = do_sort(vm, &mut elements, options.key, options.reverse);
        std::mem::swap(self.borrow_vec_mut().deref_mut(), &mut elements);
        res?;

        if !elements.is_empty() {
            return Err(vm.new_value_error("list modified during sort".to_owned()));
        }

        Ok(())
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

fn extract_cloned<F, R>(obj: &PyObject, mut f: F, vm: &VirtualMachine) -> PyResult<Vec<R>>
where
    F: FnMut(PyObjectRef) -> PyResult<R>,
{
    use crate::builtins::PyTuple;
    if let Some(tuple) = obj.payload_if_exact::<PyTuple>(vm) {
        tuple.iter().map(|x| f(x.clone())).collect()
    } else if let Some(list) = obj.payload_if_exact::<PyList>(vm) {
        list.borrow_vec().iter().map(|x| f(x.clone())).collect()
    } else {
        let iter = obj.to_owned().get_iter(vm)?;
        let iter = iter.iter::<PyObjectRef>(vm)?;
        let len = obj.to_sequence(vm).length_opt(vm).transpose()?.unwrap_or(0);
        let mut v = Vec::with_capacity(len);
        for x in iter {
            v.push(f(x?)?);
        }
        v.shrink_to_fit();
        Ok(v)
    }
}

impl<'a> MutObjectSequenceOp<'a> for PyList {
    type Guard = PyMappedRwLockReadGuard<'a, [PyObjectRef]>;

    fn do_get(index: usize, guard: &Self::Guard) -> Option<&PyObjectRef> {
        guard.get(index)
    }

    fn do_lock(&'a self) -> Self::Guard {
        self.borrow_vec()
    }
}

impl Constructor for PyList {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyList::default()
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl Initializer for PyList {
    type Args = OptionalArg<PyObjectRef>;

    fn init(zelf: PyRef<Self>, iterable: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let mut elements = if let OptionalArg::Present(iterable) = iterable {
            iterable.try_to_value(vm)?
        } else {
            vec![]
        };
        std::mem::swap(zelf.borrow_vec_mut().deref_mut(), &mut elements);
        Ok(())
    }
}

impl AsMapping for PyList {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyList::mapping_downcast(mapping).len())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyList::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyList::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf._setitem(needle, value, vm)
                } else {
                    zelf._delitem(needle, vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyList {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyList::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                PyList::sequence_downcast(seq)
                    .concat(other, vm)
                    .map(|x| x.into())
            }),
            repeat: atomic_func!(|seq, n, vm| {
                PyList::sequence_downcast(seq)
                    .mul(n as isize, vm)
                    .map(|x| x.into())
            }),
            item: atomic_func!(|seq, i, vm| {
                PyList::sequence_downcast(seq)
                    .borrow_vec()
                    .getitem_by_index(vm, i)
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyList::sequence_downcast(seq);
                if let Some(value) = value {
                    zelf.borrow_vec_mut().setitem_by_index(vm, i, value)
                } else {
                    zelf.borrow_vec_mut().del_item_by_index(vm, i)
                }
            }),
            contains: atomic_func!(|seq, target, vm| {
                let zelf = PyList::sequence_downcast(seq);
                zelf.mut_contains(vm, target)
            }),
            inplace_concat: atomic_func!(|seq, other, vm| {
                let zelf = PyList::sequence_downcast(seq);
                PyList::inplace_concat(zelf, other, vm)
            }),
            inplace_repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyList::sequence_downcast(seq);
                zelf.borrow_vec_mut().imul(vm, n as isize)?;
                Ok(zelf.to_owned().into())
            }),
        };
        &AS_SEQUENCE
    }
}

impl Iterable for PyList {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyListIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

impl Comparable for PyList {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        let a = &*zelf.borrow_vec();
        let b = &*other.borrow_vec();
        a.iter()
            .richcompare(b.iter(), op, vm)
            .map(PyComparisonValue::Implemented)
    }
}

impl Unhashable for PyList {}

fn do_sort(
    vm: &VirtualMachine,
    values: &mut Vec<PyObjectRef>,
    key_func: Option<PyObjectRef>,
    reverse: bool,
) -> PyResult<()> {
    let op = if reverse {
        PyComparisonOp::Lt
    } else {
        PyComparisonOp::Gt
    };
    let cmp = |a: &PyObjectRef, b: &PyObjectRef| a.rich_compare_bool(b, op, vm);

    if let Some(ref key_func) = key_func {
        let mut items = values
            .iter()
            .map(|x| Ok((x.clone(), vm.invoke(key_func, (x.clone(),))?)))
            .collect::<Result<Vec<_>, _>>()?;
        timsort::try_sort_by_gt(&mut items, |a, b| cmp(&a.1, &b.1))?;
        *values = items.into_iter().map(|(val, _)| val).collect();
    } else {
        timsort::try_sort_by_gt(values, cmp)?;
    }

    Ok(())
}

#[pyclass(module = false, name = "list_iterator")]
#[derive(Debug)]
pub struct PyListIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyPayload for PyListIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.list_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyListIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.len())
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyListIterator {}

impl IterNextIterable for PyListIterator {}
impl IterNext for PyListIterator {
    fn next(zelf: &crate::Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(PyIterReturn::from_result(vec.get(pos).cloned().ok_or(None)))
        })
    }
}

#[pyclass(module = false, name = "list_reverseiterator")]
#[derive(Debug)]
pub struct PyListReverseIterator {
    internal: PyMutex<PositionIterInternal<PyListRef>>,
}

impl PyPayload for PyListReverseIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.list_reverseiterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyListReverseIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().rev_length_hint(|obj| obj.len())
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_reversed_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyListReverseIterator {}

impl IterNextIterable for PyListReverseIterator {}
impl IterNext for PyListReverseIterator {
    fn next(zelf: &crate::Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().rev_next(|list, pos| {
            let vec = list.borrow_vec();
            Ok(PyIterReturn::from_result(vec.get(pos).cloned().ok_or(None)))
        })
    }
}

pub fn init(context: &Context) {
    let list_type = &context.types.list_type;
    PyList::extend_class(context, list_type);

    PyListIterator::extend_class(context, context.types.list_iterator_type);
    PyListReverseIterator::extend_class(context, context.types.list_reverseiterator_type);
}

use once_cell::sync::Lazy;

use super::{PositionIterInternal, PyGenericAlias, PyType, PyTypeRef};
use crate::common::{
    hash::{PyHash, PyUHash},
    lock::PyMutex,
};
use crate::{
    atomic_func,
    class::PyClassImpl,
    convert::{ToPyObject, TransmuteFromObject},
    function::{OptionalArg, PyArithmeticValue, PyComparisonValue},
    iter::PyExactSizeIterator,
    protocol::{PyIterReturn, PyMappingMethods, PySequenceMethods},
    recursion::ReprGuard,
    sequence::{OptionalRangeArgs, SequenceExt},
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsMapping, AsSequence, Comparable, Constructor, Hashable, IterNext, IterNextIterable,
        Iterable, PyComparisonOp, Unconstructible,
    },
    utils::collection_repr,
    vm::VirtualMachine,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
};
use std::{fmt, marker::PhantomData};

#[pyclass(module = false, name = "tuple")]
pub struct PyTuple {
    elements: Box<[PyObjectRef]>,
}

impl fmt::Debug for PyTuple {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more informational, non-recursive Debug formatter
        f.write_str("tuple")
    }
}

impl PyPayload for PyTuple {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.tuple_type
    }
}

pub trait IntoPyTuple {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef;
}

impl IntoPyTuple for () {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
        vm.ctx.empty_tuple.clone()
    }
}

impl IntoPyTuple for Vec<PyObjectRef> {
    fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
        PyTuple::new_ref(self, &vm.ctx)
    }
}

macro_rules! impl_intopyobj_tuple {
    ($(($T:ident, $idx:tt)),+) => {
        impl<$($T: ToPyObject),*> IntoPyTuple for ($($T,)*) {
            fn into_pytuple(self, vm: &VirtualMachine) -> PyTupleRef {
                PyTuple::new_ref(vec![$(self.$idx.to_pyobject(vm)),*], &vm.ctx)
            }
        }

        impl<$($T: ToPyObject),*> ToPyObject for ($($T,)*) {
            fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                self.into_pytuple(vm).into()
            }
        }
    };
}

impl_intopyobj_tuple!((A, 0));
impl_intopyobj_tuple!((A, 0), (B, 1));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_intopyobj_tuple!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));

impl PyTuple {
    pub(crate) fn fast_getitem(&self, idx: usize) -> PyObjectRef {
        self.elements[idx].clone()
    }
}

pub type PyTupleRef = PyRef<PyTuple>;

impl Constructor for PyTuple {
    type Args = OptionalArg<PyObjectRef>;

    fn py_new(cls: PyTypeRef, iterable: Self::Args, vm: &VirtualMachine) -> PyResult {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(vm.ctx.types.tuple_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(tuple) => return Ok(tuple.into_pyref().into()),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            iterable.try_to_value(vm)?
        } else {
            vec![]
        };
        // Return empty tuple only for exact tuple types if the iterable is empty.
        if elements.is_empty() && cls.is(vm.ctx.types.tuple_type) {
            Ok(vm.ctx.empty_tuple.clone().into())
        } else {
            Self {
                elements: elements.into_boxed_slice(),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }
}

impl AsRef<[PyObjectRef]> for PyTuple {
    fn as_ref(&self) -> &[PyObjectRef] {
        self.as_slice()
    }
}

impl std::ops::Deref for PyTuple {
    type Target = [PyObjectRef];
    fn deref(&self) -> &[PyObjectRef] {
        self.as_slice()
    }
}

impl<'a> std::iter::IntoIterator for &'a PyTuple {
    type Item = &'a PyObjectRef;
    type IntoIter = std::slice::Iter<'a, PyObjectRef>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> std::iter::IntoIterator for &'a PyTupleRef {
    type Item = &'a PyObjectRef;
    type IntoIter = std::slice::Iter<'a, PyObjectRef>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl PyTuple {
    pub fn new_ref(elements: Vec<PyObjectRef>, ctx: &Context) -> PyRef<Self> {
        if elements.is_empty() {
            ctx.empty_tuple.clone()
        } else {
            let elements = elements.into_boxed_slice();
            PyRef::new_ref(Self { elements }, ctx.types.tuple_type.to_owned(), None)
        }
    }

    /// Creating a new tuple with given boxed slice.
    /// NOTE: for usual case, you probably want to use PyTuple::new_ref.
    /// Calling this function implies trying micro optimization for non-zero-sized tuple.
    pub fn new_unchecked(elements: Box<[PyObjectRef]>) -> Self {
        Self { elements }
    }

    pub fn as_slice(&self) -> &[PyObjectRef] {
        &self.elements
    }
}

#[pyclass(
    flags(BASETYPE),
    with(AsMapping, AsSequence, Hashable, Comparable, Iterable, Constructor)
)]
impl PyTuple {
    #[pymethod(magic)]
    fn add(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyArithmeticValue<PyRef<Self>> {
        let added = other.downcast::<Self>().map(|other| {
            if other.elements.is_empty() && zelf.class().is(vm.ctx.types.tuple_type) {
                zelf
            } else if zelf.elements.is_empty() && other.class().is(vm.ctx.types.tuple_type) {
                other
            } else {
                let elements = zelf
                    .iter()
                    .chain(other.as_slice())
                    .cloned()
                    .collect::<Box<[_]>>();
                Self { elements }.into_ref(vm)
            }
        });
        PyArithmeticValue::from_option(added.ok())
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.elements.is_empty()
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self {
            if vm.identical_or_equal(element, &needle)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod(magic)]
    #[inline]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let s = if zelf.len() == 0 {
            "()".to_owned()
        } else if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            if zelf.len() == 1 {
                format!("({},)", zelf.elements[0].repr(vm)?)
            } else {
                collection_repr(None, "(", ")", zelf.elements.iter(), vm)?
            }
        } else {
            "(...)".to_owned()
        };
        Ok(s)
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Ok(if zelf.elements.is_empty() || value == 0 {
            vm.ctx.empty_tuple.clone()
        } else if value == 1 && zelf.class().is(vm.ctx.types.tuple_type) {
            // Special case: when some `tuple` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `tuple` itself, not its subclasses.
            zelf
        } else {
            let v = zelf.elements.mul(vm, value)?;
            let elements = v.into_boxed_slice();
            Self { elements }.into_ref(vm)
        })
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "tuple")? {
            SequenceIndex::Int(i) => self.elements.getitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => self
                .elements
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_tuple(x).into()),
        }
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    #[pymethod]
    fn index(
        &self,
        needle: PyObjectRef,
        range: OptionalRangeArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (start, stop) = range.saturate(self.len(), vm)?;
        for (index, element) in self.elements.iter().enumerate().take(stop).skip(start) {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("tuple.index(x): x not in tuple".to_owned()))
    }

    fn _contains(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self._contains(&needle, vm)
    }

    #[pymethod(magic)]
    fn getnewargs(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTupleRef,) {
        // the arguments to pass to tuple() is just one tuple - so we'll be doing tuple(tup), which
        // should just return tup, or tuplesubclass(tup), which'll copy/validate (e.g. for a
        // structseq)
        let tup_arg = if zelf.class().is(vm.ctx.types.tuple_type) {
            zelf
        } else {
            PyTuple::new_ref(zelf.elements.clone().into_vec(), &vm.ctx)
        };
        (tup_arg,)
    }

    #[pyclassmethod(magic)]
    fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::new(cls, args, vm)
    }
}

impl AsMapping for PyTuple {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyTuple::mapping_downcast(mapping).len())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyTuple::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyTuple {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyTuple::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                let zelf = PyTuple::sequence_downcast(seq);
                match PyTuple::add(zelf.to_owned(), other.to_owned(), vm) {
                    PyArithmeticValue::Implemented(tuple) => Ok(tuple.into()),
                    PyArithmeticValue::NotImplemented => Err(vm.new_type_error(format!(
                        "can only concatenate tuple (not '{}') to tuple",
                        other.class().name()
                    ))),
                }
            }),
            repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyTuple::sequence_downcast(seq);
                PyTuple::mul(zelf.to_owned(), n, vm).map(|x| x.into())
            }),
            item: atomic_func!(|seq, i, vm| {
                let zelf = PyTuple::sequence_downcast(seq);
                zelf.elements.getitem_by_index(vm, i)
            }),
            contains: atomic_func!(|seq, needle, vm| {
                let zelf = PyTuple::sequence_downcast(seq);
                zelf._contains(needle, vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Hashable for PyTuple {
    #[inline]
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        tuple_hash(zelf.as_slice(), vm)
    }
}

impl Comparable for PyTuple {
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
        zelf.iter()
            .richcompare(other.iter(), op, vm)
            .map(PyComparisonValue::Implemented)
    }
}

impl Iterable for PyTuple {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyTupleIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

#[pyclass(module = false, name = "tuple_iterator")]
#[derive(Debug)]
pub(crate) struct PyTupleIterator {
    internal: PyMutex<PositionIterInternal<PyTupleRef>>,
}

impl PyPayload for PyTupleIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.tuple_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyTupleIterator {
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
impl Unconstructible for PyTupleIterator {}

impl IterNextIterable for PyTupleIterator {}
impl IterNext for PyTupleIterator {
    fn next(zelf: &crate::Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|tuple, pos| {
            Ok(PyIterReturn::from_result(
                tuple.get(pos).cloned().ok_or(None),
            ))
        })
    }
}

pub(crate) fn init(context: &Context) {
    PyTuple::extend_class(context, context.types.tuple_type);
    PyTupleIterator::extend_class(context, context.types.tuple_iterator_type);
}

pub struct PyTupleTyped<T: TransmuteFromObject> {
    // SAFETY INVARIANT: T must be repr(transparent) over PyObjectRef, and the
    //                   elements must be logically valid when transmuted to T
    tuple: PyTupleRef,
    _marker: PhantomData<Vec<T>>,
}

impl<T: TransmuteFromObject> TryFromObject for PyTupleTyped<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let tuple = PyTupleRef::try_from_object(vm, obj)?;
        for elem in &tuple {
            T::check(vm, elem)?
        }
        // SAFETY: the contract of TransmuteFromObject upholds the variant on `tuple`
        Ok(Self {
            tuple,
            _marker: PhantomData,
        })
    }
}

impl<T: TransmuteFromObject> AsRef<[T]> for PyTupleTyped<T> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: TransmuteFromObject> PyTupleTyped<T> {
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        unsafe { &*(self.tuple.as_slice() as *const [PyObjectRef] as *const [T]) }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.tuple.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tuple.is_empty()
    }
}

impl<T: TransmuteFromObject> Clone for PyTupleTyped<T> {
    fn clone(&self) -> Self {
        Self {
            tuple: self.tuple.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: TransmuteFromObject + fmt::Debug> fmt::Debug for PyTupleTyped<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<T: TransmuteFromObject> From<PyTupleTyped<T>> for PyTupleRef {
    #[inline]
    fn from(tup: PyTupleTyped<T>) -> Self {
        tup.tuple
    }
}

impl<T: TransmuteFromObject> ToPyObject for PyTupleTyped<T> {
    #[inline]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.tuple.into()
    }
}

pub(super) fn tuple_hash(elements: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<PyHash> {
    #[cfg(target_pointer_width = "64")]
    const PRIME1: PyUHash = 11400714785074694791;
    #[cfg(target_pointer_width = "64")]
    const PRIME2: PyUHash = 14029467366897019727;
    #[cfg(target_pointer_width = "64")]
    const PRIME5: PyUHash = 2870177450012600261;
    #[cfg(target_pointer_width = "64")]
    const ROTATE: u32 = 31;

    #[cfg(target_pointer_width = "32")]
    const PRIME1: PyUHash = 2654435761;
    #[cfg(target_pointer_width = "32")]
    const PRIME2: PyUHash = 2246822519;
    #[cfg(target_pointer_width = "32")]
    const PRIME5: PyUHash = 374761393;
    #[cfg(target_pointer_width = "32")]
    const ROTATE: u32 = 13;

    let mut acc = PRIME5;
    let len = elements.len() as PyUHash;

    for val in elements {
        let lane = val.hash(vm)? as PyUHash;
        acc = acc.wrapping_add(lane.wrapping_mul(PRIME2));
        acc = acc.rotate_left(ROTATE);
        acc = acc.wrapping_mul(PRIME1);
    }

    acc = acc.wrapping_add(len ^ (PRIME5 ^ 3527539));

    if acc as PyHash == -1 {
        return Ok(1546275796);
    }
    Ok(acc as PyHash)
}

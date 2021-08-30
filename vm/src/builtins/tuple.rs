use crossbeam_utils::atomic::AtomicCell;
use std::fmt;
use std::marker::PhantomData;

use super::pytype::PyTypeRef;
use super::{
    int,
    iter::{
        IterStatus,
        IterStatus::{Active, Exhausted},
    },
    PyInt,
};
use crate::common::hash::PyHash;
use crate::function::OptionalArg;
use crate::sequence::{self, SimpleSeq};
use crate::sliceable::PySliceableSequence;
use crate::slots::{Comparable, Hashable, Iterable, PyComparisonOp, PyIter};
use crate::utils::Either;
use crate::vm::{ReprGuard, VirtualMachine};
use crate::{
    IdProtocol, IntoPyObject, PyArithmaticValue, PyClassDef, PyClassImpl, PyComparisonValue,
    PyContext, PyObjectRef, PyRef, PyResult, PyValue, TransmuteFromObject, TryFromObject,
    TypeProtocol,
};

/// tuple() -> empty tuple
/// tuple(iterable) -> tuple initialized from iterable's items
///
/// If the argument is a tuple, the return value is the same object.
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

impl PyValue for PyTuple {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_type
    }
}

macro_rules! impl_intopyobj_tuple {
    ($(($T:ident, $idx:tt)),+) => {
        impl<$($T: IntoPyObject),*> IntoPyObject for ($($T,)*) {
            fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_tuple(vec![$(self.$idx.into_pyobject(vm)),*])
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

impl PyTupleRef {
    pub(crate) fn with_elements(elements: Vec<PyObjectRef>, ctx: &PyContext) -> Self {
        if elements.is_empty() {
            ctx.empty_tuple.clone()
        } else {
            let elements = elements.into_boxed_slice();
            Self::new_ref(PyTuple { elements }, ctx.types.tuple_type.clone(), None)
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable))]
impl PyTuple {
    /// Creating a new tuple with given boxed slice.
    /// NOTE: for usual case, you probably want to use PyTupleRef::with_elements.
    /// Calling this function implies trying micro optimization for non-zero-sized tuple.
    pub(crate) fn _new(elements: Box<[PyObjectRef]>) -> Self {
        Self { elements }
    }

    pub fn as_slice(&self) -> &[PyObjectRef] {
        &self.elements
    }

    #[pymethod(magic)]
    fn add(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyArithmaticValue<PyRef<Self>> {
        let added = other.downcast::<Self>().map(|other| {
            if other.elements.is_empty() && zelf.class().is(&vm.ctx.types.tuple_type) {
                zelf
            } else if zelf.elements.is_empty() && other.class().is(&vm.ctx.types.tuple_type) {
                other
            } else {
                let elements = zelf
                    .as_slice()
                    .iter()
                    .chain(other.as_slice())
                    .cloned()
                    .collect::<Box<[_]>>();
                Self { elements }.into_ref(vm)
            }
        });
        PyArithmaticValue::from_option(added.ok())
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.elements.is_empty()
    }

    #[pymethod]
    fn count(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in self.elements.iter() {
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
        let s = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let mut str_parts = Vec::with_capacity(zelf.elements.len());
            for elem in zelf.elements.iter() {
                let s = vm.to_repr(elem)?;
                str_parts.push(s.as_str().to_owned());
            }

            if str_parts.len() == 1 {
                format!("({},)", str_parts[0])
            } else {
                format!("({})", str_parts.join(", "))
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
        } else if value == 1 && zelf.class().is(&vm.ctx.types.tuple_type) {
            // Special case: when some `tuple` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `tuple` itself, not its subclasses.
            zelf
        } else {
            let elements = sequence::seq_mul(vm, &zelf.elements, value)?
                .cloned()
                .collect::<Vec<_>>()
                .into_boxed_slice();
            Self { elements }.into_ref(vm)
        })
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let result = match zelf.elements.as_ref().get_item(vm, needle, Self::NAME)? {
            Either::A(obj) => obj,
            Either::B(vec) => vm.ctx.new_tuple(vec),
        };
        Ok(result)
    }

    #[pymethod]
    fn index(
        &self,
        needle: PyObjectRef,
        start: OptionalArg<isize>,
        stop: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let mut start = start.into_option().unwrap_or(0);
        if start < 0 {
            start += self.as_slice().len() as isize;
            if start < 0 {
                start = 0;
            }
        }
        let mut stop = stop.into_option().unwrap_or(isize::MAX);
        if stop < 0 {
            stop += self.as_slice().len() as isize;
            if stop < 0 {
                stop = 0;
            }
        }
        for (index, element) in self
            .elements
            .iter()
            .enumerate()
            .take(stop as usize)
            .skip(start as usize)
        {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("tuple.index(x): x not in tuple".to_owned()))
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        for element in self.elements.iter() {
            if vm.identical_or_equal(element, &needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[pymethod(magic)]
    fn getnewargs(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTupleRef,) {
        // the arguments to pass to tuple() is just one tuple - so we'll be doing tuple(tup), which
        // should just return tup, or tuplesubclass(tup), which'll copy/validate (e.g. for a
        // structseq)
        let tup_arg = if zelf.class().is(&vm.ctx.types.tuple_type) {
            zelf
        } else {
            PyTupleRef::with_elements(zelf.elements.clone().into_vec(), &vm.ctx)
        };
        (tup_arg,)
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        iterable: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let elements = if let OptionalArg::Present(iterable) = iterable {
            let iterable = if cls.is(&vm.ctx.types.tuple_type) {
                match iterable.downcast_exact::<Self>(vm) {
                    Ok(tuple) => return Ok(tuple),
                    Err(iterable) => iterable,
                }
            } else {
                iterable
            };
            vm.extract_elements(&iterable)?
        } else {
            vec![]
        };
        // Return empty tuple only for exact tuple types if the iterable is empty.
        if elements.is_empty() && cls.is(&vm.ctx.types.tuple_type) {
            Ok(vm.ctx.empty_tuple.clone())
        } else {
            Self {
                elements: elements.into_boxed_slice(),
            }
            .into_ref_with_type(vm, cls)
        }
    }
}

impl Hashable for PyTuple {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        crate::utils::hash_iter(zelf.elements.iter(), vm)
    }
}

impl Comparable for PyTuple {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        let a = zelf.as_slice();
        let b = other.as_slice();
        sequence::cmp(vm, a.boxed_iter(), b.boxed_iter(), op).map(PyComparisonValue::Implemented)
    }
}

impl Iterable for PyTuple {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyTupleIterator {
            position: AtomicCell::new(0),
            status: AtomicCell::new(Active),
            tuple: zelf,
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "tuple_iterator")]
#[derive(Debug)]
pub(crate) struct PyTupleIterator {
    position: AtomicCell<usize>,
    status: AtomicCell<IterStatus>,
    tuple: PyTupleRef,
}

impl PyValue for PyTupleIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.tuple_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyTupleIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        match self.status.load() {
            Active => self.tuple.len().saturating_sub(self.position.load()),
            Exhausted => 0,
        }
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // When we're exhausted, just return.
        if let Exhausted = self.status.load() {
            return Ok(());
        }
        // Else, set to min of (pos, tuple_size).
        if let Some(i) = state.payload::<PyInt>() {
            let position = std::cmp::min(
                int::try_to_primitive(i.as_bigint(), vm).unwrap_or(0),
                self.tuple.len(),
            );
            self.position.store(position);
            Ok(())
        } else {
            Err(vm.new_type_error("an integer is required.".to_owned()))
        }
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult {
        let iter = vm.get_attribute(vm.builtins.clone(), "iter")?;
        Ok(match self.status.load() {
            Exhausted => vm
                .ctx
                .new_tuple(vec![iter, vm.ctx.new_tuple(vec![vm.ctx.new_list(vec![])])]),
            Active => vm.ctx.new_tuple(vec![
                iter,
                vm.ctx.new_tuple(vec![self.tuple.clone().into_object()]),
                vm.ctx.new_int(self.position.load()),
            ]),
        })
    }
}

impl PyIter for PyTupleIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if let Exhausted = zelf.status.load() {
            return Err(vm.new_stop_iteration());
        }
        let pos = zelf.position.fetch_add(1);
        if let Some(obj) = zelf.tuple.as_slice().get(pos) {
            Ok(obj.clone())
        } else {
            zelf.status.store(Exhausted);
            Err(vm.new_stop_iteration())
        }
    }
}

pub(crate) fn init(context: &PyContext) {
    PyTuple::extend_class(context, &context.types.tuple_type);
    PyTupleIterator::extend_class(context, &context.types.tuple_iterator_type);
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
        for elem in tuple.as_slice() {
            T::check(vm, elem)?
        }
        // SAFETY: the contract of TransmuteFromObject upholds the variant on `tuple`
        Ok(Self {
            tuple,
            _marker: PhantomData,
        })
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

impl<T: TransmuteFromObject> IntoPyObject for PyTupleTyped<T> {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.tuple.into_object()
    }
}

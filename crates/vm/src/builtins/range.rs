use super::{
    PyGenericAlias, PyInt, PyIntRef, PySlice, PyTupleRef, PyType, PyTypeRef, builtins_iter,
    tuple::tuple_hash,
};
use crate::common::lock::{LazyLock, PyMutex};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine, atomic_func,
    class::{PyClassDef, PyClassImpl},
    common::hash::PyHash,
    function::{ArgIndex, FuncArgs, OptionalArg, PyComparisonValue},
    protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Hashable, IterNext, Iterable, PyComparisonOp,
        Representable, SelfIter,
    },
};
use core::cell::Cell;
use core::ffi::c_long;
use core::ptr::NonNull;
use crossbeam_utils::atomic::AtomicCell;
use malachite_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};

#[pyclass(module = false, name = "range")]
#[derive(Debug, Clone)]
pub struct PyRange {
    #[pymember(type = "object_ex")]
    pub start: PyIntRef,
    #[pymember(type = "object_ex")]
    pub stop: PyIntRef,
    #[pymember(type = "object_ex")]
    pub step: PyIntRef,
}

// spell-checker:ignore MAXFREELIST
thread_local! {
    static RANGE_FREELIST: Cell<crate::object::FreeList<PyRange>> = const { Cell::new(crate::object::FreeList::new()) };
}

impl PyPayload for PyRange {
    const MAX_FREELIST: usize = 6;
    const HAS_FREELIST: bool = true;

    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.range_type
    }

    #[inline]
    unsafe fn freelist_push(obj: *mut PyObject) -> bool {
        RANGE_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let stored = if list.len() < Self::MAX_FREELIST {
                    list.push(obj);
                    true
                } else {
                    false
                };
                fl.set(list);
                stored
            })
            .unwrap_or(false)
    }

    #[inline]
    unsafe fn freelist_pop(_payload: &Self) -> Option<NonNull<PyObject>> {
        RANGE_FREELIST
            .try_with(|fl| {
                let mut list = fl.take();
                let result = list.pop().map(|p| unsafe { NonNull::new_unchecked(p) });
                fl.set(list);
                result
            })
            .ok()
            .flatten()
    }
}

impl PyRange {
    #[inline]
    fn offset(&self, value: &BigInt) -> Option<BigInt> {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();
        match step.sign() {
            Sign::Plus if value >= start && value < stop => Some(value - start),
            Sign::Minus if value <= self.start.as_bigint() && value > stop => Some(start - value),
            _ => None,
        }
    }

    #[inline]
    #[must_use]
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        let step = self.step.as_bigint();
        match self.offset(value) {
            Some(ref offset) if offset.is_multiple_of(step) => Some((offset / step).abs()),
            Some(_) | None => None,
        }
    }

    fn contains_int(&self, value: &BigInt) -> bool {
        self.offset(value)
            .is_some_and(|offset| offset.is_multiple_of(self.step.as_bigint()))
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.compute_length().is_zero()
    }

    #[inline]
    #[must_use]
    pub fn forward(&self) -> bool {
        self.start.as_bigint() < self.stop.as_bigint()
    }

    #[inline]
    #[must_use]
    pub fn get(&self, index: &BigInt) -> Option<BigInt> {
        let start = self.start.as_bigint();
        let step = self.step.as_bigint();
        let stop = self.stop.as_bigint();
        if self.is_empty() {
            return None;
        }

        if index.is_negative() {
            let length = self.compute_length();
            let index: BigInt = &length + index;
            if index.is_negative() {
                return None;
            }

            Some(if step.is_one() {
                start + index
            } else {
                start + step * index
            })
        } else {
            let index = if step.is_one() {
                start + index
            } else {
                start + step * index
            };

            if (step.is_positive() && stop > &index) || (step.is_negative() && stop < &index) {
                Some(index)
            } else {
                None
            }
        }
    }

    #[inline]
    fn compute_length(&self) -> BigInt {
        let start = self.start.as_bigint();
        let stop = self.stop.as_bigint();
        let step = self.step.as_bigint();

        match step.sign() {
            Sign::Plus if start < stop => {
                if step.is_one() {
                    stop - start
                } else {
                    (stop - start - 1usize) / step + 1
                }
            }
            Sign::Minus if start > stop => (start - stop - 1usize) / (-step) + 1,
            Sign::Plus | Sign::Minus => BigInt::zero(),
            Sign::NoSign => unreachable!(),
        }
    }

    fn fast_iterator(&self, length: &BigInt, reverse: bool) -> Option<PyRangeIterator> {
        // CPython uses C long, which can be narrower than isize on Windows.
        let start = c_long::try_from(self.start.as_bigint()).ok()?;
        let stop = c_long::try_from(self.stop.as_bigint()).ok()?;
        let step = c_long::try_from(self.step.as_bigint()).ok()?;
        let length = c_long::try_from(length).ok()?;
        let (start, step) = if reverse {
            let reverse_step = step.checked_neg()?;
            let reverse_stop = start.checked_sub(step)?;
            // The product may overflow even though the reversed start fits.
            (
                reverse_stop.wrapping_add(length.wrapping_mul(step)),
                reverse_step,
            )
        } else {
            if length != 0 {
                // Reserve space for the exhausted position, as CPython does.
                // Its conservative check uses stop, not the final item.
                if step > 0 {
                    stop.checked_add(step - 1)?;
                } else {
                    stop.checked_add(step + 1)?;
                }
            }
            (start, step)
        };
        Some(PyRangeIterator {
            index: AtomicCell::new(0),
            start: start as isize,
            step: step as isize,
            length: length as usize,
        })
    }

    fn iterator(&self, reverse: bool, vm: &VirtualMachine) -> PyObjectRef {
        let length = self.compute_length();
        if let Some(iter) = self.fast_iterator(&length, reverse) {
            return iter.into_pyobject(vm);
        }
        let start = self.start.as_bigint();
        let step = self.step.as_bigint();
        let (start, step) = if reverse {
            (start + (&length - 1) * step, -step)
        } else {
            (start.clone(), step.clone())
        };
        PyLongRangeIterator {
            index: PyMutex::new(BigInt::zero()),
            start,
            step,
            length,
        }
        .into_pyobject(vm)
    }
}

// pub fn get_value(obj: &PyObject) -> PyRange {
//     obj.downcast_ref::<PyRange>().unwrap().clone()
// }

pub(crate) fn init(context: &'static Context) {
    PyRange::extend_class(context, context.types.range_type);
    PyLongRangeIterator::extend_class(context, context.types.long_range_iterator_type);
    PyRangeIterator::extend_class(context, context.types.range_iterator_type);
}

impl PyRange {
    fn __len__(&self) -> BigInt {
        self.compute_length()
    }

    fn __getitem__(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match RangeIndex::try_from_object(vm, subscript)? {
            RangeIndex::Slice(slice) => {
                let (mut sub_start, mut sub_stop, mut sub_step) =
                    slice.inner_indices(&self.compute_length(), vm)?;
                let range_step = &self.step;
                let range_start = &self.start;

                sub_step *= range_step.as_bigint();
                sub_start = (sub_start * range_step.as_bigint()) + range_start.as_bigint();
                sub_stop = (sub_stop * range_step.as_bigint()) + range_start.as_bigint();

                Ok(Self {
                    start: vm.ctx.new_pyref(sub_start),
                    stop: vm.ctx.new_pyref(sub_stop),
                    step: vm.ctx.new_pyref(sub_step),
                }
                .into_ref(&vm.ctx)
                .into())
            }
            RangeIndex::Int(index) => match self.get(index.as_bigint()) {
                Some(value) => Ok(vm.ctx.new_int(value).into()),
                None => Err(vm.new_index_error("range object index out of range")),
            },
        }
    }

    // TODO: Uncomment when Python adds __class_getitem__ to range
    // #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass(
    with(
        Py,
        AsMapping,
        AsNumber,
        AsSequence,
        Hashable,
        Comparable,
        Iterable,
        Representable
    ),
    flags(SEQUENCE)
)]
impl PyRange {}

#[pyclass]
impl Py<PyRange> {
    fn search_int<'a>(needle: &'a PyObject, vm: &VirtualMachine) -> Option<&'a Py<PyInt>> {
        // Other int subclasses may override equality and must be iterated.
        if needle.class().is(vm.ctx.types.bool_type) {
            needle.downcast_ref::<PyInt>()
        } else {
            needle.downcast_ref_if_exact::<PyInt>(vm)
        }
    }

    fn __contains__(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(int) = Self::search_int(needle, vm) {
            Ok(self.contains_int(int.as_bigint()))
        } else {
            self.as_object()
                .sequence_unchecked()
                .iter_contains(needle, vm)
        }
    }

    #[pymethod]
    fn index(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<BigInt> {
        if let Some(int) = Self::search_int(&object, vm) {
            self.index_of(int.as_bigint())
                .ok_or_else(|| vm.new_value_error("range.index(x): x not in range"))
        } else {
            self.as_object()
                .sequence_unchecked()
                .index(&object, vm)
                .map(BigInt::from)
        }
    }

    #[pymethod]
    fn count(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        if let Some(int) = Self::search_int(&object, vm) {
            Ok(usize::from(self.contains_int(int.as_bigint())))
        } else {
            self.as_object().sequence_unchecked().count(&object, vm)
        }
    }

    #[pymethod]
    fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.iterator(true, vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
        let range_parameters: Vec<PyObjectRef> = [&self.start, &self.stop, &self.step]
            .iter()
            .map(|x| x.as_object().to_owned())
            .collect();
        let range_parameters_tuple = vm.ctx.new_tuple(range_parameters);
        (vm.ctx.types.range_type.to_owned(), range_parameters_tuple)
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if let Some(error) = args.check_kwargs_empty_for(vm, PyRange::NAME) {
            return Err(error);
        }
        let (first, stop, step): (
            PyObjectRef,
            OptionalArg<PyObjectRef>,
            OptionalArg<PyObjectRef>,
        ) = args.bind_for(vm, PyRange::NAME)?;
        // Bind objects before calling __index__, then convert in argument order.
        let index = |object: &PyObjectRef| {
            ArgIndex::try_from_object(vm, object.clone()).map(ArgIndex::into_int_ref)
        };
        let first_index = index(&first)?;
        let (start, stop_index) = if let OptionalArg::Present(stop) = &stop {
            (first_index, index(stop)?)
        } else {
            (vm.ctx.new_int(0), first_index)
        };
        let step_index = match &step {
            OptionalArg::Present(step) => index(step)?,
            OptionalArg::Missing => vm.ctx.new_int(1),
        };
        if step_index.as_bigint().is_zero() {
            return Err(vm.new_value_error("range() arg 3 must not be zero"));
        }
        PyRange {
            start,
            stop: stop_index,
            step: step_index,
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

impl PyRange {
    fn protocol_length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.__len__()
            .to_isize()
            .map(|x| x as usize)
            .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to Rust isize"))
    }
}

impl AsMapping for PyRange {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            length: atomic_func!(
                |mapping, vm| PyRange::mapping_downcast(mapping).protocol_length(vm)
            ),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyRange::mapping_downcast(mapping).__getitem__(needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyRange {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| PyRange::sequence_downcast(seq).protocol_length(vm)),
            item: atomic_func!(|seq, i, vm| {
                PyRange::sequence_downcast(seq)
                    .get(&i.into())
                    .map(|x| PyInt::from(x).into_ref(&vm.ctx).into())
                    .ok_or_else(|| vm.new_index_error("index out of range"))
            }),
            contains: atomic_func!(|seq, needle, vm| {
                PyRange::sequence_downcast(seq).__contains__(needle, vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyRange {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|number, _vm| {
                let zelf = number.obj.downcast_ref::<PyRange>().unwrap();
                Ok(!zelf.is_empty())
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Hashable for PyRange {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let length = zelf.compute_length();
        let elements = if length.is_zero() {
            [vm.ctx.new_int(length).into(), vm.ctx.none(), vm.ctx.none()]
        } else if length.is_one() {
            [
                vm.ctx.new_int(length).into(),
                zelf.start.clone().into(),
                vm.ctx.none(),
            ]
        } else {
            [
                vm.ctx.new_int(length).into(),
                zelf.start.clone().into(),
                zelf.step.clone().into(),
            ]
        };
        tuple_hash(&elements, vm)
    }
}

impl Comparable for PyRange {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            if zelf.is(other) {
                return Ok(true.into());
            }
            let rhs = class_or_notimplemented!(Self, other);
            let lhs_len = zelf.compute_length();
            let eq = if lhs_len != rhs.compute_length() {
                false
            } else if lhs_len.is_zero() {
                true
            } else if zelf.start.as_bigint() != rhs.start.as_bigint() {
                false
            } else if lhs_len.is_one() {
                true
            } else {
                zelf.step.as_bigint() == rhs.step.as_bigint()
            };
            Ok(eq.into())
        })
    }
}

impl Iterable for PyRange {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(zelf.iterator(false, vm))
    }
}

impl Representable for PyRange {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let repr = if zelf.step.as_bigint().is_one() {
            format!("range({}, {})", zelf.start, zelf.stop)
        } else {
            format!("range({}, {}, {})", zelf.start, zelf.stop, zelf.step)
        };
        Ok(repr)
    }
}

// Long ranges need arbitrary-precision positions as well as values.
#[pyclass(module = false, name = "longrange_iterator")]
#[derive(Debug)]
pub(crate) struct PyLongRangeIterator {
    index: PyMutex<BigInt>,
    start: BigInt,
    step: BigInt,
    length: BigInt,
}

impl PyPayload for PyLongRangeIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.long_range_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl Py<PyLongRangeIterator> {
    #[pymethod]
    fn __length_hint__(&self) -> BigInt {
        let index = self.index.lock().clone();
        if index < self.length {
            self.length.clone() - index
        } else {
            BigInt::zero()
        }
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let Some(state) = state.downcast_ref_if_exact::<PyInt>(vm) else {
            return Err(vm.new_type_error(format!(
                "state must be an int, not {}",
                state.class().fully_qualified_name(vm)?
            )));
        };
        if state.as_bigint().is_positive() {
            let mut index = self.index.lock();
            let remaining = &self.length - &*index;
            // State skips items from the current position; it cannot rewind.
            *index += state.as_bigint().min(&remaining);
        }
        Ok(())
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let index = self.index.lock().clone();
        range_iter_reduce(
            self.start.clone(),
            self.length.clone(),
            self.step.clone(),
            index,
            vm,
        )
    }
}

impl SelfIter for PyLongRangeIterator {}
impl IterNext for PyLongRangeIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let index = {
            let mut index = zelf.index.lock();
            if *index >= zelf.length {
                return Ok(PyIterReturn::StopIteration(None));
            }
            let current = index.clone();
            *index += 1;
            current
        };
        let value = zelf.start.clone() + index * zelf.step.clone();
        Ok(PyIterReturn::Return(vm.ctx.new_int(value).into()))
    }
}

// The compact iterator stores isize values for the FOR_ITER specialization.
// Creation still checks the platform's C long bounds required by CPython.
#[pyclass(module = false, name = "range_iterator")]
#[derive(Debug)]
pub(crate) struct PyRangeIterator {
    index: AtomicCell<usize>,
    start: isize,
    step: isize,
    length: usize,
}

impl PyPayload for PyRangeIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.range_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl Py<PyRangeIterator> {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        let index = self.index.load();
        self.length.saturating_sub(index)
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Convert before updating the index: __index__ can advance this iterator.
        // Keep the original state alive until after the update.
        let index = ArgIndex::try_from_object(vm, state.clone())?;
        let advance = c_long::try_from(index.as_ref().as_bigint())
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?
            .max(0) as usize;
        let _ = self
            .index
            .fetch_update(|index| Some(index.saturating_add(advance).min(self.length)));
        Ok(())
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        range_iter_reduce(
            BigInt::from(self.start),
            BigInt::from(self.length),
            BigInt::from(self.step),
            BigInt::from(self.index.load()),
            vm,
        )
    }
}

impl PyRangeIterator {
    /// Fast path for FOR_ITER specialization. Returns the next isize value
    /// without allocating PyInt or PyIterReturn.
    pub(crate) fn fast_next(&self) -> Option<isize> {
        let index = self
            .index
            .fetch_update(|index| (index < self.length).then(|| index + 1))
            .ok()?;
        Some(
            self.start
                .wrapping_add((index as isize).wrapping_mul(self.step)),
        )
    }
}

impl SelfIter for PyRangeIterator {}
impl IterNext for PyRangeIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let r = match zelf.fast_next() {
            Some(value) => PyIterReturn::Return(vm.ctx.new_int(value).into()),
            None => PyIterReturn::StopIteration(None),
        };
        Ok(r)
    }
}

fn range_iter_reduce(
    start: BigInt,
    length: BigInt,
    step: BigInt,
    index: BigInt,
    vm: &VirtualMachine,
) -> PyResult<PyTupleRef> {
    let iter = builtins_iter(vm)?;
    // CPython pickles the remaining range with a None state.
    let index = index.min(length.clone());
    let stop = start.clone() + length * step.clone();
    let start = start + index * step.clone();
    let range = PyRange {
        start: PyInt::from(start).into_ref(&vm.ctx),
        stop: PyInt::from(stop).into_ref(&vm.ctx),
        step: PyInt::from(step).into_ref(&vm.ctx),
    };
    Ok(vm.new_tuple((iter, (range,), vm.ctx.none())))
}

pub(crate) enum RangeIndex {
    Int(PyIntRef),
    Slice(PyRef<PySlice>),
}

impl TryFromObject for RangeIndex {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(match obj {
            i @ PyInt => Ok(Self::Int(i)),
            s @ PySlice => Ok(Self::Slice(s)),
            obj => {
                let val = obj.try_index_opt(vm).ok_or_else(|| {
                    let name = obj.class().slot_name();
                    vm.new_type_error(format!(
                        "range indices must be integers or slices, not {}",
                        &name[..name.floor_char_boundary(200.min(name.len()))]
                    ))
                })??;
                Ok(Self::Int(val))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_iterator_preserves_arbitrary_precision_state() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let length: BigInt = BigInt::one() << 100;
            let iter = PyLongRangeIterator {
                index: PyMutex::new(BigInt::zero()),
                start: BigInt::zero(),
                step: BigInt::one(),
                length: length.clone(),
            }
            .into_ref(&vm.ctx);
            let state: BigInt = &length - 1;
            iter.__setstate__(vm.ctx.new_bigint(&state).into(), vm)
                .unwrap();
            assert_eq!(iter.__length_hint__(), BigInt::one());
            let PyIterReturn::Return(value) = PyLongRangeIterator::next(&iter, vm).unwrap() else {
                panic!("wide iterator state was lost");
            };
            assert_eq!(value.downcast_ref::<PyInt>().unwrap().as_bigint(), &state);
            for _ in 0..2 {
                assert!(matches!(
                    PyLongRangeIterator::next(&iter, vm).unwrap(),
                    PyIterReturn::StopIteration(None)
                ));
            }
        });
    }

    #[test]
    fn exhausted_fast_iterator_does_not_wrap_its_index() {
        let iter = PyRangeIterator {
            index: AtomicCell::new(usize::MAX),
            start: 0,
            step: 1,
            length: usize::MAX,
        };
        assert_eq!(iter.fast_next(), None);
        assert_eq!(iter.fast_next(), None);
    }
}

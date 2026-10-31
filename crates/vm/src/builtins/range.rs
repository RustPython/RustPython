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
use core::ptr::NonNull;
use crossbeam_utils::atomic::AtomicCell;
use malachite_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};

// Search flag passed to iter_search
enum SearchType {
    Count,
    Contains,
    Index,
}

#[inline]
fn iter_search(
    obj: &PyObject,
    item: &PyObject,
    flag: SearchType,
    vm: &VirtualMachine,
) -> PyResult<usize> {
    let mut count = 0;
    let iter = obj.get_iter(vm)?;
    for element in iter.iter::<PyObjectRef>(vm)? {
        if vm.bool_eq(item, &*element?)? {
            match flag {
                SearchType::Index => return Ok(count),
                SearchType::Contains => return Ok(1),
                SearchType::Count => count += 1,
            }
        }
    }
    match flag {
        SearchType::Count => Ok(count),
        SearchType::Contains => Ok(0),
        SearchType::Index => Err(vm.new_value_error(format!(
            "{} not in range",
            item.repr(vm)
                .as_ref()
                .map_or_else(|_| "value".as_ref(), |s| s.as_wtf8())
                .to_owned()
        ))),
    }
}

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
    fn new(cls: PyTypeRef, stop: ArgIndex, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self {
            start: vm.ctx.new_pyref(0),
            stop: stop.into(),
            step: vm.ctx.new_pyref(1),
        }
        .into_ref_with_type(vm, cls)
    }

    fn new_from(
        cls: PyTypeRef,
        start: &PyObject,
        stop: &PyObject,
        step: OptionalArg<ArgIndex>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let step = step.map_or_else(|| vm.ctx.new_int(1), |step| step.into());
        if step.as_bigint().is_zero() {
            return Err(vm.new_value_error("range() arg 3 must not be zero"));
        }
        Self {
            start: start.try_index(vm)?,
            stop: stop.try_index(vm)?,
            step,
        }
        .into_ref_with_type(vm, cls)
    }

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
    fn contains_inner(&self, needle: &PyObject, vm: &VirtualMachine) -> bool {
        // Only accept ints, not subclasses.
        if let Some(int) = needle.downcast_ref_if_exact::<PyInt>(vm) {
            match self.offset(int.as_bigint()) {
                Some(ref offset) => offset.is_multiple_of(self.step.as_bigint()),
                None => false,
            }
        } else {
            iter_search(self.as_object(), needle, SearchType::Contains, vm).unwrap_or(0) != 0
        }
    }

    fn __contains__(&self, needle: &PyObject, vm: &VirtualMachine) -> bool {
        self.contains_inner(needle, vm)
    }

    #[pymethod]
    fn index(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<BigInt> {
        if let Ok(int) = object.clone().downcast::<PyInt>() {
            match self.index_of(int.as_bigint()) {
                Some(idx) => Ok(idx),
                None => Err(vm.new_value_error(format!("{int} is not in range"))),
            }
        } else {
            // Fallback to iteration.
            Ok(BigInt::from_bytes_be(
                Sign::Plus,
                &iter_search(self.as_object(), &object, SearchType::Index, vm)?.to_be_bytes(),
            ))
        }
    }

    #[pymethod]
    fn count(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        if let Ok(int) = object.clone().downcast::<PyInt>() {
            Ok(usize::from(self.index_of(int.as_bigint()).is_some()))
        } else {
            // Dealing with classes who might compare equal with ints in their
            // __eq__, slow search.
            iter_search(self.as_object(), &object, SearchType::Count, vm)
        }
    }

    #[pymethod]
    fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
        let start = self.start.as_bigint();
        let step = self.step.as_bigint();

        // Use CPython calculation for this:
        let length = self.__len__();
        let new_stop = start - step;
        let start = &new_stop + length.clone() * step;
        let step = -step;

        if let (Some(start), Some(step), Some(_)) =
            (start.to_isize(), step.to_isize(), new_stop.to_isize())
        {
            PyRangeIterator {
                index: AtomicCell::new(0),
                start,
                step,
                // Cannot fail. If start, stop and step all successfully convert to isize, then result of zelf.len will
                // always fit in a usize.
                length: length.to_usize().unwrap_or(0),
            }
            .into_pyobject(vm)
        } else {
            PyLongRangeIterator {
                index: PyMutex::new(BigInt::zero()),
                start,
                step,
                length,
            }
            .into_pyobject(vm)
        }
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
        let range = if args.args.is_empty() {
            return Err(vm.new_arity_type_error(PyRange::NAME, 1..=3, 0));
        } else if args.args.len() == 1 {
            let stop = args.bind_for(vm, PyRange::NAME)?;
            PyRange::new(cls, stop, vm)
        } else {
            let (start, stop, step): (PyObjectRef, PyObjectRef, OptionalArg<ArgIndex>) =
                args.bind_for(vm, PyRange::NAME)?;
            PyRange::new_from(cls, &start, &stop, step, vm)
        }?;

        Ok(range.into())
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
                Ok(PyRange::sequence_downcast(seq).contains_inner(needle, vm))
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
        let (start, stop, step, length) = (
            zelf.start.as_bigint(),
            zelf.stop.as_bigint(),
            zelf.step.as_bigint(),
            zelf.__len__(),
        );
        if let (Some(start), Some(step), Some(_), Some(_)) = (
            start.to_isize(),
            step.to_isize(),
            stop.to_isize(),
            (start + step).to_isize(),
        ) {
            Ok(PyRangeIterator {
                index: AtomicCell::new(0),
                start,
                step,
                // Cannot fail. If start, stop and step all successfully convert to isize, then result of zelf.len will
                // always fit in a usize.
                length: length.to_usize().unwrap_or(0),
            }
            .into_pyobject(vm))
        } else {
            Ok(PyLongRangeIterator {
                index: PyMutex::new(BigInt::zero()),
                start: start.clone(),
                step: step.clone(),
                length,
            }
            .into_pyobject(vm))
        }
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
        let index = range_state(&self.length, &state, vm)?;
        *self.index.lock() = index;
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

// When start, stop, step are isize, we can use a faster more compact representation
// that only operates using isize to track values.
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
    fn __setstate__(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.index.store(
            range_state(&BigInt::from(self.length), &object, vm)?
                .to_usize()
                .unwrap(),
        );
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

// Silently clips state (i.e index) in range [0, length].
fn range_state(length: &BigInt, state: &PyObject, vm: &VirtualMachine) -> PyResult<BigInt> {
    if let Some(i) = state.downcast_ref::<PyInt>() {
        Ok(i.as_bigint().clone().clamp(BigInt::zero(), length.clone()))
    } else {
        Err(vm.new_type_error("an integer is required."))
    }
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
                let val = obj.try_index(vm).map_err(|_| vm.new_type_error(format!(
                    "sequence indices be integers or slices or classes that override __index__ operator, not '{}'",
                    obj.class().name()
                )))?;
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

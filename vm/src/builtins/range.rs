use super::{
    builtins_iter, tuple::tuple_hash, PyInt, PyIntRef, PySlice, PyTupleRef, PyType, PyTypeRef,
};
use crate::{
    atomic_func,
    class::PyClassImpl,
    common::hash::PyHash,
    function::{FuncArgs, OptionalArg, PyComparisonValue},
    protocol::{PyIterReturn, PyMappingMethods, PySequenceMethods},
    types::{
        AsMapping, AsSequence, Comparable, Constructor, Hashable, IterNext, IterNextIterable,
        Iterable, PyComparisonOp, Unconstructible,
    },
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use once_cell::sync::Lazy;
use std::cmp::max;

// Search flag passed to iter_search
enum SearchType {
    Count,
    Contains,
    Index,
}

// Note: might be a good idea to merge with _membership_iter_search or generalize (_sequence_iter_check?)
// and place in vm.rs for all sequences to be able to use it.
#[inline]
fn iter_search(
    obj: PyObjectRef,
    item: PyObjectRef,
    flag: SearchType,
    vm: &VirtualMachine,
) -> PyResult<usize> {
    let mut count = 0;
    let iter = obj.get_iter(vm)?;
    for element in iter.iter_without_hint::<PyObjectRef>(vm)? {
        if vm.bool_eq(&item, &*element?)? {
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
            &item
                .repr(vm)
                .map(|v| v.as_str().to_owned())
                .unwrap_or_else(|_| "value".to_owned())
        ))),
    }
}

#[pyclass(module = false, name = "range")]
#[derive(Debug, Clone)]
pub struct PyRange {
    pub start: PyIntRef,
    pub stop: PyIntRef,
    pub step: PyIntRef,
}

impl PyPayload for PyRange {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.range_type
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
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        let step = self.step.as_bigint();
        match self.offset(value) {
            Some(ref offset) if offset.is_multiple_of(step) => Some((offset / step).abs()),
            Some(_) | None => None,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.compute_length().is_zero()
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start.as_bigint() < self.stop.as_bigint()
    }

    #[inline]
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
//     obj.payload::<PyRange>().unwrap().clone()
// }

pub fn init(context: &Context) {
    PyRange::extend_class(context, context.types.range_type);
    PyLongRangeIterator::extend_class(context, context.types.long_range_iterator_type);
    PyRangeIterator::extend_class(context, context.types.range_iterator_type);
}

#[pyclass(with(AsMapping, AsSequence, Hashable, Comparable, Iterable))]
impl PyRange {
    fn new(cls: PyTypeRef, stop: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyRange {
            start: vm.new_pyref(0),
            stop: stop.try_index(vm)?,
            step: vm.new_pyref(1),
        }
        .into_ref_with_type(vm, cls)
    }

    fn new_from(
        cls: PyTypeRef,
        start: PyObjectRef,
        stop: PyObjectRef,
        step: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let step = step.unwrap_or_else(|| vm.new_pyobj(1)).try_index(vm)?;
        if step.as_bigint().is_zero() {
            return Err(vm.new_value_error("range() arg 3 must not be zero".to_owned()));
        }
        PyRange {
            start: start.try_index(vm)?,
            stop: stop.try_index(vm)?,
            step,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pygetset]
    fn start(&self) -> PyIntRef {
        self.start.clone()
    }

    #[pygetset]
    fn stop(&self) -> PyIntRef {
        self.stop.clone()
    }

    #[pygetset]
    fn step(&self) -> PyIntRef {
        self.step.clone()
    }

    #[pymethod(magic)]
    fn reversed(&self, vm: &VirtualMachine) -> PyResult {
        let start = self.start.as_bigint();
        let step = self.step.as_bigint();

        // Use CPython calculation for this:
        let length = self.len();
        let new_stop = start - step;
        let start = &new_stop + length.clone() * step;
        let step = -step;

        Ok(
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
                    index: AtomicCell::new(0),
                    start,
                    step,
                    length,
                }
                .into_pyobject(vm)
            },
        )
    }

    #[pymethod(magic)]
    fn len(&self) -> BigInt {
        self.compute_length()
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        if self.step.as_bigint().is_one() {
            format!("range({}, {})", self.start, self.stop)
        } else {
            format!("range({}, {}, {})", self.start, self.stop, self.step)
        }
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.is_empty()
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> bool {
        // Only accept ints, not subclasses.
        if let Some(int) = needle.payload_if_exact::<PyInt>(vm) {
            match self.offset(int.as_bigint()) {
                Some(ref offset) => offset.is_multiple_of(self.step.as_bigint()),
                None => false,
            }
        } else {
            iter_search(
                self.clone().into_pyobject(vm),
                needle,
                SearchType::Contains,
                vm,
            )
            .unwrap_or(0)
                != 0
        }
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
        let range_parameters: Vec<PyObjectRef> = vec![&self.start, &self.stop, &self.step]
            .iter()
            .map(|x| x.as_object().to_owned())
            .collect();
        let range_parameters_tuple = vm.ctx.new_tuple(range_parameters);
        (vm.ctx.types.range_type.to_owned(), range_parameters_tuple)
    }

    #[pymethod]
    fn index(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<BigInt> {
        if let Ok(int) = needle.clone().downcast::<PyInt>() {
            match self.index_of(int.as_bigint()) {
                Some(idx) => Ok(idx),
                None => Err(vm.new_value_error(format!("{int} is not in range"))),
            }
        } else {
            // Fallback to iteration.
            Ok(BigInt::from_bytes_be(
                Sign::Plus,
                &iter_search(
                    self.clone().into_pyobject(vm),
                    needle,
                    SearchType::Index,
                    vm,
                )?
                .to_be_bytes(),
            ))
        }
    }

    #[pymethod]
    fn count(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        if let Ok(int) = item.clone().downcast::<PyInt>() {
            if self.index_of(int.as_bigint()).is_some() {
                Ok(1)
            } else {
                Ok(0)
            }
        } else {
            // Dealing with classes who might compare equal with ints in their
            // __eq__, slow search.
            iter_search(self.clone().into_pyobject(vm), item, SearchType::Count, vm)
        }
    }

    #[pymethod(magic)]
    fn getitem(&self, subscript: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match RangeIndex::try_from_object(vm, subscript)? {
            RangeIndex::Slice(slice) => {
                let (mut substart, mut substop, mut substep) =
                    slice.inner_indices(&self.compute_length(), vm)?;
                let range_step = &self.step;
                let range_start = &self.start;

                substep *= range_step.as_bigint();
                substart = (substart * range_step.as_bigint()) + range_start.as_bigint();
                substop = (substop * range_step.as_bigint()) + range_start.as_bigint();

                Ok(PyRange {
                    start: vm.new_pyref(substart),
                    stop: vm.new_pyref(substop),
                    step: vm.new_pyref(substep),
                }
                .into_ref(vm)
                .into())
            }
            RangeIndex::Int(index) => match self.get(index.as_bigint()) {
                Some(value) => Ok(vm.ctx.new_int(value).into()),
                None => Err(vm.new_index_error("range object index out of range".to_owned())),
            },
        }
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let range = if args.args.len() <= 1 {
            let stop = args.bind(vm)?;
            PyRange::new(cls, stop, vm)
        } else {
            let (start, stop, step) = args.bind(vm)?;
            PyRange::new_from(cls, start, stop, step, vm)
        }?;

        Ok(range.into())
    }
}

impl PyRange {
    fn protocol_length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        PyInt::from(self.len())
            .try_to_primitive::<isize>(vm)
            .map(|x| x as usize)
    }
}

impl AsMapping for PyRange {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            length: atomic_func!(
                |mapping, vm| PyRange::mapping_downcast(mapping).protocol_length(vm)
            ),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyRange::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyRange {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| PyRange::sequence_downcast(seq).protocol_length(vm)),
            item: atomic_func!(|seq, i, vm| {
                PyRange::sequence_downcast(seq)
                    .get(&i.into())
                    .map(|x| PyInt::from(x).into_ref(vm).into())
                    .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))
            }),
            contains: atomic_func!(|seq, needle, vm| {
                Ok(PyRange::sequence_downcast(seq).contains(needle.to_owned(), vm))
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Hashable for PyRange {
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let length = zelf.compute_length();
        let elements = if length.is_zero() {
            [vm.ctx.new_int(length).into(), vm.ctx.none(), vm.ctx.none()]
        } else if length.is_one() {
            [
                vm.ctx.new_int(length).into(),
                zelf.start().into(),
                vm.ctx.none(),
            ]
        } else {
            [
                vm.ctx.new_int(length).into(),
                zelf.start().into(),
                zelf.step().into(),
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
            zelf.len(),
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
                index: AtomicCell::new(0),
                start: start.clone(),
                step: step.clone(),
                length,
            }
            .into_pyobject(vm))
        }
    }
}

// Semantically, this is the same as the previous representation.
//
// Unfortunately, since AtomicCell requires a Copy type, no BigInt implementations can
// generally be used. As such, usize::MAX is the upper bound on number of elements (length)
// the range can contain in RustPython.
//
// This doesn't preclude the range from containing large values, since start and step
// can be BigInts, we can store any arbitrary range of values.
#[pyclass(module = false, name = "longrange_iterator")]
#[derive(Debug)]
pub struct PyLongRangeIterator {
    index: AtomicCell<usize>,
    start: BigInt,
    step: BigInt,
    length: BigInt,
}

impl PyPayload for PyLongRangeIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.long_range_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyLongRangeIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> BigInt {
        let index = BigInt::from(self.index.load());
        if index < self.length {
            self.length.clone() - index
        } else {
            BigInt::zero()
        }
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.index.store(range_state(&self.length, state, vm)?);
        Ok(())
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        range_iter_reduce(
            self.start.clone(),
            self.length.clone(),
            self.step.clone(),
            self.index.load(),
            vm,
        )
    }
}
impl Unconstructible for PyLongRangeIterator {}

impl IterNextIterable for PyLongRangeIterator {}
impl IterNext for PyLongRangeIterator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        // TODO: In pathological case (index == usize::MAX) this can wrap around
        // (since fetch_add wraps). This would result in the iterator spinning again
        // from the beginning.
        let index = BigInt::from(zelf.index.fetch_add(1));
        let r = if index < zelf.length {
            let value = zelf.start.clone() + index * zelf.step.clone();
            PyIterReturn::Return(vm.ctx.new_int(value).into())
        } else {
            PyIterReturn::StopIteration(None)
        };
        Ok(r)
    }
}

// When start, stop, step are isizes, we can use a faster more compact representation
// that only operates using isizes to track values.
#[pyclass(module = false, name = "range_iterator")]
#[derive(Debug)]
pub struct PyRangeIterator {
    index: AtomicCell<usize>,
    start: isize,
    step: isize,
    length: usize,
}

impl PyPayload for PyRangeIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.range_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyRangeIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        let index = self.index.load();
        if index < self.length {
            self.length - index
        } else {
            0
        }
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.index
            .store(range_state(&BigInt::from(self.length), state, vm)?);
        Ok(())
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        range_iter_reduce(
            BigInt::from(self.start),
            BigInt::from(self.length),
            BigInt::from(self.step),
            self.index.load(),
            vm,
        )
    }
}
impl Unconstructible for PyRangeIterator {}

impl IterNextIterable for PyRangeIterator {}
impl IterNext for PyRangeIterator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        // TODO: In pathological case (index == usize::MAX) this can wrap around
        // (since fetch_add wraps). This would result in the iterator spinning again
        // from the beginning.
        let index = zelf.index.fetch_add(1);
        let r = if index < zelf.length {
            let value = zelf.start + (index as isize) * zelf.step;
            PyIterReturn::Return(vm.ctx.new_int(value).into())
        } else {
            PyIterReturn::StopIteration(None)
        };
        Ok(r)
    }
}

fn range_iter_reduce(
    start: BigInt,
    length: BigInt,
    step: BigInt,
    index: usize,
    vm: &VirtualMachine,
) -> PyResult<PyTupleRef> {
    let iter = builtins_iter(vm).to_owned();
    let stop = start.clone() + length * step.clone();
    let range = PyRange {
        start: PyInt::from(start).into_ref(vm),
        stop: PyInt::from(stop).into_ref(vm),
        step: PyInt::from(step).into_ref(vm),
    };
    Ok(vm.new_tuple((iter, (range,), index)))
}

// Silently clips state (i.e index) in range [0, usize::MAX].
fn range_state(length: &BigInt, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
    if let Some(i) = state.payload::<PyInt>() {
        let mut index = i.as_bigint();
        let max_usize = BigInt::from(usize::MAX);
        if index > length {
            index = max(length, &max_usize);
        }
        Ok(index.to_usize().unwrap_or(0))
    } else {
        Err(vm.new_type_error("an integer is required.".to_owned()))
    }
}

pub enum RangeIndex {
    Int(PyIntRef),
    Slice(PyRef<PySlice>),
}

impl TryFromObject for RangeIndex {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match_class!(match obj {
            i @ PyInt => Ok(RangeIndex::Int(i)),
            s @ PySlice => Ok(RangeIndex::Slice(s)),
            obj => {
                let val = obj.try_index(vm).map_err(|_| vm.new_type_error(format!(
                    "sequence indices be integers or slices or classes that override __index__ operator, not '{}'",
                    obj.class().name()
                )))?;
                Ok(RangeIndex::Int(val))
            }
        })
    }
}

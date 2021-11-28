// sliceobject.{h,c} in CPython
use super::{PyInt, PyIntRef, PyTupleRef, PyTypeRef};
use crate::{
    function::{FuncArgs, IntoPyObject, OptionalArg},
    types::{Comparable, Constructor, Hashable, PyComparisonOp, Unhashable},
    PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol, VirtualMachine,
};
use num_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::ops::Range;
use std::option::Option;

#[pyclass(module = false, name = "slice")]
#[derive(Debug)]
pub struct PySlice {
    pub start: Option<PyObjectRef>,
    pub stop: PyObjectRef,
    pub step: Option<PyObjectRef>,
}

impl PyValue for PySlice {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.slice_type
    }
}

#[pyimpl(with(Hashable, Comparable))]
impl PySlice {
    #[pyproperty]
    fn start(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.start.clone().into_pyobject(vm)
    }

    fn start_ref<'a>(&'a self, vm: &'a VirtualMachine) -> &'a PyObject {
        match &self.start {
            Some(v) => v,
            None => vm.ctx.none.as_object(),
        }
    }

    #[pyproperty]
    fn stop(&self, _vm: &VirtualMachine) -> PyObjectRef {
        self.stop.clone()
    }

    #[pyproperty]
    fn step(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.step.clone().into_pyobject(vm)
    }

    fn step_ref<'a>(&'a self, vm: &'a VirtualMachine) -> &'a PyObject {
        match &self.step {
            Some(v) => v,
            None => vm.ctx.none.as_object(),
        }
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let start_repr = self.start_ref(vm).repr(vm)?;
        let stop_repr = &self.stop.repr(vm)?;
        let step_repr = self.step_ref(vm).repr(vm)?;

        Ok(format!(
            "slice({}, {}, {})",
            start_repr.as_str(),
            stop_repr.as_str(),
            step_repr.as_str()
        ))
    }

    pub fn to_saturated(&self, vm: &VirtualMachine) -> PyResult<SaturatedSlice> {
        SaturatedSlice::with_slice(self, vm)
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let slice: PySlice = match args.args.len() {
            0 => {
                return Err(
                    vm.new_type_error("slice() must have at least one arguments.".to_owned())
                );
            }
            1 => {
                let stop = args.bind(vm)?;
                PySlice {
                    start: None,
                    stop,
                    step: None,
                }
            }
            _ => {
                let (start, stop, step): (PyObjectRef, PyObjectRef, OptionalArg<PyObjectRef>) =
                    args.bind(vm)?;
                PySlice {
                    start: Some(start),
                    stop,
                    step: step.into_option(),
                }
            }
        };
        slice.into_pyresult_with_type(vm, cls)
    }

    pub(crate) fn inner_indices(
        &self,
        length: &BigInt,
        vm: &VirtualMachine,
    ) -> PyResult<(BigInt, BigInt, BigInt)> {
        // Calculate step
        let step: BigInt;
        if vm.is_none(self.step_ref(vm)) {
            step = One::one();
        } else {
            // Clone the value, not the reference.
            let this_step: PyRef<PyInt> = self.step(vm).try_into_value(vm)?;
            step = this_step.as_bigint().clone();

            if step.is_zero() {
                return Err(vm.new_value_error("slice step cannot be zero.".to_owned()));
            }
        }

        // For convenience
        let backwards = step.is_negative();

        // Each end of the array
        let lower = if backwards {
            (-1_i8).to_bigint().unwrap()
        } else {
            Zero::zero()
        };

        let upper = if backwards {
            lower.clone() + length
        } else {
            length.clone()
        };

        // Calculate start
        let mut start: BigInt;
        if vm.is_none(self.start_ref(vm)) {
            // Default
            start = if backwards {
                upper.clone()
            } else {
                lower.clone()
            };
        } else {
            let this_start: PyRef<PyInt> = self.start(vm).try_into_value(vm)?;
            start = this_start.as_bigint().clone();

            if start < Zero::zero() {
                // From end of array
                start += length;

                if start < lower {
                    start = lower.clone();
                }
            } else if start > upper {
                start = upper.clone();
            }
        }

        // Calculate Stop
        let mut stop: BigInt;
        if vm.is_none(&self.stop) {
            stop = if backwards { lower } else { upper };
        } else {
            let this_stop: PyRef<PyInt> = self.stop(vm).try_into_value(vm)?;
            stop = this_stop.as_bigint().clone();

            if stop < Zero::zero() {
                // From end of array
                stop += length;
                if stop < lower {
                    stop = lower;
                }
            } else if stop > upper {
                stop = upper;
            }
        }

        Ok((start, stop, step))
    }

    #[pymethod]
    fn indices(&self, length: PyIntRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let length = length.as_bigint();
        if length.is_negative() {
            return Err(vm.new_value_error("length should not be negative.".to_owned()));
        }
        let (start, stop, step) = self.inner_indices(length, vm)?;
        Ok(vm.new_tuple((start, stop, step)))
    }

    #[allow(clippy::type_complexity)]
    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
    ) -> PyResult<(
        PyTypeRef,
        (Option<PyObjectRef>, PyObjectRef, Option<PyObjectRef>),
    )> {
        Ok((
            zelf.clone_class(),
            (zelf.start.clone(), zelf.stop.clone(), zelf.step.clone()),
        ))
    }
}

impl Comparable for PySlice {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(Self, other);

        let ret = match op {
            PyComparisonOp::Lt | PyComparisonOp::Le => None
                .or_else(|| {
                    vm.bool_seq_lt(zelf.start_ref(vm), other.start_ref(vm))
                        .transpose()
                })
                .or_else(|| vm.bool_seq_lt(&zelf.stop, &other.stop).transpose())
                .or_else(|| {
                    vm.bool_seq_lt(zelf.step_ref(vm), other.step_ref(vm))
                        .transpose()
                })
                .unwrap_or_else(|| Ok(op == PyComparisonOp::Le))?,
            PyComparisonOp::Eq | PyComparisonOp::Ne => {
                let eq = vm.identical_or_equal(zelf.start_ref(vm), other.start_ref(vm))?
                    && vm.identical_or_equal(&zelf.stop, &other.stop)?
                    && vm.identical_or_equal(zelf.step_ref(vm), other.step_ref(vm))?;
                if op == PyComparisonOp::Ne {
                    !eq
                } else {
                    eq
                }
            }
            PyComparisonOp::Gt | PyComparisonOp::Ge => None
                .or_else(|| {
                    vm.bool_seq_gt(zelf.start_ref(vm), other.start_ref(vm))
                        .transpose()
                })
                .or_else(|| vm.bool_seq_gt(&zelf.stop, &other.stop).transpose())
                .or_else(|| {
                    vm.bool_seq_gt(zelf.step_ref(vm), other.step_ref(vm))
                        .transpose()
                })
                .unwrap_or_else(|| Ok(op == PyComparisonOp::Ge))?,
        };

        Ok(PyComparisonValue::Implemented(ret))
    }
}

impl Unhashable for PySlice {}

/// A saturated slice with values ranging in [isize::MIN, isize::MAX]. Used for
/// slicable sequences that require indices in the aforementioned range.
///
/// Invokes `__index__` on the PySliceRef during construction so as to separate the
/// transformation from PyObject into isize and the adjusting of the slice to a given
/// sequence length. The reason this is important is due to the fact that an objects
/// `__index__` might get a lock on the sequence and cause a deadlock.
#[derive(Copy, Clone, Debug)]
pub struct SaturatedSlice {
    start: isize,
    stop: isize,
    step: isize,
}

impl SaturatedSlice {
    // Equivalent to PySlice_Unpack.
    pub fn with_slice(slice: &PySlice, vm: &VirtualMachine) -> PyResult<Self> {
        let step = to_isize_index(vm, slice.step_ref(vm))?.unwrap_or(1);
        if step == 0 {
            return Err(vm.new_value_error("slice step cannot be zero".to_owned()));
        }
        let start = to_isize_index(vm, slice.start_ref(vm))?.unwrap_or_else(|| {
            if step.is_negative() {
                isize::MAX
            } else {
                0
            }
        });

        let stop = to_isize_index(vm, &slice.stop(vm))?.unwrap_or_else(|| {
            if step.is_negative() {
                isize::MIN
            } else {
                isize::MAX
            }
        });
        Ok(Self { start, stop, step })
    }

    // Equivalent to PySlice_AdjustIndices
    /// Convert for usage in indexing the underlying rust collections. Called *after*
    /// __index__ has been called on the Slice which might mutate the collection.
    pub fn adjust_indices(&self, len: usize) -> (Range<usize>, isize, usize) {
        if len == 0 {
            return (0..0, self.step, 0);
        }
        let range = if self.step.is_negative() {
            let stop = if self.stop == -1 {
                len
            } else {
                saturate_index(self.stop.saturating_add(1), len)
            };
            let start = if self.start == -1 {
                len
            } else {
                saturate_index(self.start.saturating_add(1), len)
            };
            stop..start
        } else {
            saturate_index(self.start, len)..saturate_index(self.stop, len)
        };

        let (range, slicelen) = if range.start >= range.end {
            (range.start..range.start, 0)
        } else {
            let slicelen = (range.end - range.start - 1) / self.step.unsigned_abs() + 1;
            (range, slicelen)
        };
        (range, self.step, slicelen)
    }

    pub fn iter(&self, len: usize) -> SaturatedSliceIter {
        SaturatedSliceIter::new(self, len)
    }
}

pub struct SaturatedSliceIter {
    index: isize,
    step: isize,
    len: usize,
}

impl SaturatedSliceIter {
    pub fn new(slice: &SaturatedSlice, seq_len: usize) -> Self {
        let (range, step, len) = slice.adjust_indices(seq_len);
        Self::from_adjust_indices(range, step, len)
    }

    pub fn from_adjust_indices(range: Range<usize>, step: isize, len: usize) -> Self {
        let index = if step.is_negative() {
            range.end as isize - 1
        } else {
            range.start as isize
        };
        Self { index, step, len }
    }

    pub fn positive_order(mut self) -> Self {
        if self.step.is_negative() {
            self.index += self.step * self.len.saturating_sub(1) as isize;
            self.step = self.step.saturating_abs()
        }
        self
    }
}

impl Iterator for SaturatedSliceIter {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let ret = self.index as usize;
        // SAFETY: if index is overflowed, len should be zero
        self.index = self.index.wrapping_add(self.step);
        Some(ret)
    }
}

// Go from PyObjectRef to isize w/o overflow error, out of range values are substituted by
// isize::MIN or isize::MAX depending on type and value of step.
// Equivalent to PyEval_SliceIndex.
fn to_isize_index(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Option<isize>> {
    if vm.is_none(obj) {
        return Ok(None);
    }
    let result = vm.to_index_opt(obj.to_owned()).unwrap_or_else(|| {
        Err(vm.new_type_error(
            "slice indices must be integers or None or have an __index__ method".to_owned(),
        ))
    })?;
    let value = result.as_bigint();
    let is_negative = value.is_negative();
    Ok(Some(value.to_isize().unwrap_or_else(|| {
        if is_negative {
            isize::MIN
        } else {
            isize::MAX
        }
    })))
}

// Saturate p in range [0, len] inclusive
pub fn saturate_index(p: isize, len: usize) -> usize {
    let len = len.to_isize().unwrap_or(isize::MAX);
    let mut p = p;
    if p < 0 {
        p += len;
        if p < 0 {
            p = 0;
        }
    }
    if p > len {
        p = len;
    }
    p as usize
}

#[pyclass(module = false, name = "EllipsisType")]
#[derive(Debug)]
pub struct PyEllipsis;

impl PyValue for PyEllipsis {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.ellipsis_type
    }
}

impl Constructor for PyEllipsis {
    type Args = ();

    fn py_new(_cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.ellipsis.clone().into())
    }
}

#[pyimpl(with(Constructor))]
impl PyEllipsis {
    #[pymethod(magic)]
    fn repr(&self) -> String {
        "Ellipsis".to_owned()
    }

    #[pymethod(magic)]
    fn reduce(&self) -> String {
        "Ellipsis".to_owned()
    }
}

pub fn init(context: &PyContext) {
    PySlice::extend_class(context, &context.types.slice_type);
    PyEllipsis::extend_class(context, &context.ellipsis.clone_class());
}

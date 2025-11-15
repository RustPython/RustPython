// sliceobject.{h,c} in CPython
// spell-checker:ignore sliceobject
use super::{PyGenericAlias, PyStrRef, PyTupleRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::hash::{PyHash, PyUHash},
    convert::ToPyObject,
    function::{ArgIndex, FuncArgs, OptionalArg, PyComparisonValue},
    sliceable::SaturatedSlice,
    types::{Comparable, Constructor, Hashable, PyComparisonOp, Representable},
};
use malachite_bigint::{BigInt, ToBigInt};
use num_traits::{One, Signed, Zero};

#[pyclass(module = false, name = "slice", unhashable = true, traverse)]
#[derive(Debug)]
pub struct PySlice {
    pub start: Option<PyObjectRef>,
    pub stop: PyObjectRef,
    pub step: Option<PyObjectRef>,
}

impl PyPayload for PySlice {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.slice_type
    }
}

#[pyclass(with(Comparable, Representable, Hashable))]
impl PySlice {
    #[pygetset]
    fn start(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.start.clone().to_pyobject(vm)
    }

    pub(crate) fn start_ref<'a>(&'a self, vm: &'a VirtualMachine) -> &'a PyObject {
        match &self.start {
            Some(v) => v,
            None => vm.ctx.none.as_object(),
        }
    }

    #[pygetset]
    pub(crate) fn stop(&self, _vm: &VirtualMachine) -> PyObjectRef {
        self.stop.clone()
    }

    #[pygetset]
    fn step(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.step.clone().to_pyobject(vm)
    }

    pub(crate) fn step_ref<'a>(&'a self, vm: &'a VirtualMachine) -> &'a PyObject {
        match &self.step {
            Some(v) => v,
            None => vm.ctx.none.as_object(),
        }
    }

    pub fn to_saturated(&self, vm: &VirtualMachine) -> PyResult<SaturatedSlice> {
        SaturatedSlice::with_slice(self, vm)
    }

    #[pyslot]
    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let slice: Self = match args.args.len() {
            0 => {
                return Err(vm.new_type_error("slice() must have at least one arguments."));
            }
            1 => {
                let stop = args.bind(vm)?;
                Self {
                    start: None,
                    stop,
                    step: None,
                }
            }
            _ => {
                let (start, stop, step): (PyObjectRef, PyObjectRef, OptionalArg<PyObjectRef>) =
                    args.bind(vm)?;
                Self {
                    start: Some(start),
                    stop,
                    step: step.into_option(),
                }
            }
        };
        slice.into_ref_with_type(vm, cls).map(Into::into)
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
            let this_step = self.step(vm).try_index(vm)?;
            step = this_step.as_bigint().clone();

            if step.is_zero() {
                return Err(vm.new_value_error("slice step cannot be zero."));
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
            let this_start = self.start(vm).try_index(vm)?;
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
            let this_stop = self.stop(vm).try_index(vm)?;
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
    fn indices(&self, length: ArgIndex, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let length = length.as_bigint();
        if length.is_negative() {
            return Err(vm.new_value_error("length should not be negative."));
        }
        let (start, stop, step) = self.inner_indices(length, vm)?;
        Ok(vm.new_tuple((start, stop, step)))
    }

    #[allow(clippy::type_complexity)]
    #[pymethod]
    fn __reduce__(
        zelf: PyRef<Self>,
    ) -> PyResult<(
        PyTypeRef,
        (Option<PyObjectRef>, PyObjectRef, Option<PyObjectRef>),
    )> {
        Ok((
            zelf.class().to_owned(),
            (zelf.start.clone(), zelf.stop.clone(), zelf.step.clone()),
        ))
    }

    // TODO: Uncomment when Python adds __class_getitem__ to slice
    // #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

impl Hashable for PySlice {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        const XXPRIME_1: PyUHash = if cfg!(target_pointer_width = "64") {
            11400714785074694791
        } else {
            2654435761
        };
        const XXPRIME_2: PyUHash = if cfg!(target_pointer_width = "64") {
            14029467366897019727
        } else {
            2246822519
        };
        const XXPRIME_5: PyUHash = if cfg!(target_pointer_width = "64") {
            2870177450012600261
        } else {
            374761393
        };
        const ROTATE: u32 = if cfg!(target_pointer_width = "64") {
            31
        } else {
            13
        };

        let mut acc = XXPRIME_5;
        for part in &[zelf.start_ref(vm), &zelf.stop, zelf.step_ref(vm)] {
            let lane = part.hash(vm)? as PyUHash;
            if lane == u64::MAX as PyUHash {
                return Ok(-1 as PyHash);
            }
            acc = acc.wrapping_add(lane.wrapping_mul(XXPRIME_2));
            acc = acc.rotate_left(ROTATE);
            acc = acc.wrapping_mul(XXPRIME_1);
        }
        if acc == u64::MAX as PyUHash {
            return Ok(1546275796 as PyHash);
        }
        Ok(acc as PyHash)
    }
}

impl Comparable for PySlice {
    fn cmp(
        zelf: &Py<Self>,
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
                if op == PyComparisonOp::Ne { !eq } else { eq }
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

impl Representable for PySlice {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let start_repr = zelf.start_ref(vm).repr(vm)?;
        let stop_repr = zelf.stop.repr(vm)?;
        let step_repr = zelf.step_ref(vm).repr(vm)?;

        Ok(format!("slice({start_repr}, {stop_repr}, {step_repr})"))
    }
}

#[pyclass(module = false, name = "EllipsisType")]
#[derive(Debug)]
pub struct PyEllipsis;

impl PyPayload for PyEllipsis {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.ellipsis_type
    }
}

impl Constructor for PyEllipsis {
    type Args = ();

    fn py_new(_cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.ellipsis.clone().into())
    }
}

#[pyclass(with(Constructor, Representable))]
impl PyEllipsis {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.names.Ellipsis.to_owned()
    }
}

impl Representable for PyEllipsis {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(vm.ctx.names.Ellipsis.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub fn init(ctx: &Context) {
    PySlice::extend_class(ctx, ctx.types.slice_type);
    PyEllipsis::extend_class(ctx, ctx.types.ellipsis_type);
}

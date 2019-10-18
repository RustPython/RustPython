use num_bigint::BigInt;

use super::objint::PyInt;
use super::objtype::{class_has_attr, PyClassRef};
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[pyclass]
#[derive(Debug)]
pub struct PySlice {
    pub start: Option<PyObjectRef>,
    pub stop: PyObjectRef,
    pub step: Option<PyObjectRef>,
}

impl PyValue for PySlice {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.slice_type()
    }
}

pub type PySliceRef = PyRef<PySlice>;

fn get_property_value(vm: &VirtualMachine, value: &Option<PyObjectRef>) -> PyObjectRef {
    if let Some(value) = value {
        value.clone()
    } else {
        vm.get_none()
    }
}

#[pyimpl]
impl PySlice {
    #[pyproperty(name = "start")]
    fn start(&self, vm: &VirtualMachine) -> PyObjectRef {
        get_property_value(vm, &self.start)
    }

    #[pyproperty(name = "stop")]
    fn stop(&self, _vm: &VirtualMachine) -> PyObjectRef {
        self.stop.clone()
    }

    #[pyproperty(name = "step")]
    fn step(&self, vm: &VirtualMachine) -> PyObjectRef {
        get_property_value(vm, &self.step)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> PyResult<String> {
        let start = self.start(_vm);
        let stop = self.stop(_vm);
        let step = self.step(_vm);

        let start_repr = _vm.to_repr(&start)?;
        let stop_repr = _vm.to_repr(&stop)?;
        let step_repr = _vm.to_repr(&step)?;

        Ok(format!(
            "slice({}, {}, {})",
            start_repr.as_str(),
            stop_repr.as_str(),
            step_repr.as_str()
        ))
    }

    pub fn start_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        if let Some(obj) = &self.start {
            to_index_value(vm, obj)
        } else {
            Ok(None)
        }
    }

    pub fn stop_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        to_index_value(vm, &self.stop)
    }

    pub fn step_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        if let Some(obj) = &self.step {
            to_index_value(vm, obj)
        } else {
            Ok(None)
        }
    }

    #[pyslot(new)]
    fn tp_new(cls: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PySliceRef> {
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
        slice.into_ref_with_type(vm, cls)
    }

    fn inner_eq(&self, other: &PySlice, vm: &VirtualMachine) -> PyResult<bool> {
        if !vm.identical_or_equal(&self.start(vm), &other.start(vm))? {
            return Ok(false);
        }
        if !vm.identical_or_equal(&self.stop(vm), &other.stop(vm))? {
            return Ok(false);
        }
        if !vm.identical_or_equal(&self.step(vm), &other.step(vm))? {
            return Ok(false);
        }
        Ok(true)
    }

    #[inline]
    fn inner_lte(&self, other: &PySlice, eq: bool, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(v) = vm.bool_seq_lt(self.start(vm), other.start(vm))? {
            return Ok(v);
        }
        if let Some(v) = vm.bool_seq_lt(self.stop(vm), other.stop(vm))? {
            return Ok(v);
        }
        if let Some(v) = vm.bool_seq_lt(self.step(vm), other.step(vm))? {
            return Ok(v);
        }
        Ok(eq)
    }

    #[inline]
    fn inner_gte(&self, other: &PySlice, eq: bool, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(v) = vm.bool_seq_gt(self.start(vm), other.start(vm))? {
            return Ok(v);
        }
        if let Some(v) = vm.bool_seq_gt(self.stop(vm), other.stop(vm))? {
            return Ok(v);
        }
        if let Some(v) = vm.bool_seq_gt(self.step(vm), other.step(vm))? {
            return Ok(v);
        }
        Ok(eq)
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let eq = self.inner_eq(rhs, vm)?;
            Ok(vm.ctx.new_bool(eq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let eq = self.inner_eq(rhs, vm)?;
            Ok(vm.ctx.new_bool(!eq))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let lt = self.inner_lte(rhs, false, vm)?;
            Ok(vm.ctx.new_bool(lt))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let gt = self.inner_gte(rhs, false, vm)?;
            Ok(vm.ctx.new_bool(gt))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let ge = self.inner_gte(rhs, true, vm)?;
            Ok(vm.ctx.new_bool(ge))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(rhs) = rhs.payload::<PySlice>() {
            let le = self.inner_lte(rhs, true, vm)?;
            Ok(vm.ctx.new_bool(le))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type".to_string()))
    }
}

fn to_index_value(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Option<BigInt>> {
    if obj.is(&vm.ctx.none) {
        return Ok(None);
    }

    if let Some(val) = obj.payload::<PyInt>() {
        Ok(Some(val.as_bigint().clone()))
    } else {
        let cls = obj.class();
        if class_has_attr(&cls, "__index__") {
            let index_result = vm.call_method(obj, "__index__", vec![])?;
            if let Some(val) = index_result.payload::<PyInt>() {
                Ok(Some(val.as_bigint().clone()))
            } else {
                Err(vm.new_type_error("__index__ method returned non integer".to_string()))
            }
        } else {
            Err(vm.new_type_error(
                "slice indices must be integers or None or have an __index__ method".to_string(),
            ))
        }
    }
}

pub fn init(context: &PyContext) {
    let slice_type = &context.types.slice_type;
    PySlice::extend_class(context, slice_type);
}

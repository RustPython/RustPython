use crate::function::PyFuncArgs;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use crate::obj::objint::PyInt;
use crate::obj::objtype::{class_has_attr, PyClassRef};
use num_bigint::BigInt;

#[derive(Debug, FromArgs)]
pub struct PySlice {
    #[pyarg(positional_only)]
    pub start: Option<PyObjectRef>,
    #[pyarg(positional_only)]
    pub stop: Option<PyObjectRef>,
    #[pyarg(positional_only, default)]
    pub step: Option<PyObjectRef>,
}

impl PyValue for PySlice {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.slice_type()
    }
}

pub type PySliceRef = PyRef<PySlice>;

fn slice_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let (cls, slice): (PyClassRef, PySlice) = match args.args.len() {
        0 | 1 => {
            return Err(vm.new_type_error("slice() must have at least one arguments.".to_owned()));
        }
        2 => {
            let (cls, stop) = args.bind(vm)?;
            (
                cls,
                PySlice {
                    start: None,
                    stop: Some(stop),
                    step: None,
                },
            )
        }
        _ => args.bind(vm)?,
    };
    slice.into_ref_with_type(vm, cls).map(|x| x.into_object())
}

fn get_property_value(vm: &VirtualMachine, value: &Option<PyObjectRef>) -> PyObjectRef {
    if let Some(value) = value {
        value.clone()
    } else {
        vm.get_none()
    }
}

impl PySliceRef {
    fn start(self, vm: &VirtualMachine) -> PyObjectRef {
        get_property_value(vm, &self.start)
    }

    fn stop(self, vm: &VirtualMachine) -> PyObjectRef {
        get_property_value(vm, &self.stop)
    }

    fn step(self, vm: &VirtualMachine) -> PyObjectRef {
        get_property_value(vm, &self.step)
    }

    pub fn start_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        if let Some(obj) = &self.start {
            to_index_value(vm, obj)
        } else {
            Ok(None)
        }
    }

    pub fn stop_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        if let Some(obj) = &self.stop {
            to_index_value(vm, obj)
        } else {
            Ok(None)
        }
    }

    pub fn step_index(&self, vm: &VirtualMachine) -> PyResult<Option<BigInt>> {
        if let Some(obj) = &self.step {
            to_index_value(vm, obj)
        } else {
            Ok(None)
        }
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
    let slice_type = &context.slice_type;

    extend_class!(context, slice_type, {
        "__new__" => context.new_rustfunc(slice_new),
        "start" => context.new_property(PySliceRef::start),
        "stop" => context.new_property(PySliceRef::stop),
        "step" => context.new_property(PySliceRef::step)
    });
}

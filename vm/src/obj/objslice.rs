use num_bigint::BigInt;

use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objint;
use crate::obj::objtype::PyClassRef;

#[derive(Debug)]
pub struct PySlice {
    // TODO: should be private
    pub start: Option<BigInt>,
    pub stop: Option<BigInt>,
    pub step: Option<BigInt>,
}

impl PyValue for PySlice {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.slice_type()
    }
}

pub type PySliceRef = PyRef<PySlice>;

fn slice_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    no_kwargs!(vm, args);
    let (cls, start, stop, step): (
        &PyObjectRef,
        Option<&PyObjectRef>,
        Option<&PyObjectRef>,
        Option<&PyObjectRef>,
    ) = match args.args.len() {
        0 | 1 => Err(vm.new_type_error("slice() must have at least one arguments.".to_owned())),
        2 => {
            arg_check!(
                vm,
                args,
                required = [
                    (cls, Some(vm.ctx.type_type())),
                    (stop, Some(vm.ctx.int_type()))
                ]
            );
            Ok((cls, None, Some(stop), None))
        }
        _ => {
            arg_check!(
                vm,
                args,
                required = [
                    (cls, Some(vm.ctx.type_type())),
                    (start, Some(vm.ctx.int_type())),
                    (stop, Some(vm.ctx.int_type()))
                ],
                optional = [(step, Some(vm.ctx.int_type()))]
            );
            Ok((cls, Some(start), Some(stop), step))
        }
    }?;
    PySlice {
        start: start.map(|x| objint::get_value(x).clone()),
        stop: stop.map(|x| objint::get_value(x).clone()),
        step: step.map(|x| objint::get_value(x).clone()),
    }
    .into_ref_with_type(vm, cls.clone().downcast().unwrap())
    .map(PyRef::into_object)
}

fn get_property_value(vm: &VirtualMachine, value: &Option<BigInt>) -> PyObjectRef {
    if let Some(value) = value {
        vm.ctx.new_int(value.clone())
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

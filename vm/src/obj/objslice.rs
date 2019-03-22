use num_bigint::BigInt;

use crate::function::PyFuncArgs;
use crate::pyobject::{PyContext, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

use super::objint;

#[derive(Debug)]
pub struct PySlice {
    // TODO: should be private
    pub start: Option<BigInt>,
    pub stop: Option<BigInt>,
    pub step: Option<BigInt>,
}

impl PyValue for PySlice {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.slice_type()]
    }
}

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
    Ok(PyObject::new(
        PySlice {
            start: start.map(|x| objint::get_value(x).clone()),
            stop: stop.map(|x| objint::get_value(x).clone()),
            step: step.map(|x| objint::get_value(x).clone()),
        },
        cls.clone(),
    ))
}

fn get_property_value(vm: &VirtualMachine, value: &Option<BigInt>) -> PyResult {
    if let Some(value) = value {
        Ok(vm.ctx.new_int(value.clone()))
    } else {
        Ok(vm.get_none())
    }
}

fn slice_start(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(slice, Some(vm.ctx.slice_type()))]);
    if let Some(PySlice { start, .. }) = &slice.payload() {
        get_property_value(vm, start)
    } else {
        panic!("Slice has incorrect payload.");
    }
}

fn slice_stop(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(slice, Some(vm.ctx.slice_type()))]);
    if let Some(PySlice { stop, .. }) = &slice.payload() {
        get_property_value(vm, stop)
    } else {
        panic!("Slice has incorrect payload.");
    }
}

fn slice_step(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(slice, Some(vm.ctx.slice_type()))]);
    if let Some(PySlice { step, .. }) = &slice.payload() {
        get_property_value(vm, step)
    } else {
        panic!("Slice has incorrect payload.");
    }
}

pub fn init(context: &PyContext) {
    let zip_type = &context.slice_type;

    context.set_attr(zip_type, "__new__", context.new_rustfunc(slice_new));
    context.set_attr(zip_type, "start", context.new_property(slice_start));
    context.set_attr(zip_type, "stop", context.new_property(slice_stop));
    context.set_attr(zip_type, "step", context.new_property(slice_step));
}

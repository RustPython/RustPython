/*
 * Various types to support iteration.
 */

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objstr;
use super::objtype; // Required for arg_check! to use isinstance

pub fn get_iter(vm: &mut VirtualMachine, iter_target: &PyObjectRef) -> PyResult {
    // Check what we are going to iterate over:
    let iterated_obj = if objtype::isinstance(iter_target, vm.ctx.iter_type()) {
        // If object is already an iterator, return that one.
        return Ok(iter_target.clone())
    } else if objtype::isinstance(iter_target, vm.ctx.list_type()) {
        iter_target.clone()
    } else {
        let type_str = objstr::get_value(&vm.to_str(iter_target.typ()).unwrap());
        let type_error = vm.new_type_error(format!("Cannot iterate over {}", type_str));
        return Err(type_error);
    };

    let iter_obj = PyObject::new(
        PyObjectKind::Iterator {
            position: 0,
            iterated_obj: iterated_obj,
        },
        vm.ctx.iter_type(),
    );

    // We are all good here:
    Ok(iter_obj)
}

// Sequence iterator:
fn iter_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iter_target, None)]
    );

    get_iter(vm, iter_target)
}

fn iter_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iter, Some(vm.ctx.iter_type()))]
    );
    // Return self:
    Ok(iter.clone())
}

fn iter_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iter, Some(vm.ctx.iter_type()))]
    );

    let next_obj: Option<PyObjectRef> = {
        // We require a mutable pyobject here to update the iterator:
        let mut iterator: &mut PyObject = &mut iter.borrow_mut();
        iterator.nxt()
    };

    // Return next item, or StopIteration
    match next_obj {
        Some(value) => Ok(value),
        None => {
            let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
            let stop_iteration = vm.new_exception(stop_iteration_type, "End of iterator".to_string());
            Err(stop_iteration)
        }
    }
}

pub fn init(context: &PyContext) {
    let ref iter_type = context.iter_type;
    iter_type.set_attr("__new__", context.new_rustfunc(iter_new));
    iter_type.set_attr("__iter__", context.new_rustfunc(iter_iter));
    iter_type.set_attr("__next__", context.new_rustfunc(iter_next));
}


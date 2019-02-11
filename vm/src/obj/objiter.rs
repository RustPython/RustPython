/*
 * Various types to support iteration.
 */

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
// use super::objstr;
use super::objtype; // Required for arg_check! to use isinstance

/*
 * This helper function is called at multiple places. First, it is called
 * in the vm when a for loop is entered. Next, it is used when the builtin
 * function 'iter' is called.
 */
pub fn get_iter(vm: &mut VirtualMachine, iter_target: &PyObjectRef) -> PyResult {
    vm.call_method(iter_target, "__iter__", vec![])
    // let type_str = objstr::get_value(&vm.to_str(iter_target.typ()).unwrap());
    // let type_error = vm.new_type_error(format!("Cannot iterate over {}", type_str));
    // return Err(type_error);
}

pub fn call_next(vm: &mut VirtualMachine, iter_obj: &PyObjectRef) -> PyResult {
    vm.call_method(iter_obj, "__next__", vec![])
}

/*
 * Helper function to retrieve the next object (or none) from an iterator.
 */
pub fn get_next_object(
    vm: &mut VirtualMachine,
    iter_obj: &PyObjectRef,
) -> Result<Option<PyObjectRef>, PyObjectRef> {
    let next_obj: PyResult = call_next(vm, iter_obj);

    match next_obj {
        Ok(value) => Ok(Some(value)),
        Err(next_error) => {
            // Check if we have stopiteration, or something else:
            if objtype::isinstance(&next_error, &vm.ctx.exceptions.stop_iteration) {
                Ok(None)
            } else {
                Err(next_error)
            }
        }
    }
}

/* Retrieve all elements from an iterator */
pub fn get_all(
    vm: &mut VirtualMachine,
    iter_obj: &PyObjectRef,
) -> Result<Vec<PyObjectRef>, PyObjectRef> {
    let mut elements = vec![];
    loop {
        let element = get_next_object(vm, iter_obj)?;
        match element {
            Some(v) => elements.push(v),
            None => break,
        }
    }
    Ok(elements)
}

pub fn new_stop_iteration(vm: &mut VirtualMachine) -> PyObjectRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception(stop_iteration_type, "End of iterator".to_string())
}

fn contains(vm: &mut VirtualMachine, args: PyFuncArgs, iter_type: PyObjectRef) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iter, Some(iter_type)), (needle, None)]
    );
    loop {
        if let Some(element) = get_next_object(vm, iter)? {
            let equal = vm._eq(needle.clone(), element.clone())?;
            if objbool::get_value(&equal) {
                return Ok(vm.new_bool(true));
            } else {
                continue;
            }
        } else {
            return Ok(vm.new_bool(false));
        }
    }
}

/// Common setup for iter types, adds __iter__ and __contains__ methods
pub fn iter_type_init(context: &PyContext, iter_type: &PyObjectRef) {
    let contains_func = {
        let cloned_iter_type = iter_type.clone();
        move |vm: &mut VirtualMachine, args: PyFuncArgs| {
            contains(vm, args, cloned_iter_type.clone())
        }
    };
    context.set_attr(
        &iter_type,
        "__contains__",
        context.new_rustfunc(contains_func),
    );
    let iter_func = {
        let cloned_iter_type = iter_type.clone();
        move |vm: &mut VirtualMachine, args: PyFuncArgs| {
            arg_check!(
                vm,
                args,
                required = [(iter, Some(cloned_iter_type.clone()))]
            );
            // Return self:
            Ok(iter.clone())
        }
    };
    context.set_attr(&iter_type, "__iter__", context.new_rustfunc(iter_func));
}

// Sequence iterator:
fn iter_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter_target, None)]);

    get_iter(vm, iter_target)
}

fn iter_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter, Some(vm.ctx.iter_type()))]);

    if let PyObjectPayload::Iterator {
        ref mut position,
        iterated_obj: ref mut iterated_obj_ref,
    } = iter.borrow_mut().payload
    {
        let iterated_obj = iterated_obj_ref.borrow_mut();
        match iterated_obj.payload {
            PyObjectPayload::Sequence { ref elements } => {
                if *position < elements.len() {
                    let obj_ref = elements[*position].clone();
                    *position += 1;
                    Ok(obj_ref)
                } else {
                    Err(new_stop_iteration(vm))
                }
            }

            PyObjectPayload::Range { ref range } => {
                if let Some(int) = range.get(*position) {
                    *position += 1;
                    Ok(vm.ctx.new_int(int))
                } else {
                    Err(new_stop_iteration(vm))
                }
            }

            PyObjectPayload::Bytes { ref value } => {
                if *position < value.len() {
                    let obj_ref = vm.ctx.new_int(value[*position]);
                    *position += 1;
                    Ok(obj_ref)
                } else {
                    Err(new_stop_iteration(vm))
                }
            }

            _ => {
                panic!("NOT IMPL");
            }
        }
    } else {
        panic!("NOT IMPL");
    }
}

pub fn init(context: &PyContext) {
    let iter_type = &context.iter_type;

    let iter_doc = "iter(iterable) -> iterator\n\
                    iter(callable, sentinel) -> iterator\n\n\
                    Get an iterator from an object.  In the first form, the argument must\n\
                    supply its own iterator, or be a sequence.\n\
                    In the second form, the callable is called until it returns the sentinel.";

    iter_type_init(context, iter_type);
    context.set_attr(&iter_type, "__new__", context.new_rustfunc(iter_new));
    context.set_attr(&iter_type, "__next__", context.new_rustfunc(iter_next));
    context.set_attr(&iter_type, "__doc__", context.new_str(iter_doc.to_string()));
}

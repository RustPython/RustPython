/*
 * Various types to support iteration.
 */

use super::super::pyobject::{
    AttributeProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
use super::objstr;
use super::objtype; // Required for arg_check! to use isinstance

/*
 * This helper function is called at multiple places. First, it is called
 * in the vm when a for loop is entered. Next, it is used when the builtin
 * function 'iter' is called.
 */
pub fn get_iter(vm: &mut VirtualMachine, iter_target: &PyObjectRef) -> PyResult {
    // Check what we are going to iterate over:
    let iterated_obj = if objtype::isinstance(iter_target, vm.ctx.iter_type()) {
        // If object is already an iterator, return that one.
        return Ok(iter_target.clone());
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
    arg_check!(vm, args, required = [(iter_target, None)]);

    get_iter(vm, iter_target)
}

fn iter_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter, Some(vm.ctx.iter_type()))]);
    // Return self:
    Ok(iter.clone())
}

fn iter_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iter, Some(vm.ctx.iter_type())), (needle, None)]
    );
    loop {
        match vm.call_method(&iter, "__next__", vec![]) {
            Ok(element) => match vm.call_method(needle, "__eq__", vec![element.clone()]) {
                Ok(value) => {
                    if objbool::get_value(&value) {
                        return Ok(vm.new_bool(true));
                    } else {
                        continue;
                    }
                }
                Err(_) => return Err(vm.new_type_error("".to_string())),
            },
            Err(_) => return Ok(vm.new_bool(false)),
        }
    }
}

fn iter_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter, Some(vm.ctx.iter_type()))]);

    if let PyObjectKind::Iterator {
        ref mut position,
        iterated_obj: ref iterated_obj_ref,
    } = iter.borrow_mut().kind
    {
        let iterated_obj = &*iterated_obj_ref.borrow_mut();
        match iterated_obj.kind {
            PyObjectKind::List { ref elements } => {
                if *position < elements.len() {
                    let obj_ref = elements[*position].clone();
                    *position += 1;
                    Ok(obj_ref)
                } else {
                    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
                    let stop_iteration =
                        vm.new_exception(stop_iteration_type, "End of iterator".to_string());
                    Err(stop_iteration)
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
    let ref iter_type = context.iter_type;
    iter_type.set_attr("__contains__", context.new_rustfunc(iter_contains));
    iter_type.set_attr("__iter__", context.new_rustfunc(iter_iter));
    iter_type.set_attr("__new__", context.new_rustfunc(iter_new));
    iter_type.set_attr("__next__", context.new_rustfunc(iter_next));
}

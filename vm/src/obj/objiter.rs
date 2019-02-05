/*
 * Various types to support iteration.
 */

use super::super::pyobject::{
    IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objbool;
// use super::objstr;
use super::objtype; // Required for arg_check! to use isinstance
use num_bigint::{BigInt, ToBigInt};

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

// Should filter/map have their own class?
pub fn create_filter(
    vm: &mut VirtualMachine,
    predicate: &PyObjectRef,
    iterable: &PyObjectRef,
) -> PyResult {
    let iterator = get_iter(vm, iterable)?;
    let iter_obj = PyObject::new(
        PyObjectPayload::FilterIterator {
            predicate: predicate.clone(),
            iterator,
        },
        vm.ctx.iter_type(),
    );

    Ok(iter_obj)
}

pub fn create_map(
    vm: &mut VirtualMachine,
    mapper: &PyObjectRef,
    iterables: &[PyObjectRef],
) -> PyResult {
    let iterators = iterables
        .into_iter()
        .map(|iterable| get_iter(vm, iterable))
        .collect::<Result<Vec<_>, _>>()?;
    let iter_obj = PyObject::new(
        PyObjectPayload::MapIterator {
            mapper: mapper.clone(),
            iterators,
        },
        vm.ctx.iter_type(),
    );

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

    let ref mut payload = iter.borrow_mut().payload;
    match payload {
        PyObjectPayload::Iterator {
            ref mut position,
            iterated_obj: ref mut iterated_obj_ref,
        } => {
            let iterated_obj = iterated_obj_ref.borrow_mut();
            match iterated_obj.payload {
                PyObjectPayload::Sequence { ref elements } => {
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

                PyObjectPayload::Range { ref range } => {
                    if let Some(int) = range.get(BigInt::from(*position)) {
                        *position += 1;
                        Ok(vm.ctx.new_int(int.to_bigint().unwrap()))
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
        }
        PyObjectPayload::FilterIterator {
            ref mut predicate,
            ref mut iterator,
        } => {
            loop {
                let next_obj = call_next(vm, iterator)?;
                let predicate_value = if predicate.is(&vm.get_none()) {
                    next_obj.clone()
                } else {
                    // the predicate itself can raise StopIteration which does stop the filter
                    // iteration
                    vm.invoke(
                        predicate.clone(),
                        PyFuncArgs {
                            args: vec![next_obj.clone()],
                            kwargs: vec![],
                        },
                    )?
                };
                if objbool::boolval(vm, predicate_value)? {
                    return Ok(next_obj);
                }
            }
        }
        PyObjectPayload::MapIterator {
            ref mut mapper,
            ref mut iterators,
        } => {
            let next_objs = iterators
                .iter()
                .map(|iterator| call_next(vm, iterator))
                .collect::<Result<Vec<_>, _>>()?;

            // the mapper itself can raise StopIteration which does stop the map iteration
            vm.invoke(
                mapper.clone(),
                PyFuncArgs {
                    args: next_objs,
                    kwargs: vec![],
                },
            )
        }
        _ => {
            panic!("NOT IMPL");
        }
    }
}

pub fn init(context: &PyContext) {
    let iter_type = &context.iter_type;
    context.set_attr(
        &iter_type,
        "__contains__",
        context.new_rustfunc(iter_contains),
    );
    context.set_attr(&iter_type, "__iter__", context.new_rustfunc(iter_iter));
    context.set_attr(&iter_type, "__new__", context.new_rustfunc(iter_new));
    context.set_attr(&iter_type, "__next__", context.new_rustfunc(iter_next));
}

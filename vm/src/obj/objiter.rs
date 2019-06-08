/*
 * Various types to support iteration.
 */

use std::cell::Cell;

use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objtype;
use super::objtype::PyClassRef;

/*
 * This helper function is called at multiple places. First, it is called
 * in the vm when a for loop is entered. Next, it is used when the builtin
 * function 'iter' is called.
 */
pub fn get_iter(vm: &VirtualMachine, iter_target: &PyObjectRef) -> PyResult {
    if let Ok(method) = vm.get_method(iter_target.clone(), "__iter__") {
        vm.invoke(method, vec![])
    } else if vm.get_method(iter_target.clone(), "__getitem__").is_ok() {
        Ok(PySequenceIterator {
            position: Cell::new(0),
            obj: iter_target.clone(),
        }
        .into_ref(vm)
        .into_object())
    } else {
        let message = format!("Cannot iterate over {}", iter_target.class().name);
        return Err(vm.new_type_error(message));
    }
}

pub fn call_next(vm: &VirtualMachine, iter_obj: &PyObjectRef) -> PyResult {
    vm.call_method(iter_obj, "__next__", vec![])
}

/*
 * Helper function to retrieve the next object (or none) from an iterator.
 */
pub fn get_next_object(
    vm: &VirtualMachine,
    iter_obj: &PyObjectRef,
) -> PyResult<Option<PyObjectRef>> {
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
pub fn get_all(vm: &VirtualMachine, iter_obj: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
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

pub fn new_stop_iteration(vm: &VirtualMachine) -> PyObjectRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception(stop_iteration_type, "End of iterator".to_string())
}

#[pyclass]
#[derive(Debug)]
pub struct PySequenceIterator {
    pub position: Cell<usize>,
    pub obj: PyObjectRef,
}

impl PyValue for PySequenceIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.iter_type()
    }
}

#[pyimpl]
impl PySequenceIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let number = vm.ctx.new_int(self.position.get());
        match vm.call_method(&self.obj, "__getitem__", vec![number]) {
            Ok(val) => {
                self.position.set(self.position.get() + 1);
                Ok(val)
            }
            Err(ref e) if objtype::isinstance(&e, &vm.ctx.exceptions.index_error) => {
                Err(new_stop_iteration(vm))
            }
            // also catches stop_iteration => stop_iteration
            Err(e) => Err(e),
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.iter_type);
}

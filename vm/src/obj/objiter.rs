/*
 * Various types to support iteration.
 */

use std::cell::Cell;

use super::objtuple::PyTuple;
use super::objtype::{self, PyClassRef};
use crate::pyobject::{
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

/*
 * This helper function is called at multiple places. First, it is called
 * in the vm when a for loop is entered. Next, it is used when the builtin
 * function 'iter' is called.
 */
pub fn get_iter(vm: &VirtualMachine, iter_target: &PyObjectRef) -> PyResult {
    if let Some(method_or_err) = vm.get_method(iter_target.clone(), "__iter__") {
        let method = method_or_err?;
        vm.invoke(&method, vec![])
    } else {
        vm.get_method_or_type_error(iter_target.clone(), "__getitem__", || {
            format!("Cannot iterate over {}", iter_target.class().name)
        })?;
        let obj_iterator = PySequenceIterator {
            position: Cell::new(0),
            obj: iter_target.clone(),
            reversed: false,
        };
        Ok(obj_iterator.into_ref(vm).into_object())
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
pub fn get_all<T: TryFromObject>(vm: &VirtualMachine, iter_obj: &PyObjectRef) -> PyResult<Vec<T>> {
    let mut elements = vec![];
    while let Some(element) = get_next_object(vm, iter_obj)? {
        elements.push(T::try_from_object(vm, element)?);
    }
    Ok(elements)
}

pub fn new_stop_iteration(vm: &VirtualMachine) -> PyObjectRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception(stop_iteration_type, "End of iterator".to_string())
}

pub fn stop_iter_value(vm: &VirtualMachine, exc: &PyObjectRef) -> PyResult {
    let args = vm.get_attribute(exc.clone(), "args")?;
    let args: &PyTuple = args.payload().unwrap();
    let val = args
        .elements
        .first()
        .cloned()
        .unwrap_or_else(|| vm.get_none());
    Ok(val)
}

#[pyclass]
#[derive(Debug)]
pub struct PySequenceIterator {
    pub position: Cell<isize>,
    pub obj: PyObjectRef,
    pub reversed: bool,
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
        if self.position.get() >= 0 {
            let step: isize = if self.reversed { -1 } else { 1 };
            let number = vm.ctx.new_int(self.position.get());
            match vm.call_method(&self.obj, "__getitem__", vec![number]) {
                Ok(val) => {
                    self.position.set(self.position.get() + step);
                    Ok(val)
                }
                Err(ref e) if objtype::isinstance(&e, &vm.ctx.exceptions.index_error) => {
                    Err(new_stop_iteration(vm))
                }
                // also catches stop_iteration => stop_iteration
                Err(e) => Err(e),
            }
        } else {
            Err(new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.types.iter_type);
}

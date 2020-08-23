/*
 * Various types to support iteration.
 */

use crossbeam_utils::atomic::AtomicCell;
use num_traits::{Signed, ToPrimitive};

use super::objint::PyInt;
use super::objsequence;
use super::objtype::{self, PyClassRef};
use crate::exceptions::PyBaseExceptionRef;
use crate::pyobject::{
    BorrowValue, PyCallable, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
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
        let iter = vm.invoke(&method, vec![])?;
        if iter.has_class_attr("__next__") {
            Ok(iter)
        } else {
            Err(vm.new_type_error(format!(
                "iter() returned non-iterator of type '{}'",
                iter.lease_class().name
            )))
        }
    } else {
        vm.get_method_or_type_error(iter_target.clone(), "__getitem__", || {
            format!(
                "'{}' object is not iterable",
                iter_target.lease_class().name
            )
        })?;
        Ok(PySequenceIterator::new_forward(iter_target.clone())
            .into_ref(vm)
            .into_object())
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
    let cap = length_hint(vm, iter_obj.clone())?.unwrap_or(0);
    // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
    // https://github.com/python/cpython/blob/master/Objects/listobject.c#L934-L940
    if cap >= isize::max_value() as usize {
        return Ok(Vec::new());
    }
    let mut elements = Vec::with_capacity(cap);
    while let Some(element) = get_next_object(vm, iter_obj)? {
        elements.push(T::try_from_object(vm, element)?);
    }
    elements.shrink_to_fit();
    Ok(elements)
}

pub fn new_stop_iteration(vm: &VirtualMachine) -> PyBaseExceptionRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception_empty(stop_iteration_type)
}
pub fn stop_iter_with_value(val: PyObjectRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception(stop_iteration_type, vec![val])
}

pub fn stop_iter_value(vm: &VirtualMachine, exc: &PyBaseExceptionRef) -> PyResult {
    let args = exc.args();
    let val = args
        .borrow_value()
        .first()
        .cloned()
        .unwrap_or_else(|| vm.get_none());
    Ok(val)
}

pub fn length_hint(vm: &VirtualMachine, iter: PyObjectRef) -> PyResult<Option<usize>> {
    if let Some(len) = objsequence::opt_len(&iter, vm) {
        match len {
            Ok(len) => return Ok(Some(len)),
            Err(e) => {
                if !objtype::isinstance(&e, &vm.ctx.exceptions.type_error) {
                    return Err(e);
                }
            }
        }
    }
    let hint = match vm.get_method(iter, "__length_hint__") {
        Some(hint) => hint?,
        None => return Ok(None),
    };
    let result = match vm.invoke(&hint, vec![]) {
        Ok(res) => res,
        Err(e) => {
            return if objtype::isinstance(&e, &vm.ctx.exceptions.type_error) {
                Ok(None)
            } else {
                Err(e)
            }
        }
    };
    let result = result
        .payload_if_subclass::<PyInt>(vm)
        .ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                result.lease_class().name
            ))
        })?
        .borrow_value();
    if result.is_negative() {
        return Err(vm.new_value_error("__length_hint__() should return >= 0".to_owned()));
    }
    let hint = result.to_usize().ok_or_else(|| {
        vm.new_value_error("Python int too large to convert to Rust usize".to_owned())
    })?;
    Ok(Some(hint))
}

#[pyclass(module = false, name = "iter")]
#[derive(Debug)]
pub struct PySequenceIterator {
    pub position: AtomicCell<isize>,
    pub obj: PyObjectRef,
    pub reversed: bool,
}

impl PyValue for PySequenceIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.iter_type.clone()
    }
}

#[pyimpl]
impl PySequenceIterator {
    pub fn new_forward(obj: PyObjectRef) -> Self {
        Self {
            position: AtomicCell::new(0),
            obj,
            reversed: false,
        }
    }

    pub fn new_reversed(obj: PyObjectRef, len: isize) -> Self {
        Self {
            position: AtomicCell::new(len - 1),
            obj,
            reversed: true,
        }
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let step: isize = if self.reversed { -1 } else { 1 };
        let pos = self.position.fetch_add(step);
        if pos >= 0 {
            match vm.call_method(&self.obj, "__getitem__", vec![vm.ctx.new_int(pos)]) {
                Err(ref e) if objtype::isinstance(&e, &vm.ctx.exceptions.index_error) => {
                    Err(new_stop_iteration(vm))
                }
                // also catches stop_iteration => stop_iteration
                ret => ret,
            }
        } else {
            Err(new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }

    #[pymethod(name = "__length_hint__")]
    fn length_hint(&self, vm: &VirtualMachine) -> PyResult<isize> {
        let pos = self.position.load();
        let hint = if self.reversed {
            pos + 1
        } else {
            let len = objsequence::opt_len(&self.obj, vm).unwrap_or_else(|| {
                Err(vm.new_type_error("sequence has no __len__ method".to_owned()))
            })?;
            len as isize - pos
        };
        Ok(hint)
    }
}

pub fn seq_iter_method(obj: PyObjectRef) -> PySequenceIterator {
    PySequenceIterator::new_forward(obj)
}

#[pyclass(module = false, name = "callable_iterator")]
#[derive(Debug)]
pub struct PyCallableIterator {
    callable: PyCallable,
    sentinel: PyObjectRef,
    done: AtomicCell<bool>,
}

impl PyValue for PyCallableIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.callable_iterator.clone()
    }
}

#[pyimpl]
impl PyCallableIterator {
    pub fn new(callable: PyCallable, sentinel: PyObjectRef) -> Self {
        Self {
            callable,
            sentinel,
            done: AtomicCell::new(false),
        }
    }

    #[pymethod(magic)]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        if self.done.load() {
            return Err(new_stop_iteration(vm));
        }

        let ret = self.callable.invoke(vec![], vm)?;

        if vm.bool_eq(ret.clone(), self.sentinel.clone())? {
            self.done.store(true);
            Err(new_stop_iteration(vm))
        } else {
            Ok(ret)
        }
    }

    #[pymethod(magic)]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

pub fn init(context: &PyContext) {
    PySequenceIterator::extend_class(context, &context.types.iter_type);
    PyCallableIterator::extend_class(context, &context.types.callable_iterator);
}

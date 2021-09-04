/*
 * utilities to support iteration.
 */

use crate::builtins::int::{self, PyInt};
use crate::builtins::iter::PySequenceIterator;
use crate::exceptions::PyBaseExceptionRef;
use crate::vm::VirtualMachine;
use crate::{IdProtocol, PyObjectRef, PyResult, PyValue, TypeProtocol};
use num_traits::Signed;

/*
 * This helper function is called at multiple places. First, it is called
 * in the vm when a for loop is entered. Next, it is used when the builtin
 * function 'iter' is called.
 */
pub fn get_iter(vm: &VirtualMachine, iter_target: PyObjectRef) -> PyResult {
    let getiter = {
        let cls = iter_target.class();
        cls.mro_find_map(|x| x.slots.iter.load())
    };
    if let Some(getiter) = getiter {
        let iter = getiter(iter_target, vm)?;
        let cls = iter.class();
        let is_iter = cls.iter_mro().any(|x| x.slots.iternext.load().is_some());
        if is_iter {
            drop(cls);
            Ok(iter)
        } else {
            Err(vm.new_type_error(format!(
                "iter() returned non-iterator of type '{}'",
                cls.name
            )))
        }
    } else {
        vm.get_method_or_type_error(iter_target.clone(), "__getitem__", || {
            format!("'{}' object is not iterable", iter_target.class().name)
        })?;
        Ok(PySequenceIterator::new(iter_target)
            .into_ref(vm)
            .into_object())
    }
}

pub fn call_next(vm: &VirtualMachine, iter_obj: &PyObjectRef) -> PyResult {
    let iternext = {
        let cls = iter_obj.class();
        cls.mro_find_map(|x| x.slots.iternext.load())
            .ok_or_else(|| vm.new_type_error(format!("'{}' object is not an iterator", cls.name)))?
    };
    iternext(iter_obj, vm)
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
            if next_error.isinstance(&vm.ctx.exceptions.stop_iteration) {
                Ok(None)
            } else {
                Err(next_error)
            }
        }
    }
}

pub fn try_map<F, R>(
    vm: &VirtualMachine,
    iter_obj: &PyObjectRef,
    cap: usize,
    mut f: F,
) -> PyResult<Vec<R>>
where
    F: FnMut(PyObjectRef) -> PyResult<R>,
{
    // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
    // https://github.com/python/cpython/blob/v3.9.0/Objects/listobject.c#L922-L928
    if cap >= isize::max_value() as usize {
        return Ok(Vec::new());
    }
    let mut results = Vec::with_capacity(cap);
    while let Some(element) = get_next_object(vm, iter_obj)? {
        results.push(f(element)?);
    }
    results.shrink_to_fit();
    Ok(results)
}

pub fn stop_iter_with_value(val: PyObjectRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let stop_iteration_type = vm.ctx.exceptions.stop_iteration.clone();
    vm.new_exception(stop_iteration_type, vec![val])
}

pub fn stop_iter_value(vm: &VirtualMachine, exc: &PyBaseExceptionRef) -> PyObjectRef {
    let args = exc.args();
    vm.unwrap_or_none(args.as_slice().first().cloned())
}

pub fn length_hint(vm: &VirtualMachine, iter: PyObjectRef) -> PyResult<Option<usize>> {
    if let Some(len) = vm.obj_len_opt(&iter) {
        match len {
            Ok(len) => return Ok(Some(len)),
            Err(e) => {
                if !e.isinstance(&vm.ctx.exceptions.type_error) {
                    return Err(e);
                }
            }
        }
    }
    let hint = match vm.get_method(iter, "__length_hint__") {
        Some(hint) => hint?,
        None => return Ok(None),
    };
    let result = match vm.invoke(&hint, ()) {
        Ok(res) => {
            if res.is(&vm.ctx.not_implemented) {
                return Ok(None);
            }
            res
        }
        Err(e) => {
            return if e.isinstance(&vm.ctx.exceptions.type_error) {
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
                result.class().name
            ))
        })?
        .as_bigint();
    if result.is_negative() {
        return Err(vm.new_value_error("__length_hint__() should return >= 0".to_owned()));
    }
    let hint = int::try_to_primitive(result, vm)?;
    Ok(Some(hint))
}

// pub fn seq_iter_method(obj: PyObjectRef) -> PySequenceIterator {
//     PySequenceIterator::new_forward(obj)
// }

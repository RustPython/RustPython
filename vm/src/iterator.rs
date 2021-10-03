/*
 * utilities to support iteration.
 */

use crate::{
    builtins::{PyBaseExceptionRef, PyInt},
    protocol::{PyIter, PyIterReturn},
    IdProtocol, PyObjectRef, PyResult, TypeProtocol, VirtualMachine,
};

pub fn try_map<F, R>(vm: &VirtualMachine, iter: &PyIter, cap: usize, mut f: F) -> PyResult<Vec<R>>
where
    F: FnMut(PyObjectRef) -> PyResult<R>,
{
    // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
    // https://github.com/python/cpython/blob/v3.9.0/Objects/listobject.c#L922-L928
    if cap >= isize::max_value() as usize {
        return Ok(Vec::new());
    }
    let mut results = Vec::with_capacity(cap);
    while let PyIterReturn::Return(element) = iter.next(vm)? {
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
    let hint = result
        .payload_if_subclass::<PyInt>(vm)
        .ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                result.class().name()
            ))
        })?
        .try_to_primitive::<isize>(vm)?;
    if hint.is_negative() {
        Err(vm.new_value_error("__length_hint__() should return >= 0".to_owned()))
    } else {
        Ok(Some(hint as usize))
    }
}

// pub fn seq_iter_method(obj: PyObjectRef) -> PySequenceIterator {
//     PySequenceIterator::new_forward(obj)
// }

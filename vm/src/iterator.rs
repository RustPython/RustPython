/*
 * utilities to support iteration.
 */

use crate::{
    protocol::{PyIter, PyIterReturn},
    PyObjectRef, PyResult, VirtualMachine,
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

// pub fn seq_iter_method(obj: PyObjectRef) -> PySequenceIterator {
//     PySequenceIterator::new_forward(obj)
// }

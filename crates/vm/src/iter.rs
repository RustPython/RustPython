use crate::{
    PyObject, PyObjectRef, PyResult,
    function::{Either, PyComparisonValue},
    protocol::{PyIter, PyIterReturn},
    types::PyComparisonOp,
    vm::VirtualMachine,
};
use itertools::Itertools;

pub trait PyExactSizeIterator<'a>: ExactSizeIterator<Item = &'a PyObject> + Sized {
    fn eq(self, other: impl PyExactSizeIterator<'a>, vm: &VirtualMachine) -> PyResult<bool> {
        let lhs = self;
        let rhs = other;
        if lhs.len() != rhs.len() {
            return Ok(false);
        }
        for (a, b) in lhs.zip_eq(rhs) {
            if !vm.identical_or_equal(a, b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn richcompare(
        self,
        other: impl PyExactSizeIterator<'a>,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        let lhs_len = self.len();
        let rhs_len = other.len();
        // Tuple comparison visits the common prefix even for unequal lengths.
        for (a, b) in self.zip(other) {
            if vm.identical_or_equal(a, b)? {
                continue;
            }
            return match op.eval_eq(|| false) {
                Some(equal) => Ok(Either::B(equal.into())),
                None => a.rich_compare(b, op, vm).map(Either::A),
            };
        }
        Ok(Either::B(op.eval_ord(lhs_len.cmp(&rhs_len)).into()))
    }
}

impl<'a, T> PyExactSizeIterator<'a> for T where T: ExactSizeIterator<Item = &'a PyObject> + Sized {}

/// Compare two iterator streams, applying the requested operation to the first
/// unequal pair. Unlike indexed sequence comparisons, even equality compares
/// that pair again, and inequality dispatches its own comparison method.
pub(crate) fn richcompare_iterators(
    lhs: &PyIter,
    rhs: &PyIter,
    op: PyComparisonOp,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    loop {
        let a = match lhs.next(vm)? {
            PyIterReturn::Return(item) => Some(item),
            PyIterReturn::StopIteration(_) => None,
        };
        // Both streams advance, including when the left stream just ended.
        let b = rhs.next(vm).map(|result| match result {
            PyIterReturn::Return(item) => Some(item),
            PyIterReturn::StopIteration(_) => None,
        });
        let result = match (&a, &b) {
            (Some(a), Ok(Some(b))) => {
                a.rich_compare_bool(b, PyComparisonOp::Eq, vm)
                    .and_then(|equal| {
                        if equal {
                            Ok(None)
                        } else {
                            a.rich_compare_bool(b, op, vm).map(Some)
                        }
                    })
            }
            (_, Ok(b)) => Ok(Some(op.eval_ord(a.is_some().cmp(&b.is_some())))),
            (_, Err(error)) => Err(error.clone()),
        };
        // Finalizers may mutate either stream before the next iteration.
        drop(a);
        drop(b);
        if let Some(result) = result? {
            return Ok(result);
        }
    }
}

/// Compare two sequences whose item-comparison callbacks may mutate the
/// containers. `get(i)` must snapshot `(len, item_at_i)` and drop any
/// container lock before returning.
///
/// After an unequal item, sizes are read again: if a callback shrank a
/// sequence so `i` is now past the end, the result is a length comparison
/// rather than the item inequality.
pub fn richcompare_mutating_seqs(
    mut get_a: impl FnMut(usize) -> (usize, Option<PyObjectRef>),
    mut get_b: impl FnMut(usize) -> (usize, Option<PyObjectRef>),
    op: PyComparisonOp,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    if matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
        let (a_len, _) = get_a(0);
        let (b_len, _) = get_b(0);
        if a_len != b_len {
            return Ok(op == PyComparisonOp::Ne);
        }
    }

    let mut i = 0usize;
    loop {
        let (a_len, a_item) = get_a(i);
        let (b_len, b_item) = get_b(i);
        let (Some(a_item), Some(b_item)) = (a_item, b_item) else {
            return Ok(op.eval_ord(a_len.cmp(&b_len)));
        };

        if vm.bool_eq(&a_item, &b_item)? {
            i += 1;
            continue;
        }

        let (a_len, _) = get_a(i);
        let (b_len, _) = get_b(i);
        if i >= a_len || i >= b_len {
            return Ok(op.eval_ord(a_len.cmp(&b_len)));
        }

        return match op {
            PyComparisonOp::Eq => Ok(false),
            PyComparisonOp::Ne => Ok(true),
            _ => a_item.rich_compare_bool(&b_item, op, vm),
        };
    }
}

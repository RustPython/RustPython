use crate::{PyObject, PyObjectRef, PyResult, types::PyComparisonOp, vm::VirtualMachine};
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
    ) -> PyResult<bool> {
        let less = match op {
            PyComparisonOp::Eq => return PyExactSizeIterator::eq(self, other, vm),
            PyComparisonOp::Ne => return PyExactSizeIterator::eq(self, other, vm).map(|eq| !eq),
            PyComparisonOp::Lt | PyComparisonOp::Le => true,
            PyComparisonOp::Gt | PyComparisonOp::Ge => false,
        };

        let lhs = self;
        let rhs = other;
        let lhs_len = lhs.len();
        let rhs_len = rhs.len();
        for (a, b) in lhs.zip(rhs) {
            if vm.bool_eq(a, b)? {
                continue;
            }
            let ret = if less {
                vm.bool_seq_lt(a, b)?
            } else {
                vm.bool_seq_gt(a, b)?
            };
            if let Some(v) = ret {
                return Ok(v);
            }
        }
        Ok(op.eval_ord(lhs_len.cmp(&rhs_len)))
    }
}

impl<'a, T> PyExactSizeIterator<'a> for T where T: ExactSizeIterator<Item = &'a PyObject> + Sized {}

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

use crate::{types::PyComparisonOp, vm::VirtualMachine, PyObjectRef, PyResult};
use itertools::Itertools;

pub trait PyExactSizeIterator<'a>: ExactSizeIterator<Item = &'a PyObjectRef> + Sized {
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

impl<'a, T> PyExactSizeIterator<'a> for T where T: ExactSizeIterator<Item = &'a PyObjectRef> + Sized {}

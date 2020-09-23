use crate::pyobject::{PyObjectRef, PyResult};
use crate::slots::PyComparisonOp;
use crate::vm::VirtualMachine;
use num_traits::cast::ToPrimitive;

pub(super) type DynPyIter<'a> = Box<dyn ExactSizeIterator<Item = &'a PyObjectRef> + 'a>;

#[allow(clippy::len_without_is_empty)]
pub(crate) trait SimpleSeq {
    fn len(&self) -> usize;
    fn boxed_iter(&self) -> DynPyIter;
}

impl<'a, D> SimpleSeq for D
where
    D: 'a + std::ops::Deref<Target = [PyObjectRef]>,
{
    fn len(&self) -> usize {
        self.deref().len()
    }

    fn boxed_iter(&self) -> DynPyIter {
        Box::new(self.deref().iter())
    }
}

pub(crate) fn eq(vm: &VirtualMachine, zelf: DynPyIter, other: DynPyIter) -> PyResult<bool> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf, other) {
            if !vm.identical_or_equal(a, b)? {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn cmp(
    vm: &VirtualMachine,
    zelf: DynPyIter,
    other: DynPyIter,
    op: PyComparisonOp,
) -> PyResult<bool> {
    let less = match op {
        PyComparisonOp::Eq => return eq(vm, zelf, other),
        PyComparisonOp::Ne => return eq(vm, zelf, other).map(|eq| !eq),
        PyComparisonOp::Lt | PyComparisonOp::Le => true,
        PyComparisonOp::Gt | PyComparisonOp::Ge => false,
    };
    let (lhs_len, rhs_len) = (zelf.len(), other.len());
    for (a, b) in Iterator::zip(zelf, other) {
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

pub(crate) struct SeqMul<'a> {
    seq: &'a dyn SimpleSeq,
    repetitions: usize,
    iter: Option<DynPyIter<'a>>,
}

impl ExactSizeIterator for SeqMul<'_> {}

impl<'a> Iterator for SeqMul<'a> {
    type Item = &'a PyObjectRef;
    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.as_mut().and_then(Iterator::next) {
            Some(item) => Some(item),
            None => {
                if self.repetitions == 0 {
                    None
                } else {
                    self.repetitions -= 1;
                    self.iter = Some(self.seq.boxed_iter());
                    self.next()
                }
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.iter.as_ref().map_or(0, ExactSizeIterator::len)
            + (self.repetitions * self.seq.len());
        (size, Some(size))
    }
}

pub(crate) fn seq_mul(seq: &impl SimpleSeq, repetitions: isize) -> SeqMul {
    let repetitions = if seq.len() > 0 {
        repetitions.to_usize().unwrap_or(0)
    } else {
        0
    };
    SeqMul {
        seq,
        repetitions,
        iter: None,
    }
}

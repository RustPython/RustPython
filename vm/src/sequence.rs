use crate::pyobject::{IdProtocol, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
use std::ops::Deref;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

type DynPyIter<'a> = Box<dyn ExactSizeIterator<Item = &'a PyObjectRef> + 'a>;

#[allow(clippy::len_without_is_empty)]
pub trait SimpleSeq {
    fn len(&self) -> usize;
    fn iter(&self) -> DynPyIter;
}

// impl SimpleSeq for &[PyObjectRef] {
//     fn len(&self) -> usize {
//         (&**self).len()
//     }
//     fn iter(&self) -> DynPyIter {
//         Box::new((&**self).iter())
//     }
// }

impl SimpleSeq for Vec<PyObjectRef> {
    fn len(&self) -> usize {
        self.len()
    }
    fn iter(&self) -> DynPyIter {
        Box::new(self.as_slice().iter())
    }
}

impl SimpleSeq for std::collections::VecDeque<PyObjectRef> {
    fn len(&self) -> usize {
        self.len()
    }
    fn iter(&self) -> DynPyIter {
        Box::new(self.iter())
    }
}

impl<T> SimpleSeq for std::cell::Ref<'_, T>
where
    T: SimpleSeq,
{
    fn len(&self) -> usize {
        self.deref().len()
    }
    fn iter(&self) -> DynPyIter {
        self.deref().iter()
    }
}

impl<T> SimpleSeq for RwLockReadGuard<'_, T>
where
    T: SimpleSeq,
{
    fn len(&self) -> usize {
        self.deref().len()
    }
    fn iter(&self) -> DynPyIter {
        self.deref().iter()
    }
}

impl<T> SimpleSeq for RwLockWriteGuard<'_, T>
where
    T: SimpleSeq,
{
    fn len(&self) -> usize {
        self.deref().len()
    }
    fn iter(&self) -> DynPyIter {
        self.deref().iter()
    }
}

// impl<'a, I>

pub(crate) fn eq(
    vm: &VirtualMachine,
    zelf: &impl SimpleSeq,
    other: &impl SimpleSeq,
) -> PyResult<bool> {
    if zelf.len() == other.len() {
        for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
            if a.is(b) {
                continue;
            }
            if !vm.bool_eq(a.clone(), b.clone())? {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(crate) fn lt(
    vm: &VirtualMachine,
    zelf: &impl SimpleSeq,
    other: &impl SimpleSeq,
) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_lt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }
    Ok(zelf.len() < other.len())
}

pub(crate) fn gt(
    vm: &VirtualMachine,
    zelf: &impl SimpleSeq,
    other: &impl SimpleSeq,
) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_gt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }
    Ok(zelf.len() > other.len())
}

pub(crate) fn ge(
    vm: &VirtualMachine,
    zelf: &impl SimpleSeq,
    other: &impl SimpleSeq,
) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_gt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }

    Ok(zelf.len() >= other.len())
}

pub(crate) fn le(
    vm: &VirtualMachine,
    zelf: &impl SimpleSeq,
    other: &impl SimpleSeq,
) -> PyResult<bool> {
    for (a, b) in Iterator::zip(zelf.iter(), other.iter()) {
        if let Some(v) = vm.bool_seq_lt(a.clone(), b.clone())? {
            return Ok(v);
        }
    }

    Ok(zelf.len() <= other.len())
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
        if self.seq.len() == 0 {
            return None;
        }
        match self.iter.as_mut().and_then(Iterator::next) {
            Some(item) => Some(item),
            None => {
                if self.repetitions == 0 {
                    None
                } else {
                    self.repetitions -= 1;
                    self.iter = Some(self.seq.iter());
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
    SeqMul {
        seq,
        repetitions: repetitions.max(0) as usize,
        iter: None,
    }
}

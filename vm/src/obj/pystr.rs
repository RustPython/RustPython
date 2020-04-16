use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

pub trait PyCommonString<'a, E>
where
    Self: 'a,
{
    fn py_split<SP, SN, SW, R>(
        &self,
        sep: Option<&Self>,
        maxsplit: isize,
        vm: &VirtualMachine,
        split: SP,
        splitn: SN,
        splitw: SW,
    ) -> Vec<R>
    where
        SP: Fn(&Self, &Self, &VirtualMachine) -> Vec<R>,
        SN: Fn(&Self, &Self, usize, &VirtualMachine) -> Vec<R>,
        SW: Fn(&Self, isize, &VirtualMachine) -> Vec<R>,
    {
        if let Some(pattern) = sep {
            if maxsplit < 0 {
                split(self, pattern, vm)
            } else {
                splitn(self, pattern, (maxsplit + 1) as usize, vm)
            }
        } else {
            splitw(self, maxsplit, vm)
        }
    }
    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef;
    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef;
}

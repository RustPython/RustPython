use crate::pyobject::{PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

pub trait PySequenceContainer
where
    Self: PyValue,
{
    fn as_slice(&self) -> &[PyObjectRef];

    #[inline]
    fn cmp<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: Fn(&[PyObjectRef], &[PyObjectRef]) -> PyResult<bool>,
    {
        let r = if let Some(other) = other.payload_if_subclass::<Self>(vm) {
            vm.new_bool(op(self.as_slice(), other.as_slice())?)
        } else {
            vm.ctx.not_implemented()
        };
        Ok(r)
    }
}

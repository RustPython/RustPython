use crate::{builtins::PyBaseExceptionRef, PyObjectRef, PyResult, VirtualMachine};

/// Implemented by any type that can be returned from a built-in Python function.
///
/// `ToPyObject` has a blanket implementation for any built-in object payload,
/// and should be implemented by many primitive Rust types, allowing a built-in
/// function to simply return a `bool` or a `usize` for example.
pub trait ToPyObject {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef;
}

pub trait ToPyResult {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult;
}

pub trait ToPyException {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef;
}

pub trait IntoPyException {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef;
}

impl<T> IntoPyException for &'_ T
where
    T: ToPyException,
{
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        self.to_pyexception(vm)
    }
}

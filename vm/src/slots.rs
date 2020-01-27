use crate::function::{OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::pyobject::{IdProtocol, PyObjectRef, PyRef, PyResult, PyValue};
use crate::VirtualMachine;

#[derive(Default)]
pub struct PyClassSlots {
    pub new: Option<PyNativeFunc>,
    pub call: Option<PyNativeFunc>,
    pub descr_get: Option<PyNativeFunc>,
}

impl std::fmt::Debug for PyClassSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PyClassSlots")
    }
}

#[pyimpl]
pub trait PyBuiltinCallable: PyValue {
    #[pymethod(magic)]
    #[pyslot]
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult;
}

#[pyimpl]
pub trait PyBuiltinDescriptor: PyValue {
    #[pymethod(magic)]
    #[pyslot(descr_get)]
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult;

    fn _cls_is<T>(cls: &OptionalArg<PyObjectRef>, other: &T) -> bool
    where
        T: IdProtocol,
    {
        match cls {
            OptionalArg::Present(cls) => cls.is(other),
            OptionalArg::Missing => false,
        }
    }
}

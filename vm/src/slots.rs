use crate::function::{OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::pyobject::{IdProtocol, PyObjectRef, PyRef, PyResult, PyValue};
use crate::VirtualMachine;

bitflags! {
    pub struct PyTpFlags: u64 {
        const BASETYPE = 1 << 10;
    }
}

impl PyTpFlags {
    // CPython default: Py_TPFLAGS_HAVE_STACKLESS_EXTENSION | Py_TPFLAGS_HAVE_VERSION_TAG
    pub const DEFAULT: Self = Self::from_bits_truncate(0);

    pub fn has_feature(self, flag: Self) -> bool {
        self.contains(flag)
    }
}

impl Default for PyTpFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Default)]
pub struct PyClassSlot {
    pub flags: PyTpFlags,
    pub new: Option<PyNativeFunc>,
    pub call: Option<PyNativeFunc>,
    pub descr_get: Option<PyNativeFunc>,
}

impl std::fmt::Debug for PyClassSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PyClassSlot")
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

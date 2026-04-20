use crate::{
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    convert::{ToPyObject, ToPyResult},
};
use rustpython_host_env::nt as host_nt;

/// Windows HANDLE wrapper for Python interop
#[derive(Clone, Copy)]
pub struct WinHandle(pub host_nt::Handle);

pub(crate) trait WindowsSysResultValue {
    type Ok: ToPyObject;

    fn is_err(&self) -> bool;

    fn into_ok(self) -> Self::Ok;
}

impl WindowsSysResultValue for host_nt::Handle {
    type Ok = WinHandle;

    fn is_err(&self) -> bool {
        host_nt::is_invalid_handle(*self)
    }

    fn into_ok(self) -> Self::Ok {
        WinHandle(self)
    }
}

// BOOL is i32 in windows-sys 0.61+
impl WindowsSysResultValue for i32 {
    type Ok = ();

    fn is_err(&self) -> bool {
        *self == 0
    }

    fn into_ok(self) -> Self::Ok {}
}

pub(crate) struct WindowsSysResult<T>(pub T);

impl<T: WindowsSysResultValue> WindowsSysResult<T> {
    pub(crate) fn is_err(&self) -> bool {
        self.0.is_err()
    }

    pub(crate) fn into_pyresult(self, vm: &VirtualMachine) -> PyResult<T::Ok> {
        if !self.is_err() {
            Ok(self.0.into_ok())
        } else {
            Err(vm.new_last_os_error())
        }
    }
}

impl<T: WindowsSysResultValue> ToPyResult for WindowsSysResult<T> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        let ok = self.into_pyresult(vm)?;
        Ok(ok.to_pyobject(vm))
    }
}

type HandleInt = isize;

impl TryFromObject for WinHandle {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let handle = HandleInt::try_from_object(vm, obj)?;
        Ok(WinHandle(handle as host_nt::Handle))
    }
}

impl ToPyObject for WinHandle {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        (self.0 as HandleInt).to_pyobject(vm)
    }
}

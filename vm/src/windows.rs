use crate::{
    convert::{ToPyObject, ToPyResult},
    stdlib::os::errno_err,
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
};
use windows::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::{BOOL, HANDLE as RAW_HANDLE};

pub(crate) trait WindowsSysResultValue {
    type Ok: ToPyObject;
    fn is_err(&self) -> bool;
    fn into_ok(self) -> Self::Ok;
}

impl WindowsSysResultValue for RAW_HANDLE {
    type Ok = HANDLE;
    fn is_err(&self) -> bool {
        *self == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE
    }
    fn into_ok(self) -> Self::Ok {
        HANDLE(self)
    }
}

impl WindowsSysResultValue for BOOL {
    type Ok = ();
    fn is_err(&self) -> bool {
        *self == 0
    }
    fn into_ok(self) -> Self::Ok {}
}

pub(crate) struct WindowsSysResult<T>(pub T);

impl<T: WindowsSysResultValue> WindowsSysResult<T> {
    pub fn is_err(&self) -> bool {
        self.0.is_err()
    }
    pub fn into_pyresult(self, vm: &VirtualMachine) -> PyResult<T::Ok> {
        if self.is_err() {
            Err(errno_err(vm))
        } else {
            Ok(self.0.into_ok())
        }
    }
}

impl<T: WindowsSysResultValue> ToPyResult for WindowsSysResult<T> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        let ok = self.into_pyresult(vm)?;
        Ok(ok.to_pyobject(vm))
    }
}

type HandleInt = usize; // TODO: change to isize when fully ported to windows-rs

impl TryFromObject for HANDLE {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let handle = HandleInt::try_from_object(vm, obj)?;
        Ok(HANDLE(handle as isize))
    }
}

impl ToPyObject for HANDLE {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        (self.0 as HandleInt).to_pyobject(vm)
    }
}

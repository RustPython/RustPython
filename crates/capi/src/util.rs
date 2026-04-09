use core::ffi::c_long;
use core::ffi::{c_char, c_int};
use rustpython_vm::{PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine};

pub(crate) trait FfiResult {
    type Output;
    fn into_output(self, vm: &VirtualMachine) -> Self::Output;
}

impl FfiResult for () {
    type Output = ();

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self
    }
}

impl<T> FfiResult for PyRef<T>
where
    Self: Into<PyObjectRef>,
{
    type Output = *mut PyObject;

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self.into().into_raw().as_ptr()
    }
}

impl<T> FfiResult for Option<PyRef<T>>
where
    PyRef<T>: Into<PyObjectRef>,
{
    type Output = *mut PyObject;

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self.map_or_else(core::ptr::null_mut, |obj| obj.into().into_raw().as_ptr())
    }
}

impl FfiResult for PyObjectRef {
    type Output = *mut PyObject;

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self.into_raw().as_ptr()
    }
}
impl FfiResult for *const PyObject {
    type Output = *mut PyObject;

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self.cast_mut()
    }
}

impl FfiResult for PyResult<*const PyObject> {
    type Output = *mut PyObject;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.map_or_else(
            |err| {
                vm.push_exception(Some(err));
                core::ptr::null_mut()
            },
            |ptr| ptr.cast_mut(),
        )
    }
}

impl FfiResult for PyResult {
    type Output = *mut PyObject;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.map_or_else(
            |err| {
                vm.push_exception(Some(err));
                core::ptr::null_mut()
            },
            |obj| obj.into_raw().as_ptr(),
        )
    }
}

impl<T> FfiResult for PyResult<PyRef<T>>
where
    PyRef<T>: Into<PyObjectRef>,
{
    type Output = *mut PyObject;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.map(Into::into).into_output(vm)
    }
}

impl FfiResult for PyResult<c_long> {
    type Output = c_long;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.unwrap_or_else(|err| {
            vm.push_exception(Some(err));
            -1
        })
    }
}

impl FfiResult for isize {
    type Output = isize;

    fn into_output(self, _vm: &VirtualMachine) -> Self::Output {
        self
    }
}

impl FfiResult for PyResult<isize> {
    type Output = isize;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.unwrap_or_else(|err| {
            vm.push_exception(Some(err));
            -1
        })
    }
}

impl FfiResult for PyResult<bool> {
    type Output = c_int;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.map_or_else(
            |err| {
                vm.push_exception(Some(err));
                -1
            },
            Into::into,
        )
    }
}

impl FfiResult for PyResult<c_int> {
    type Output = c_int;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.unwrap_or_else(|err| {
            vm.push_exception(Some(err));
            -1
        })
    }
}

impl FfiResult for PyResult<*mut c_char> {
    type Output = *mut c_char;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.map(|ptr| ptr.cast_const()).into_output(vm).cast_mut()
    }
}

impl FfiResult for PyResult<*const c_char> {
    type Output = *const c_char;

    fn into_output(self, vm: &VirtualMachine) -> Self::Output {
        self.unwrap_or_else(|err| {
            vm.push_exception(Some(err));
            core::ptr::null_mut()
        })
    }
}

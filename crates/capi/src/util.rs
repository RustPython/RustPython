use crate::PyObject;
use crate::handles::{exported_object_handle, resolve_object_handle, wrapper_refcnt};
use core::convert::Infallible;
use core::ffi::{c_char, c_double, c_int, c_long, c_ulonglong, c_void};
use core::ptr::NonNull;
use rustpython_vm::{PyObjectRef, PyRef, PyResult, VirtualMachine};

pub(crate) trait FfiResult<Output = Self> {
    const ERR_VALUE: Output;

    fn into_output(self, vm: &VirtualMachine) -> Output;
}

impl FfiResult for () {
    const ERR_VALUE: () = ();

    fn into_output(self, _vm: &VirtualMachine) {
        self
    }
}

impl FfiResult<c_int> for () {
    const ERR_VALUE: c_int = -1;

    fn into_output(self, _vm: &VirtualMachine) -> c_int {
        0
    }
}

impl<T> FfiResult<*mut PyObject> for PyRef<T>
where
    Self: Into<PyObjectRef>,
{
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        let ptr = self.into().into_raw().as_ptr();
        unsafe { exported_object_handle(ptr) }
    }
}

impl FfiResult<*mut PyObject> for PyObjectRef {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        let ptr = self.into_raw().as_ptr();
        unsafe { exported_object_handle(ptr) }
    }
}

pub(crate) unsafe fn owned_from_exported_new_ref(exported: *mut PyObject) -> PyObjectRef {
    if unsafe { wrapper_refcnt(exported) }.is_some() {
        let resolved = unsafe { resolve_object_handle(exported) };
        unsafe { (&*resolved).to_owned() }
    } else {
        let resolved = unsafe { resolve_object_handle(exported) };
        unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(resolved)) }
    }
}

impl FfiResult for *mut PyObject {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        unsafe { exported_object_handle(self) }
    }
}

impl FfiResult<*mut PyObject> for *const PyObject {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        unsafe { exported_object_handle(self.cast_mut()) }
    }
}

impl FfiResult for *mut c_void {
    const ERR_VALUE: *mut c_void = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut c_void {
        self
    }
}

impl FfiResult<*mut c_char> for *const u8 {
    const ERR_VALUE: *mut c_char = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut c_char {
        self.cast_mut().cast()
    }
}

impl FfiResult<isize> for usize {
    const ERR_VALUE: isize = -1;

    fn into_output(self, _vm: &VirtualMachine) -> isize {
        self as isize
    }
}

impl FfiResult for c_long {
    const ERR_VALUE: c_long = -1;

    fn into_output(self, _vm: &VirtualMachine) -> c_long {
        self
    }
}

impl FfiResult for c_ulonglong {
    const ERR_VALUE: c_ulonglong = c_ulonglong::MAX;

    fn into_output(self, _vm: &VirtualMachine) -> c_ulonglong {
        self
    }
}

impl FfiResult for c_double {
    const ERR_VALUE: c_double = -1.0;

    fn into_output(self, _vm: &VirtualMachine) -> c_double {
        self
    }
}

impl FfiResult<c_int> for bool {
    const ERR_VALUE: c_int = -1;

    fn into_output(self, _vm: &VirtualMachine) -> c_int {
        self as c_int
    }
}

impl FfiResult<()> for PyResult<Infallible> {
    const ERR_VALUE: () = ();

    fn into_output(self, vm: &VirtualMachine) {
        match self {
            Err(err) => vm.push_exception(Some(err)),
        }
    }
}

impl FfiResult<*mut c_void> for Option<*mut c_void> {
    const ERR_VALUE: *mut c_void = core::ptr::null_mut();

    fn into_output(self, vm: &VirtualMachine) -> *mut c_void {
        self.map_or_else(|| Self::ERR_VALUE, |obj| obj.into_output(vm))
    }
}

impl<T> FfiResult<*mut PyObject> for Option<T>
where
    T: FfiResult<*mut PyObject>,
{
    const ERR_VALUE: *mut PyObject = T::ERR_VALUE;

    fn into_output(self, vm: &VirtualMachine) -> *mut PyObject {
        self.map_or_else(|| Self::ERR_VALUE, |obj| obj.into_output(vm))
    }
}

impl<Output, T> FfiResult<Output> for PyResult<T>
where
    T: FfiResult<Output>,
{
    const ERR_VALUE: Output = T::ERR_VALUE;

    fn into_output(self, vm: &VirtualMachine) -> Output {
        self.map_or_else(
            |err| {
                vm.push_exception(Some(err));
                T::ERR_VALUE
            },
            |obj| obj.into_output(vm),
        )
    }
}

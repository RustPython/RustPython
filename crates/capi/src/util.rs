use crate::PyObject;
use core::convert::Infallible;
use core::ffi::{c_char, c_double, c_int, c_long, c_ulong, c_void};
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
        self.into().into_raw().as_ptr()
    }
}

impl FfiResult<*mut PyObject> for PyObjectRef {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        self.into_raw().as_ptr()
    }
}

impl FfiResult for *mut PyObject {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        self
    }
}

impl FfiResult<*mut PyObject> for *const PyObject {
    const ERR_VALUE: *mut PyObject = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut PyObject {
        self.cast_mut()
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

impl FfiResult<*mut c_char> for *mut u8 {
    const ERR_VALUE: *mut c_char = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut c_char {
        self.cast()
    }
}

impl FfiResult for *const c_char {
    const ERR_VALUE: *const c_char = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *const c_char {
        self
    }
}

impl FfiResult<isize> for usize {
    const ERR_VALUE: isize = -1;

    fn into_output(self, _vm: &VirtualMachine) -> isize {
        self.try_into()
            .expect("Output value is too large to fit into target type")
    }
}

impl FfiResult for isize {
    const ERR_VALUE: Self = -1;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

#[cfg(not(windows))]
impl FfiResult for c_int {
    const ERR_VALUE: Self = -1;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl FfiResult for usize {
    const ERR_VALUE: Self = Self::MAX;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl FfiResult for c_long {
    const ERR_VALUE: Self = -1;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl FfiResult for c_ulong {
    const ERR_VALUE: Self = Self::MAX;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

#[cfg(windows)]
impl FfiResult for core::ffi::c_longlong {
    const ERR_VALUE: Self = -1;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

#[cfg(windows)]
impl FfiResult for core::ffi::c_ulonglong {
    const ERR_VALUE: Self = Self::MAX;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl FfiResult for c_double {
    const ERR_VALUE: Self = -1.0;

    fn into_output(self, _vm: &VirtualMachine) -> Self {
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
            Err(err) => vm.set_exception(Some(err)),
        }
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
                vm.set_exception(Some(err));
                T::ERR_VALUE
            },
            |obj| obj.into_output(vm),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::any::type_name;
    use core::ffi::{c_longlong, c_ulonglong};
    use core::fmt::Debug;

    #[test]
    fn ffi_result_err_value() {
        fn assert_error_value<T, Output>(value: Output)
        where
            T: FfiResult<Output> + 'static,
            Output: PartialEq + Debug,
        {
            assert_eq!(value, T::ERR_VALUE, "{}", type_name::<T>(),);
        }

        assert_error_value::<(), _>(());
        assert_error_value::<(), c_int>(-1);

        assert_error_value::<isize, _>(-1);
        assert_error_value::<usize, _>(usize::MAX);
        assert_error_value::<usize, isize>(-1);
        assert_error_value::<c_int, _>(-1); // i32
        assert_error_value::<c_long, _>(-1); //Windows i32, unix i64
        assert_error_value::<c_ulong, _>(c_ulong::MAX); // Windows u32, unix u64
        assert_error_value::<c_longlong, _>(-1); // i64
        assert_error_value::<c_ulonglong, _>(c_ulonglong::MAX); // u64
        assert_error_value::<c_double, _>(-1.0);
        assert_error_value::<bool, _>(-1);

        assert_error_value::<PyResult<c_int>, _>(-1);
        assert_error_value::<PyResult<usize>, _>(usize::MAX);
    }
}

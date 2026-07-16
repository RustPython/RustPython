use crate::PyObject;
use core::convert::Infallible;
use core::ffi::{CStr, c_char, c_double, c_int, c_long, c_ulong, c_void};
use core::ptr::NonNull;
use rustpython_vm::{Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};
use std::any::type_name;

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

impl<T> FfiResult<*mut Py<T>> for PyRef<T>
where
    Self: Into<PyObjectRef>,
{
    const ERR_VALUE: *mut Py<T> = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *mut Py<T> {
        self.into().into_raw().as_ptr().cast()
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

impl FfiResult<*const c_char> for &CStr {
    const ERR_VALUE: *const c_char = core::ptr::null_mut();

    fn into_output(self, _vm: &VirtualMachine) -> *const c_char {
        self.as_ptr()
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

pub(crate) trait CStrExt<'a> {
    unsafe fn try_as_str(self, vm: &VirtualMachine) -> PyResult<&'a str>;
    unsafe fn try_as_str_opt(self, vm: &VirtualMachine) -> PyResult<Option<&'a str>>;
}

pub(crate) trait FfiPtrExt: Sized {
    type Owned;
    type Borrowed;

    unsafe fn assume_owned_or_opt(self) -> Option<Self::Owned>;
    unsafe fn assume_owned_or_err(self, vm: &VirtualMachine) -> PyResult<Self::Owned> {
        unsafe { self.assume_owned_or_opt() }.ok_or_else(|| {
            vm.take_raised_exception().unwrap_or_else(|| {
                vm.new_system_error("Native function returned NULL, but there was no exception set")
            })
        })
    }
    unsafe fn assume_owned(self) -> Self::Owned;
    unsafe fn assume_borrowed_or_opt<'a>(self) -> Option<&'a Self::Borrowed>;
    unsafe fn assume_borrowed<'a>(self) -> &'a Self::Borrowed;

    unsafe fn assume_borrowed_and_cast<'a, T: PyPayload>(
        self,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<T>>;
}

impl FfiPtrExt for *mut PyObject {
    type Owned = PyObjectRef;
    type Borrowed = PyObject;

    #[inline]
    unsafe fn assume_owned_or_opt(self) -> Option<PyObjectRef> {
        NonNull::new(self).map(|ptr| unsafe { PyObjectRef::from_raw(ptr) })
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_owned(self) -> PyObjectRef {
        debug_assert!(
            !self.is_null(),
            "Attempted to dereference NULL {}",
            type_name::<Self>()
        );
        unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(self)) }
    }

    #[inline]
    unsafe fn assume_borrowed_or_opt<'a>(self) -> Option<&'a PyObject> {
        unsafe { self.as_ref() }
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_borrowed<'a>(self) -> &'a PyObject {
        debug_assert!(
            !self.is_null(),
            "Attempted to dereference NULL {}",
            type_name::<Self>()
        );
        unsafe { self.as_ref_unchecked() }
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_borrowed_and_cast<'a, T: PyPayload>(
        self,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<T>> {
        unsafe { self.assume_borrowed() }.try_downcast_ref(vm)
    }
}

impl<T: PyPayload> FfiPtrExt for *mut Py<T> {
    type Owned = PyRef<T>;
    type Borrowed = Py<T>;

    #[inline]
    unsafe fn assume_owned_or_opt(self) -> Option<PyRef<T>> {
        NonNull::new(self).map(|ptr| unsafe { PyRef::from_non_null(ptr) })
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_owned(self) -> PyRef<T> {
        debug_assert!(
            !self.is_null(),
            "Attempted to dereference NULL {}",
            type_name::<Self>()
        );
        unsafe { PyRef::from_raw(self.cast_const()) }
    }

    #[inline]
    unsafe fn assume_borrowed_or_opt<'a>(self) -> Option<&'a Py<T>> {
        unsafe { self.as_ref() }
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_borrowed<'a>(self) -> &'a Py<T> {
        debug_assert!(
            !self.is_null(),
            "Attempted to dereference NULL {}",
            type_name::<Self>()
        );
        unsafe { self.as_ref_unchecked() }
    }

    #[inline]
    #[track_caller]
    unsafe fn assume_borrowed_and_cast<'a, U: PyPayload>(
        self,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<U>> {
        unsafe { self.cast::<PyObject>().assume_borrowed_and_cast(vm) }
    }
}

impl<'a> CStrExt<'a> for *mut c_char {
    unsafe fn try_as_str(self, vm: &VirtualMachine) -> PyResult<&'a str> {
        unsafe { self.try_as_str_opt(vm) }?
            .ok_or_else(|| vm.new_system_error("argument must not be null"))
    }

    unsafe fn try_as_str_opt(self, vm: &VirtualMachine) -> PyResult<Option<&'a str>> {
        NonNull::new(self)
            .map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) }.to_str())
            .transpose()
            .map_err(|_| vm.new_system_error("argument must be valid UTF-8"))
    }
}

impl<'a> CStrExt<'a> for *const c_char {
    unsafe fn try_as_str(self, vm: &VirtualMachine) -> PyResult<&'a str> {
        unsafe { self.cast_mut().try_as_str(vm) }
    }

    unsafe fn try_as_str_opt(self, vm: &VirtualMachine) -> PyResult<Option<&'a str>> {
        unsafe { self.cast_mut().try_as_str_opt(vm) }
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

    #[test]
    #[should_panic = "Attempted to dereference NULL"]
    fn break_ptr_api_contract_owned() {
        let ptr: *mut PyObject = core::ptr::null_mut();
        let _ = unsafe { ptr.assume_owned() };
    }

    #[test]
    #[should_panic = "Attempted to dereference NULL"]
    fn break_ptr_api_contract_borrowed() {
        let ptr: *mut PyObject = core::ptr::null_mut();
        let _ = unsafe { ptr.assume_borrowed() };
    }
}

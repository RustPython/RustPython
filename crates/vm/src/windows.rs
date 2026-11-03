use crate::{
    PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::PyInt,
    convert::{ToPyObject, ToPyResult},
};
use num_traits::ToPrimitive;
use rustpython_host_env::nt as host_nt;

/// Windows HANDLE wrapper for Python interop
#[derive(Clone, Copy)]
pub struct WinHandle(pub host_nt::Handle);

impl WinHandle {
    /// Convert an F_HANDLE bitfield after the caller has finished binding.
    /// The caller must keep the original argument alive through its operation.
    pub(crate) fn from_bitfield(
        obj: &PyObject,
        vm: &VirtualMachine,
        callee: &str,
        argument: &str,
    ) -> PyResult<Self> {
        let value = Self::parse_bitfield::<{ usize::BITS }>(obj, vm, callee, argument)?;
        Ok(Self(value as host_nt::Handle))
    }

    /// Convert a Windows `unsigned long` (`k`) after argument binding.
    pub(crate) fn unsigned_long_bitfield(
        obj: &PyObject,
        vm: &VirtualMachine,
        callee: &str,
        argument: &str,
    ) -> PyResult<u32> {
        Ok(Self::parse_bitfield::<32>(obj, vm, callee, argument)? as u32)
    }

    fn parse_bitfield<const WIDTH: u32>(
        obj: &PyObject,
        vm: &VirtualMachine,
        callee: &str,
        argument: &str,
    ) -> PyResult<usize> {
        let (handle, fits) = {
            let owned;
            let int = if let Some(int) = obj.downcast_ref::<PyInt>() {
                int
            } else if let Some(index) = obj.try_index_opt(vm) {
                owned = index?;
                &owned
            } else {
                let actual = if vm.is_none(obj) {
                    b"None".to_vec()
                } else {
                    let name = obj.class().slot_name();
                    name.as_bytes()[..name.len().min(50)].to_vec()
                };
                let mut message = format!("{callee}() {argument} must be int, not ").into_bytes();
                message.extend_from_slice(&actual);
                return Err(match String::from_utf8(message) {
                    Ok(message) => vm.new_type_error(message),
                    Err(error) => {
                        let start = error.utf8_error().valid_up_to();
                        let bytes = error.into_bytes();
                        let end = bytes.len();
                        vm.new_unicode_decode_error(
                            vm.ctx.new_str("utf-8"),
                            vm.ctx.new_bytes(bytes),
                            start,
                            end,
                            vm.ctx.new_str("unexpected end of data"),
                        )
                    }
                });
            };
            if WIDTH == 32 {
                if let Some(value) = int.as_bigint().to_i32() {
                    (value as u32 as usize, true)
                } else if let Some(value) = int.as_bigint().to_u32() {
                    (value as usize, true)
                } else {
                    (int.as_u32_mask() as usize, false)
                }
            } else if let Some(value) = int.as_bigint().to_isize() {
                (value as usize, true)
            } else if let Some(value) = int.as_bigint().to_usize() {
                (value, true)
            } else {
                (int.as_u64_mask() as usize, false)
            }
        };
        if !fits {
            crate::stdlib::_warnings::warn(
                vm.ctx.exceptions.deprecation_warning,
                "integer value out of range".to_owned(),
                1,
                vm,
            )?;
        }
        Ok(handle)
    }
}

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
        Ok(Self(handle as host_nt::Handle))
    }
}

impl ToPyObject for WinHandle {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        (self.0 as HandleInt).to_pyobject(vm)
    }
}

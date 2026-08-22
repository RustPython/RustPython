use crate::object::define_py_check;
use crate::{PyObject, pystate::with_vm};
use bitflags::bitflags;
use core::ffi::{CStr, c_char, c_double, c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void};
use malachite_bigint::{BigInt, BigUint, Sign};
use rustpython_vm::builtins::{PyInt, try_bigint_to_f64, try_f64_to_bigint};
use rustpython_vm::common::int::bytes_to_int;
use rustpython_vm::protocol::handle_bytes_to_int_err;
use rustpython_vm::{AsObject, PyResult, VirtualMachine};

define_py_check!(fn PyLong_Check, types.int_type);
define_py_check!(exact fn PyLong_CheckExact, types.int_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLong(value: c_long) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromLongLong(value: c_longlong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSsize_t(value: isize) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromSize_t(value: usize) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLong(value: c_ulong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUnsignedLongLong(value: c_ulonglong) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromDouble(value: c_double) -> *mut PyObject {
    with_vm(|vm| Ok(vm.ctx.new_bigint(&try_f64_to_bigint(value, vm)?)))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromInt32(value: i32) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromInt64(value: i64) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUInt32(value: u32) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromUInt64(value: u64) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(value))
}

bitflags! {
    #[derive(Clone, Copy)]
    struct AsNativeBytesFlags: c_int {
        const BIG_ENDIAN = 0;
        const LITTLE_ENDIAN = 1;
        const NATIVE_ENDIAN = 3;
        const UNSIGNED_BUFFER = 4;
        const REJECT_NEGATIVE = 8;
        const ALLOW_INDEX = 16;
    }
}

impl AsNativeBytesFlags {
    #[inline]
    fn is_little_endian(self) -> bool {
        if self.contains(Self::NATIVE_ENDIAN) {
            cfg!(target_endian = "little")
        } else {
            self.contains(Self::LITTLE_ENDIAN)
        }
    }

    fn from_bits_or_default(vm: &VirtualMachine, raw_flags: c_int) -> PyResult<Self> {
        const PY_ASNATIVEBYTES_DEFAULTS: c_int = -1;
        if raw_flags == PY_ASNATIVEBYTES_DEFAULTS {
            return Ok(Self::default());
        };

        let flags = Self::from_bits(raw_flags)
            .ok_or_else(|| vm.new_value_error("Invalid NativeBytes flags"))?;
        if flags.contains(Self::LITTLE_ENDIAN) & flags.contains(Self::NATIVE_ENDIAN) {
            Err(vm.new_value_error("Cannot specify both LITTLE_ENDIAN and NATIVE_ENDIAN"))
        } else {
            Ok(flags)
        }
    }
}

impl Default for AsNativeBytesFlags {
    fn default() -> Self {
        Self::NATIVE_ENDIAN | Self::UNSIGNED_BUFFER
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let flags = AsNativeBytesFlags::from_bits_or_default(vm, flags)?;
        let little_endian = flags.is_little_endian();
        let bytes = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) };

        let value = if flags.contains(AsNativeBytesFlags::UNSIGNED_BUFFER) {
            if little_endian {
                BigInt::from_bytes_le(Sign::Plus, bytes)
            } else {
                BigInt::from_bytes_be(Sign::Plus, bytes)
            }
        } else if little_endian {
            BigInt::from_signed_bytes_le(bytes)
        } else {
            BigInt::from_signed_bytes_be(bytes)
        };

        Ok(vm.ctx.new_bigint(&value))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromUnsignedNativeBytes(
    buffer: *const c_void,
    n_bytes: usize,
    flags: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let flags = AsNativeBytesFlags::from_bits_or_default(vm, flags)?;
        let bytes = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), n_bytes) };

        let value = if flags.is_little_endian() {
            BigInt::from_bytes_le(Sign::Plus, bytes)
        } else {
            BigInt::from_bytes_be(Sign::Plus, bytes)
        };

        Ok(vm.ctx.new_bigint(&value))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_FromVoidPtr(ptr: *mut c_void) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_int(ptr as usize))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FromString(
    str: *const c_char,
    pend: *mut *mut c_char,
    base: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let bytes = unsafe { CStr::from_ptr(str) }.to_bytes();
        let parsed = bytes_to_int(bytes, base as u32, vm.state.int_max_str_digits.load())
            .map(|value| vm.ctx.new_bigint(&value));

        if let Some(pend) = unsafe { pend.as_mut() } {
            let end_offset = if parsed.is_ok() { bytes.len() } else { 0 };
            unsafe { *pend = bytes.as_ptr().add(end_offset).cast_mut().cast() };
        }

        parsed.map_err(|err| {
            let obj = vm.ctx.new_bytes(bytes.to_vec());
            handle_bytes_to_int_err(err, obj.as_object(), vm)
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLong(obj: *mut PyObject) -> c_long {
    with_vm::<PyResult<c_long>, _>(|vm| {
        unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsDouble(obj: *mut PyObject) -> c_double {
    with_vm::<PyResult<c_double>, _>(|vm| {
        let int = unsafe { &*obj }.try_downcast_ref::<PyInt>(vm)?;
        try_bigint_to_f64(int.as_bigint(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt(obj: *mut PyObject) -> c_int {
    with_vm::<PyResult<c_int>, _>(|vm| {
        unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C int"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt32(obj: *mut PyObject, out: *mut i32) -> c_int {
    with_vm(|vm| {
        let value: i32 = unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to int32_t"))?;
        unsafe { *out = value };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsInt64(obj: *mut PyObject, out: *mut i64) -> c_int {
    with_vm(|vm| {
        let value: i64 = unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to int64_t"))?;
        unsafe { *out = value };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsLongLong(obj: *mut PyObject) -> c_longlong {
    with_vm::<PyResult<c_longlong>, _>(|vm| {
        unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long long"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSize_t(obj: *mut PyObject) -> usize {
    with_vm::<PyResult<usize>, _>(|vm| {
        let value: usize = unsafe { &*obj }
            .try_downcast_ref::<PyInt>(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C size_t"))?;
        Ok(value)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsSsize_t(obj: *mut PyObject) -> isize {
    with_vm::<PyResult<isize>, _>(|vm| {
        unsafe { &*obj }
            .try_downcast_ref::<PyInt>(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C ssize_t"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt32(obj: *mut PyObject, out: *mut u32) -> c_int {
    with_vm(|vm| {
        let value: u32 = unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to uint32_t"))?;
        unsafe { *out = value };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUInt64(obj: *mut PyObject, out: *mut u64) -> c_int {
    with_vm(|vm| {
        let value: u64 = unsafe { &*obj }
            .to_owned()
            .try_index(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to uint64_t"))?;
        unsafe { *out = value };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLong(obj: *mut PyObject) -> c_ulong {
    with_vm::<PyResult<c_ulong>, _>(|vm| {
        unsafe { &*obj }
            .try_downcast_ref::<PyInt>(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| {
                vm.new_overflow_error("Python int too large to convert to C unsigned long")
            })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongMask(obj: *mut PyObject) -> c_ulong {
    with_vm::<PyResult<c_ulong>, _>(|vm| {
        let int = unsafe { &*obj }.to_owned().try_index(vm)?;
        if const { c_ulong::BITS == 32 } {
            Ok(c_ulong::from(int.as_u32_mask()))
        } else {
            Ok(int.as_u64_mask() as c_ulong)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLongMask(obj: *mut PyObject) -> c_ulonglong {
    with_vm::<PyResult<c_ulonglong>, _>(|vm| {
        let int = unsafe { &*obj }.to_owned().try_index(vm)?;
        Ok(int.as_u64_mask())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsVoidPtr(obj: *mut PyObject) -> *mut c_void {
    with_vm(|vm| {
        let value = unsafe { &*obj }.try_downcast_ref::<PyInt>(vm)?;

        let unsigned: Result<usize, _> = value.as_bigint().try_into();
        if let Ok(v) = unsigned {
            return Ok(v as *mut c_void);
        }
        let signed: Result<isize, _> = value.as_bigint().try_into();
        if let Ok(v) = signed {
            return Ok((v as usize) as *mut c_void);
        }

        Err(vm.new_overflow_error("int too large to convert to pointer"))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_AsUnsignedLongLong(obj: *mut PyObject) -> c_ulonglong {
    with_vm::<PyResult<c_ulonglong>, _>(|vm| {
        unsafe { &*obj }
            .try_downcast_ref::<PyInt>(vm)?
            .as_bigint()
            .try_into()
            .map_err(|_| {
                vm.new_overflow_error("Python int too large to convert to C unsigned long long")
            })
    })
}

#[repr(C)]
pub struct PyLongLayout {
    pub bits_per_digit: u8,
    pub digit_size: u8,
    pub digits_order: i8,
    pub digit_endianness: i8,
}

#[repr(C)]
#[derive(Default)]
pub struct PyLongExport {
    pub value: i64,
    pub negative: u8,
    pub ndigits: isize,
    pub digits: *const c_void,
    _reserved: *mut Vec<u32>,
}

pub struct PyLongWriter {
    negative: bool,
    digits: Vec<u32>,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyLong_GetNativeLayout() -> *const PyLongLayout {
    const NATIVE_LONG_LAYOUT: PyLongLayout = PyLongLayout {
        bits_per_digit: 32,
        digit_size: 4,
        digits_order: -1,
        digit_endianness: if cfg!(target_endian = "little") {
            -1
        } else {
            1
        },
    };
    &NATIVE_LONG_LAYOUT
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_Export(
    obj: *mut PyObject,
    export_long: *mut PyLongExport,
) -> c_int {
    with_vm::<PyResult<()>, _>(|vm| {
        let py_int = unsafe { &*obj }.try_downcast_ref::<PyInt>(vm)?;
        let bigint = py_int.as_bigint();

        if let Ok(value) = i64::try_from(bigint) {
            unsafe {
                *export_long = PyLongExport {
                    value,
                    ..Default::default()
                };
            }
            return Ok(());
        }

        let (sign, digits) = bigint.to_u32_digits();
        let boxed_digits = Box::new(digits);
        let ndigits = boxed_digits.len().try_into().map_err(|_| {
            vm.new_overflow_error("PyLong_Export: too many digits to fit into Py_ssize_t")
        })?;

        unsafe {
            *export_long = PyLongExport {
                value: 0,
                negative: u8::from(matches!(sign, Sign::Minus)),
                ndigits,
                digits: boxed_digits.as_ptr().cast(),
                _reserved: Box::into_raw(boxed_digits),
            };
        }
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLong_FreeExport(export_long: *mut PyLongExport) {
    if export_long.is_null() {
        return;
    }

    let export_long = unsafe { &mut *export_long };
    if !export_long._reserved.is_null() {
        unsafe {
            drop(Box::from_raw(export_long._reserved));
        }
    }
    core::mem::take(export_long);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLongWriter_Create(
    negative: c_int,
    ndigits: isize,
    digits: *mut *mut c_void,
) -> *mut PyLongWriter {
    with_vm::<PyResult<*mut c_void>, _>(|vm| {
        if ndigits <= 0 {
            return Err(vm.new_value_error("PyLongWriter_Create: ndigits must be greater than 0"));
        }
        if digits.is_null() {
            return Err(vm.new_system_error("PyLongWriter_Create: digits must not be null"));
        }
        if negative != 0 && negative != 1 {
            return Err(vm.new_value_error("PyLongWriter_Create: negative must be 0 or 1"));
        }

        let ndigits = ndigits
            .try_into()
            .map_err(|_| vm.new_overflow_error("PyLongWriter_Create: ndigits out of range"))?;

        let mut writer = Box::new(PyLongWriter {
            negative: negative == 1,
            digits: vec![0; ndigits],
        });

        unsafe {
            *digits = writer.digits.as_mut_ptr().cast();
        }

        Ok(Box::into_raw(writer).cast())
    })
    .cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLongWriter_Finish(writer: *mut PyLongWriter) -> *mut PyObject {
    with_vm(|vm| {
        if writer.is_null() {
            return Err(vm.new_system_error("PyLongWriter_Finish: writer must not be null"));
        }

        let writer = unsafe { Box::from_raw(writer) };
        let mut digits = writer.digits;
        while matches!(digits.last(), Some(0)) {
            digits.pop();
        }

        if digits.is_empty() {
            return Ok(vm.ctx.new_int(0));
        }

        let magnitude = BigUint::new(digits);
        let sign = if writer.negative {
            Sign::Minus
        } else {
            Sign::Plus
        };

        let value = BigInt::from_biguint(sign, magnitude);
        Ok(vm.ctx.new_int(value))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyLongWriter_Discard(writer: *mut PyLongWriter) {
    if writer.is_null() {
        return;
    }

    unsafe {
        drop(Box::from_raw(writer));
    }
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyInt;

    #[test]
    fn py_int_u32() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123);
            assert!(number.is_instance_of::<PyInt>());
            assert_eq!(number.extract::<i32>().unwrap(), 123);
        })
    }

    #[test]
    fn py_int_u64() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123u64);
            assert!(number.is_instance_of::<PyInt>());
            assert_eq!(number.extract::<u64>().unwrap(), 123);
        })
    }

    #[test]
    fn py_int_u128() {
        Python::attach(|py| {
            let value = 1u128 << 100;
            let number = PyInt::new(py, value);
            assert_eq!(number.extract::<u128>().unwrap(), value);
        })
    }
}

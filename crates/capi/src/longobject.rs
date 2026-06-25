use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void};
use malachite_bigint::{BigInt, BigUint, Sign};
use rustpython_vm::PyResult;
use rustpython_vm::builtins::PyInt;

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
pub unsafe extern "C" fn PyLong_AsUnsignedLongLong(obj: *mut PyObject) -> c_ulonglong {
    with_vm::<PyResult<c_ulonglong>, _>(|vm| {
        unsafe { &*obj }
            .to_owned()
            .try_downcast::<PyInt>(vm)?
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
    fn test_py_int_u32() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123);
            assert!(number.is_instance_of::<PyInt>());
            assert_eq!(number.extract::<i32>().unwrap(), 123);
        })
    }

    #[test]
    fn test_py_int_u64() {
        Python::attach(|py| {
            let number = PyInt::new(py, 123u64);
            assert!(number.is_instance_of::<PyInt>());
            assert_eq!(number.extract::<u64>().unwrap(), 123);
        })
    }
}

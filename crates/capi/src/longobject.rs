use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_double, c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void};
use rustpython_vm::builtins::{PyInt, try_bigint_to_f64, try_f64_to_bigint};
use rustpython_vm::common::int::bytes_to_int;
use rustpython_vm::protocol::handle_bytes_to_int_err;
use rustpython_vm::{AsObject, PyResult};

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

    #[test]
    fn py_int_u128() {
        Python::attach(|py| {
            let value = 1u128 << 100;
            let number = PyInt::new(py, value);
            assert_eq!(number.extract::<u128>().unwrap(), value);
        })
    }
}

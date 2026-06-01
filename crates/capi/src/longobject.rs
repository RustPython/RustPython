use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{c_long, c_longlong, c_ulong, c_ulonglong};
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

use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_ulong;
use rustpython_vm::builtins::PyType;
use rustpython_vm::convert::IntoObject;
use rustpython_vm::{AsObject, Context, Py};
use std::ffi::{c_int, c_uint};
use std::mem::MaybeUninit;

const PY_TPFLAGS_LONG_SUBCLASS: c_ulong = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: c_ulong = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: c_ulong = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: c_ulong = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: c_ulong = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: c_ulong = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: c_ulong = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: c_ulong = 1 << 31;

#[unsafe(no_mangle)]
pub static mut PyType_Type: MaybeUninit<&'static Py<PyType>> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyLong_Type: MaybeUninit<&'static Py<PyType>> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyTuple_Type: MaybeUninit<&'static Py<PyType>> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyUnicode_Type: MaybeUninit<&'static Py<PyType>> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyBool_Type: MaybeUninit<&'static Py<PyType>> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *const Py<PyType> {
    // SAFETY: The caller must guarantee that `op` is a valid pointer to a `PyObject`.
    unsafe { (*op).class() }
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn PyType_GetFlags(ptr: *const Py<PyType>) -> c_ulong {
    let ctx = Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;

    // SAFETY: The caller must guarantee that `ptr` is a valid pointer to a `PyType` object.
    let ty = unsafe { &*ptr };
    let mut flags = ty.slots.flags.bits();

    if ty.is_subtype(zoo.int_type) {
        flags |= PY_TPFLAGS_LONG_SUBCLASS;
    }
    if ty.is_subtype(zoo.list_type) {
        flags |= PY_TPFLAGS_LIST_SUBCLASS
    }
    if ty.is_subtype(zoo.tuple_type) {
        flags |= PY_TPFLAGS_TUPLE_SUBCLASS;
    }
    if ty.is_subtype(zoo.bytes_type) {
        flags |= PY_TPFLAGS_BYTES_SUBCLASS;
    }
    if ty.is_subtype(zoo.str_type) {
        flags |= PY_TPFLAGS_UNICODE_SUBCLASS;
    }
    if ty.is_subtype(zoo.dict_type) {
        flags |= PY_TPFLAGS_DICT_SUBCLASS;
    }
    if ty.is_subtype(exp_zoo.base_exception_type) {
        flags |= PY_TPFLAGS_BASE_EXC_SUBCLASS;
    }
    if ty.is_subtype(zoo.type_type) {
        flags |= PY_TPFLAGS_TYPE_SUBCLASS;
    }

    flags
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetName(ptr: *const Py<PyType>) -> *mut PyObject {
    let ty = unsafe { &*ptr };
    with_vm(move |vm| ty.__name__(vm).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetQualName(ptr: *const Py<PyType>) -> *mut PyObject {
    let ty = unsafe { &*ptr };
    with_vm(move |vm| ty.__qualname__(vm).into_object().into_raw().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetAttr(_obj: *mut PyObject, _name: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyObject_GetAttr");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Repr(_obj: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyObject_Repr");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Str(_obj: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyObject_Str");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut PyObject {
    with_vm(|vm| {
        let ctx = &vm.ctx;
        match constant_id {
            0 => ctx.none.as_object(),
            1 => ctx.false_value.as_object(),
            2 => ctx.true_value.as_object(),
            3 => ctx.ellipsis.as_object(),
            4 => ctx.not_implemented.as_object(),
            _ => panic!("Invalid constant_id passed to Py_GetConstantBorrowed"),
        }
        .as_raw()
        .cast_mut()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.to_owned().is_true(vm).map_or_else(
            |err| {
                vm.push_exception(Some(err));
                -1
            },
            |is_true| is_true.into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyNone, PyString};

    #[test]
    fn test_is_truthy() {
        Python::attach(|py| {
            assert!(!py.None().is_truthy(py).unwrap());
        })
    }

    #[test]
    fn test_is_none() {
        Python::attach(|py| {
            assert!(py.None().is_none(py));
        })
    }

    #[test]
    fn test_bool() {
        Python::attach(|py| {
            assert!(PyBool::new(py, true).is_truthy().unwrap());
            assert!(!PyBool::new(py, false).is_truthy().unwrap());
        })
    }

    #[test]
    fn test_type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_str().unwrap(), "str");
        })
    }

    #[test]
    #[ignore = "Instance checking on static type pointers is yet supported"]
    fn test_static_type_pointers() {
        Python::attach(|py| {
            assert!(py.None().bind(py).is_instance_of::<PyNone>());
            assert!(PyBool::new(py, true).is_instance_of::<PyBool>());
        })
    }
}

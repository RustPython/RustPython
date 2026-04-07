use crate::PyObject;
use core::ffi::c_ulong;
use std::mem::MaybeUninit;
use rustpython_vm::builtins::PyType;
use rustpython_vm::{Context, Py};
use crate::pylifecycle::INITIALIZED;

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


/// Initialize the static type pointers. This should be called once during interpreter initialization,
/// and before any of the static type pointers are used.
///
/// Panics:
/// Panics when the interpreter is already initialized.
#[allow(static_mut_refs)]
pub(crate) fn init_static_type_pointers() {
    assert!(!INITIALIZED.is_completed(), "Python already initialized, we should not touch the static type pointers");
    let zoo = &Context::genesis().types;
    unsafe {
        PyType_Type.write(zoo.type_type);
        PyLong_Type.write(zoo.int_type);
        PyTuple_Type.write(zoo.tuple_type);
        PyUnicode_Type.write(zoo.str_type);
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *const Py<PyType> {
    // SAFETY: The caller must guarantee that `op` is a valid pointer to a `PyObject`.
    unsafe { (*op).class() }
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn PyType_GetFlags(ptr: *const Py<PyType>) -> c_ulong {
    let ctx =Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;

    // SAFETY: The caller must guarantee that `ptr` is a valid pointer to a `PyType` object.
    let ty = unsafe { &*ptr};
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
pub extern "C" fn PyType_GetName(_ptr: *const Py<PyType>) -> *mut PyObject {
    crate::log_stub("PyType_GetName");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetQualName(_ptr: *const Py<PyType>) -> *mut PyObject {
    crate::log_stub("PyType_GetQualName");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_CallNoArgs(_callable: *mut PyObject) -> *mut PyObject {
    crate::log_stub("PyObject_CallNoArgs");
    std::ptr::null_mut()
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
pub extern "C" fn Py_GetConstantBorrowed(_constant_id: core::ffi::c_uint) -> *mut PyObject {
    crate::log_stub("Py_GetConstantBorrowed");
    std::ptr::null_mut()
}

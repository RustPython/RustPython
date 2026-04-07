use crate::PyObject;
use core::ffi::c_ulong;
use rustpython_vm::builtins::PyType;
use rustpython_vm::{AsObject, Context, Py};
use std::sync::LazyLock;

pub struct PyTypeObject {
    ty: LazyLock<&'static Py<PyType>>,
}

impl PyTypeObject {
    const fn new(f: fn() -> &'static Py<PyType>) -> PyTypeObject {
        PyTypeObject {
            ty: LazyLock::new(f),
        }
    }
}

const PY_TPFLAGS_LONG_SUBCLASS: c_ulong = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: c_ulong = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: c_ulong = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: c_ulong = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: c_ulong = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: c_ulong = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: c_ulong = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: c_ulong = 1 << 31;

#[unsafe(no_mangle)]
pub static mut PyType_Type: PyTypeObject = PyTypeObject::new(|| {
    let zoo = &Context::genesis().types;
    zoo.type_type
});

#[unsafe(no_mangle)]
pub static mut PyLong_Type: PyTypeObject = PyTypeObject::new(|| {
    let zoo = &Context::genesis().types;
    zoo.int_type
});

#[unsafe(no_mangle)]
pub static mut PyTuple_Type: PyTypeObject = PyTypeObject::new(|| {
    let zoo = &Context::genesis().types;
    zoo.tuple_type
});

#[unsafe(no_mangle)]
pub static mut PyUnicode_Type: PyTypeObject = PyTypeObject::new(|| {
    let zoo = &Context::genesis().types;
    zoo.str_type
});

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *mut PyTypeObject {
    if op.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let ty = (*op).class();
        if ty.is(*PyType_Type.ty) {
            &raw mut PyType_Type
        } else if ty.is(*PyLong_Type.ty) {
            &raw mut PyLong_Type
        } else if ty.is(*PyTuple_Type.ty) {
            &raw mut PyTuple_Type
        } else if ty.is(*PyUnicode_Type.ty) {
            &raw mut PyUnicode_Type
        } else {
            todo!("Unsupported type: {:?}", ty.name());
        }
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn PyType_GetFlags(ty: *mut PyTypeObject) -> c_ulong {
    if ty.is_null() {
        panic!("PyType_GetFlags called with null type pointer");
    }

    let ctx =Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;
    let ty_inner = unsafe { *(*ty).ty };
    let mut flags = ty_inner.slots.flags.bits();

    if ty_inner.is_subtype(zoo.int_type) {
        flags |= PY_TPFLAGS_LONG_SUBCLASS;
    }
    if ty_inner.is_subtype(zoo.list_type) {
        flags |= PY_TPFLAGS_LIST_SUBCLASS
    }
    if ty_inner.is_subtype(zoo.tuple_type) {
        flags |= PY_TPFLAGS_TUPLE_SUBCLASS;
    }
    if ty_inner.is_subtype(zoo.bytes_type) {
        flags |= PY_TPFLAGS_BYTES_SUBCLASS;
    }
    if ty_inner.is_subtype(zoo.str_type) {
        flags |= PY_TPFLAGS_UNICODE_SUBCLASS;
    }
    if ty_inner.is_subtype(zoo.dict_type) {
        flags |= PY_TPFLAGS_DICT_SUBCLASS;
    }
    if ty_inner.is_subtype(exp_zoo.base_exception_type) {
        flags |= PY_TPFLAGS_BASE_EXC_SUBCLASS;
    }
    if ty_inner.is_subtype(zoo.type_type) {
        flags |= PY_TPFLAGS_TYPE_SUBCLASS;
    }

    flags
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetName(_ty: *mut PyTypeObject) -> *mut PyObject {
    crate::log_stub("PyType_GetName");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetQualName(_ty: *mut PyTypeObject) -> *mut PyObject {
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

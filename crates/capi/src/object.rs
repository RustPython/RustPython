use std::sync::LazyLock;
use rustpython_vm::{Context, AsObject, Py};
use rustpython_vm::builtins::PyType;
use crate::{PyObject};



pub struct  PyTypeObject {
    ty: LazyLock<&'static Py<PyType>>,
}

impl PyTypeObject {
    const fn new(f: fn() -> &'static Py<PyType>) -> PyTypeObject
    {
        PyTypeObject {
            ty: LazyLock::new(f),
        }
    }
}

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
    zoo.union_type
});

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C-unwind" fn _Py_TYPE(op: *mut PyObject) -> *mut PyTypeObject {
    if op.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let ty = (*op).class();
        if ty.is(*PyTuple_Type.ty) {
             &raw mut PyType_Type
        } else if ty.is(*PyTuple_Type.ty){
            &raw mut PyLong_Type
        } else if ty.is(*PyTuple_Type.ty){
            &raw mut PyTuple_Type
        } else if ty.is(*PyTuple_Type.ty){
            &raw mut PyUnicode_Type
        } else {
            todo!("Unsupported type: {:?}", ty.name());
        }
    }

}

#[unsafe(no_mangle)]
pub extern "C-unwind" fn Py_TYPE(op: *mut PyObject) -> *mut PyTypeObject {
    _Py_TYPE(op)
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetFlags(_ty: *mut PyTypeObject) -> usize {
    crate::log_stub("PyType_GetFlags");
    0
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

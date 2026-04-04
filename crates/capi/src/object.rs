use std::mem::{transmute, MaybeUninit};
use std::ptr::NonNull;
use rustpython_vm::{PyObjectRef, VirtualMachine, Context};
use rustpython_vm::builtins::PyType;
use crate::{PyObject};

type PyTypeObject = MaybeUninit<&'static PyType>;

#[unsafe(no_mangle)]
pub static mut PyType_Type: PyTypeObject = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyLong_Type: PyTypeObject = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyTuple_Type: PyTypeObject = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyUnicode_Type: PyTypeObject = MaybeUninit::uninit();

unsafe fn setup_type_pointers(ctx: &Context) {
    let zoo = &ctx.types;

    unsafe {
        PyType_Type.write(zoo.type_type.payload());
        PyLong_Type.write(zoo.int_type.payload());
        PyTuple_Type.write(zoo.tuple_type.payload());
        PyUnicode_Type.write(zoo.str_type.payload());
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn _Py_TYPE(op: *mut PyObject) -> *mut PyTypeObject {
    if op.is_null() {
        return std::ptr::null_mut();
    }

    // SAFETY: op is non-null and expected to be a valid pointer for this shim.
    unsafe { transmute((*op).class()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *mut PyTypeObject {
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

use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::{c_int, c_uint, c_ulong};
use rustpython_vm::builtins::PyType;
use rustpython_vm::{AsObject, Context, Py};

const PY_TPFLAGS_LONG_SUBCLASS: u32 = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: u32 = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: u32 = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: u32 = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: u32 = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: u32 = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: u32 = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: u32 = 1 << 31;

pub type PyTypeObject = Py<PyType>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    unsafe { (*op).class() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { &*op };
        let ty = unsafe { &*ty };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ptr: *const PyTypeObject) -> c_ulong {
    let ctx = Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;

    let ty = unsafe { &*ptr };
    let mut flags = ty.slots.flags.bits() as u32;

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

    flags as c_ulong
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut PyObject {
    with_vm(|vm| {
        let ctx = &vm.ctx;
        let constant = match constant_id {
            0 => ctx.none.as_object(),
            1 => ctx.false_value.as_object(),
            2 => ctx.true_value.as_object(),
            3 => ctx.ellipsis.as_object(),
            4 => ctx.not_implemented.as_object(),
            _ => {
                return Err(
                    vm.new_system_error("Invalid constant ID passed to Py_GetConstantBorrowed")
                );
            }
        }
        .as_raw();
        Ok(constant)
    })
}

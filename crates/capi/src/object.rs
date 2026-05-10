use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::{AsObject, Py};

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
    let ty = unsafe { &*ptr };
    ty.slots.flags.bits() as u32 as c_ulong
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe {
            CStr::from_ptr(attr_name)
                .to_str()
                .expect("attribute name must be valid UTF-8")
        };
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { CStr::from_ptr(attr_name) }
            .to_str()
            .expect("attribute name must be valid UTF-8");
        let value = unsafe { &*value }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        let value = unsafe { &*value }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Repr(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(obj) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.repr(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Str(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(obj) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.str(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.to_owned().is_true(vm)
    })
}

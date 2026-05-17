use crate::{PyObject, pystate::with_vm};
use core::ffi::c_int;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetItem(obj: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.get_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetItem(
    obj: *mut PyObject,
    key: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        let value = unsafe { &*value }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItem(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsSubclass(derived: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let derived = unsafe { &*derived };
        let cls = unsafe { &*cls };
        derived.is_subclass(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsInstance(inst: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let inst = unsafe { &*inst };
        let cls = unsafe { &*cls };
        inst.is_instance(cls, vm)
    })
}

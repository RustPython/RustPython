use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::{c_char, c_int, c_void};
use rustpython_vm::builtins::PyCapsule;

#[allow(non_camel_case_types)]
pub type PyCapsule_Destructor = unsafe extern "C" fn(capsule: *mut PyObject);

#[unsafe(no_mangle)]
pub extern "C" fn PyCapsule_New(
    pointer: *mut c_void,
    _name: *const c_char,
    destructor: Option<PyCapsule_Destructor>,
) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_capsule(pointer, destructor))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCapsule_GetPointer(
    capsule: *mut PyObject,
    _name: *const c_char,
) -> *mut c_void {
    with_vm(|vm| {
        let capsule = unsafe { &*capsule }.try_downcast_ref::<PyCapsule>(vm)?;
        Ok(capsule.pointer())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCapsule_IsValid(capsule: *mut PyObject, _name: *const c_char) -> c_int {
    with_vm(|vm| {
        let Some(capsule) =
            unsafe { capsule.as_ref() }.and_then(|obj| obj.downcast_ref_if_exact::<PyCapsule>(vm))
        else {
            return false;
        };

        !capsule.pointer().is_null()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyCapsule_GetName(_capsule: *mut PyObject) -> *const c_char {
    core::ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyCapsule;

    #[test]
    fn test_capsule_new() {
        Python::attach(|py| {
            let value = String::from("Some data");
            let capsule = PyCapsule::new_with_value(py, value, c"my_capsule").unwrap();
            assert!(capsule.is_valid_checked(Some(c"my_capsule")));
            let ptr = capsule.pointer_checked(Some(c"my_capsule")).unwrap();
            assert_eq!(unsafe { ptr.cast::<String>().as_ref() }, "Some data");
        })
    }
}

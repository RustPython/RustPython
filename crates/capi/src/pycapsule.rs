use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::NonNull;
use rustpython_vm::builtins::PyCapsule;
use rustpython_vm::{PyObjectRef, PyResult, VirtualMachine};

#[allow(non_camel_case_types)]
pub type PyCapsule_Destructor = unsafe extern "C" fn(capsule: *mut PyObject);

#[unsafe(no_mangle)]
pub extern "C" fn PyCapsule_New(
    pointer: *mut c_void,
    name: *const c_char,
    destructor: Option<PyCapsule_Destructor>,
) -> *mut PyObject {
    with_vm(|vm| {
        if pointer.is_null() {
            return Err(vm.new_value_error("PyCapsule_New called with null pointer"));
        }
        let name = NonNull::new(name.cast_mut()).map(|ptr| unsafe { CStr::from_ptr(ptr.as_ptr()) });
        Ok(vm.ctx.new_capsule(pointer, name, destructor))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_GetPointer(
    capsule: *mut PyObject,
    name: *const c_char,
) -> *mut c_void {
    with_vm(|vm| Ok(checked_capsule(vm, unsafe { &*capsule }, name)?.pointer()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_GetName(capsule: *mut PyObject) -> *const c_char {
    with_vm(|vm| {
        let capsule = unsafe { &*capsule }
            .downcast_ref_if_exact::<PyCapsule>(vm)
            .ok_or_else(|| vm.new_value_error("Invalid capsule"))?;
        Ok(capsule.name().map(CStr::as_ptr).unwrap_or_default())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_GetContext(capsule: *mut PyObject) -> *mut c_void {
    with_vm(|vm| {
        let capsule = unsafe { &*capsule }
            .downcast_ref_if_exact::<PyCapsule>(vm)
            .ok_or_else(|| vm.new_value_error("Invalid capsule"))?;
        Ok(capsule.context())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetContext(
    capsule: *mut PyObject,
    context: *mut c_void,
) -> c_int {
    with_vm(|vm| {
        let capsule = unsafe { &*capsule }
            .downcast_ref_if_exact::<PyCapsule>(vm)
            .ok_or_else(|| vm.new_value_error("Invalid capsule"))?;
        let _: () = capsule.set_context(context);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_SetPointer(
    capsule: *mut PyObject,
    pointer: *mut c_void,
) -> c_int {
    with_vm(|vm| {
        let capsule = unsafe { &*capsule }
            .downcast_ref_if_exact::<PyCapsule>(vm)
            .ok_or_else(|| vm.new_value_error("Invalid capsule"))?;
        let _: () = capsule.set_pointer(pointer);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_IsValid(capsule: *mut PyObject, name: *const c_char) -> c_int {
    with_vm(|vm| {
        if capsule.is_null() {
            return false;
        }

        checked_capsule(vm, unsafe { &*capsule }, name).is_ok()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCapsule_Import(name: *const c_char, _no_block: c_int) -> *mut c_void {
    with_vm(|vm| {
        let capsule_name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .map_err(|_| vm.new_system_error("capsule name is not valid UTF-8"))?;
        let (module_name, attrs_path) = capsule_name.split_once('.').ok_or_else(|| {
            vm.new_import_error(
                "capsule name is missing attribute path",
                vm.ctx.new_str(capsule_name),
            )
        })?;
        let mut obj: PyObjectRef = vm.import(module_name, 0)?;

        for attr in attrs_path.split('.') {
            obj = obj.get_attr(attr, vm)?;
        }

        Ok(checked_capsule(vm, &obj, name)?.pointer())
    })
}

#[inline]
fn names_match(stored_name: *const c_char, expected_name: *const c_char) -> bool {
    if stored_name.is_null() || expected_name.is_null() {
        stored_name.is_null() && expected_name.is_null()
    } else {
        unsafe { CStr::from_ptr(stored_name) == CStr::from_ptr(expected_name) }
    }
}

#[inline]
fn checked_capsule<'a>(
    vm: &VirtualMachine,
    obj: &'a PyObject,
    name: *const c_char,
) -> PyResult<&'a PyCapsule> {
    let capsule = obj
        .downcast_ref_if_exact::<PyCapsule>(vm)
        .ok_or_else(|| vm.new_value_error("Invalid capsule"))?;

    if !names_match(capsule.name().map(CStr::as_ptr).unwrap_or_default(), name) {
        return Err(vm.new_value_error("Capsule name does not match"));
    }

    if capsule.pointer().is_null() {
        return Err(vm.new_value_error("Capsule has null pointer"));
    }

    Ok(capsule)
}

#[cfg(false)]
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

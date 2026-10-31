use crate::util::CStrExt;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{c_char, c_int};
use rustpython_vm::AsObject;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Check(obj: *mut PyObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { obj.assume_borrowed() };
        Ok(obj.mapping_unchecked().check())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_mapping(vm)?.length(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Length(obj: *mut PyObject) -> isize {
    unsafe { PyMapping_Size(obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Keys(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let keys = obj.try_mapping(vm)?.keys(vm)?;
        let iter = keys.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Values(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let values = obj.try_mapping(vm)?.values(vm)?;
        let iter = values.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Items(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let items = obj.try_mapping(vm)?.items(vm)?;
        let iter = items.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetItemString(
    obj: *mut PyObject,
    key: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.try_as_str(vm) }?;
        obj.get_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItem(
    obj: *mut PyObject,
    key: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *result = core::ptr::null_mut();
        }
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };

        match obj.get_item(key, vm) {
            Ok(value) => {
                unsafe {
                    *result = value.into_raw().as_ptr();
                }
                Ok(true)
            }
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(false),
            Err(err) => Err(err),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_GetOptionalItemString(
    obj: *mut PyObject,
    key: *const c_char,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *result = core::ptr::null_mut();
        }
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.try_as_str(vm) }?;

        match obj.get_item(key, vm) {
            Ok(value) => {
                unsafe {
                    *result = value.into_raw().as_ptr();
                }
                Ok(true)
            }
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(false),
            Err(err) => Err(err),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKey(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };
        obj.get_item(key, vm).is_ok()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyString(obj: *mut PyObject, key: *const c_char) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        if let Ok(key) = unsafe { key.try_as_str(vm) } {
            obj.get_item(key, vm).is_ok()
        } else {
            false
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyWithError(
    obj: *mut PyObject,
    key: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };

        match obj.get_item(key, vm) {
            Ok(_) => Ok(true),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(false),
            Err(err) => Err(err),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_HasKeyStringWithError(
    obj: *mut PyObject,
    key: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.try_as_str(vm) }?;

        match obj.get_item(key, vm) {
            Ok(_) => Ok(true),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(false),
            Err(err) => Err(err),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_SetItemString(
    obj: *mut PyObject,
    key: *const c_char,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.try_as_str(vm) }?;
        let value = unsafe { value.assume_borrowed() }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyMapping, PyMappingMethods, PyTuple};

    #[test]
    fn size_keys_values_items() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("a", 1).unwrap();
            dict.set_item("b", 2).unwrap();
            let mapping = dict.cast_into::<PyMapping>().unwrap();

            assert_eq!(mapping.len().unwrap(), 2);

            let keys = mapping.keys().unwrap();
            assert_eq!(keys.len(), 2);

            let values = mapping.values().unwrap();
            assert_eq!(values.len(), 2);

            let items = mapping.items().unwrap();
            assert_eq!(items.len(), 2);
            assert!(items.iter().all(|item| item.cast_into::<PyTuple>().is_ok()));
        })
    }
}

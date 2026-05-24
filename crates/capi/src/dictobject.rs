use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use core::ptr::NonNull;
use rustpython_vm::AsObject;
use rustpython_vm::builtins::PyDict;

define_py_check!(fn PyDict_Check, types.dict_type);
define_py_check!(exact fn PyDict_CheckExact, types.dict_type);
define_py_check!(fn PyDictKeys_Check, types.dict_keys_type);
define_py_check!(fn PyDictValues_Check, types.dict_values_type);
define_py_check!(fn PyDictItems_Check, types.dict_items_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_New() -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_dict())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItem(
    dict: *mut PyObject,
    key: *mut PyObject,
    val: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*key };
        let value = unsafe { &*val }.to_owned();
        dict.inner_setitem(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemRef(
    dict: *mut PyObject,
    key: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe { *result = core::ptr::null_mut() };
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*key };

        if let Some(value) = dict.inner_getitem_opt(key, vm)? {
            unsafe {
                *result = value.into_raw().as_ptr();
            }
            Ok(true)
        } else {
            unsafe {
                *result = core::ptr::null_mut();
            }
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Size(dict: *mut PyObject) -> isize {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        Ok(dict.__len__())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Next(
    dict: *mut PyObject,
    pos: *mut isize,
    key: *mut *mut PyObject,
    value: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let index = unsafe { *pos } as usize;

        if let Some((next_pos, k, v)) = dict.next_entry(index) {
            unsafe {
                *pos = next_pos as isize;
                if let Some(key) = NonNull::new(key) {
                    key.write(k.as_object().as_raw().cast_mut());
                }
                if let Some(value) = NonNull::new(value) {
                    value.write(v.as_object().as_raw().cast_mut());
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{IntoPyDict, PyDict, PyInt};

    #[test]
    fn test_create_empty_dict() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            assert!(dict.is_instance_of::<PyDict>());
        })
    }

    #[test]
    fn test_create_dict_with_items() {
        Python::attach(|py| {
            let dict = [(1, 2), (3, 4)].into_py_dict(py)?;
            let value = dict.get_item(1)?.unwrap().cast_into::<PyInt>()?;
            assert_eq!(value, 2);
            assert_eq!(dict.len(), 2);

            Ok::<_, PyErr>(())
        })
        .unwrap()
    }

    #[test]
    fn test_dict_iter() {
        Python::attach(|py| {
            let dict = [(1, 2), (3, 4)].into_py_dict(py).unwrap();
            let values = dict
                .into_iter()
                .flat_map(|(k, v)| [k.extract().unwrap(), v.extract().unwrap()])
                .collect::<Vec<u32>>();
            assert_eq!(values, vec![1, 2, 3, 4]);
        })
    }
}

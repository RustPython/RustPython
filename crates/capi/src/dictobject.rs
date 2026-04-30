use crate::PyObject;
use crate::pystate::with_vm;
use crate::handles::{exported_object_handle, resolve_object_handle};
use core::ffi::c_int;
use rustpython_vm::AsObject;
use rustpython_vm::builtins::PyDict;

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_New() -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_dict())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_SetItem(
    dict: *mut PyObject,
    key: *mut PyObject,
    val: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*resolve_object_handle(key) };
        let value = unsafe { &*resolve_object_handle(val) }.to_owned();
        dict.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_GetItemRef(
    dict: *mut PyObject,
    key: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*resolve_object_handle(key) };

        if let Some(value) = dict.get_item_opt(key, vm)? {
            unsafe {
                *result = exported_object_handle(value.into_raw().as_ptr());
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
pub extern "C" fn PyDict_Size(dict: *mut PyObject) -> isize {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        Ok(dict.__len__())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_Next(
    dict: *mut PyObject,
    pos: *mut isize,
    key: *mut *mut PyObject,
    value: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let index = unsafe { *pos } as usize;
        let items = dict.items_vec();

        if let Some((k, v)) = items.get(index) {
            unsafe {
                *key = exported_object_handle(k.as_object().as_raw().cast_mut());
                *value = exported_object_handle(v.as_object().as_raw().cast_mut());
                *pos += 1;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_GetItemWithError(dict: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*resolve_object_handle(key) };
        Ok(dict
            .get_item_opt(key, vm)?
            .map(|obj| unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) })
            .unwrap_or(core::ptr::null_mut()))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_Keys(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let keys = dict
            .items_vec()
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        Ok(vm.ctx.new_list(keys))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_Values(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let values = dict
            .items_vec()
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        Ok(vm.ctx.new_list(values))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_Items(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*resolve_object_handle(dict) }.try_downcast_ref::<PyDict>(vm)?;
        let items = dict
            .items_vec()
            .into_iter()
            .map(|(key, value)| vm.ctx.new_tuple(vec![key, value]).into())
            .collect::<Vec<_>>();
        Ok(vm.ctx.new_list(items))
    })
}

#[cfg(test)]
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

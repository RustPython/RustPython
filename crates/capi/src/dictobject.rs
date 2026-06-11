use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use core::ptr::NonNull;
use rustpython_vm::AsObject;
use rustpython_vm::PyPayload;
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
pub unsafe extern "C" fn PyDict_Contains(dict: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*key };
        Ok(dict.inner_getitem_opt(key, vm)?.is_some())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Copy(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        Ok(dict.copy().into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItem(dict: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let key = unsafe { &*key };
        dict.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Items(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let items = dict
            .items_vec()
            .into_iter()
            .map(|(k, v)| vm.ctx.new_tuple(vec![k, v]).into())
            .collect();
        Ok(vm.ctx.new_list(items))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Keys(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        Ok(vm.ctx.new_list(dict.keys_vec()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Values(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        Ok(vm.ctx.new_list(dict.values_vec()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Merge(
    dict: *mut PyObject,
    other: *mut PyObject,
    override_: c_int,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let other = unsafe { &*other }.to_owned();
        if override_ != 0 {
            dict.merge_object(other, vm)
        } else {
            dict.merge_object_if_missing(other, vm)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Update(dict: *mut PyObject, other: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let other = unsafe { &*other }.to_owned();
        dict.merge_object(other, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_MergeFromSeq2(
    dict: *mut PyObject,
    seq2: *mut PyObject,
    override_: c_int,
) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { &*dict }.try_downcast_ref::<PyDict>(vm)?;
        let seq2 = unsafe { &*seq2 }.to_owned();
        dict.merge_from_seq2(seq2, override_ != 0, vm)
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
    use pyo3::types::{IntoPyDict, PyDict, PyDictMethods, PyInt, PyList};

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

    #[test]
    fn dict_contains() {
        Python::attach(|py| {
            let dict = [(1, 2)].into_py_dict(py).unwrap();
            assert!(dict.contains(1).unwrap());
            assert!(!dict.contains(3).unwrap());
        })
    }

    #[test]
    fn dict_copy_and_del_item() {
        Python::attach(|py| {
            let dict = [(1, 2), (3, 4)].into_py_dict(py).unwrap();
            let copied = dict.copy().unwrap();
            assert_eq!(copied.len(), 2);
            copied.del_item(1).unwrap();
            assert!(!copied.contains(1).unwrap());
        })
    }

    #[test]
    fn dict_keys_values_items() {
        Python::attach(|py| {
            let dict = [(1, 2), (3, 4)].into_py_dict(py).unwrap();
            assert_eq!(dict.keys().len(), 2);
            assert_eq!(dict.values().len(), 2);
            assert_eq!(dict.items().len(), 2);
        })
    }

    #[test]
    fn dict_update_and_merge() {
        Python::attach(|py| {
            let dict = [(1, 10)].into_py_dict(py).unwrap();
            let replacement = [(1, 20), (2, 30)].into_py_dict(py).unwrap();
            dict.update(replacement.as_mapping()).unwrap();
            assert_eq!(
                dict.get_item(1).unwrap().unwrap().extract::<i32>().unwrap(),
                20
            );
            assert_eq!(
                dict.get_item(2).unwrap().unwrap().extract::<i32>().unwrap(),
                30
            );

            let merged_missing = [(1, 99), (3, 40)].into_py_dict(py).unwrap();
            dict.update_if_missing(merged_missing.as_mapping()).unwrap();
            assert_eq!(
                dict.get_item(1).unwrap().unwrap().extract::<i32>().unwrap(),
                20
            );
            assert_eq!(
                dict.get_item(3).unwrap().unwrap().extract::<i32>().unwrap(),
                40
            );
        })
    }

    #[test]
    fn dict_merge_from_seq2() {
        Python::attach(|py| {
            let seq = PyList::new(py, [(1, 10), (1, 20), (2, 30)]).unwrap();
            let dict = PyDict::from_sequence(seq.as_any()).unwrap();
            assert_eq!(
                dict.get_item(1).unwrap().unwrap().extract::<i32>().unwrap(),
                20
            );
            assert_eq!(
                dict.get_item(2).unwrap().unwrap().extract::<i32>().unwrap(),
                30
            );
        })
    }
}

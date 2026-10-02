use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyAnyDictRef, PyDict, PyFrozenDict};
use rustpython_vm::{AsObject, Py, PyPayload, PyResult, VirtualMachine};

define_py_check!(fn PyDict_Check, types.dict_type);
define_py_check!(exact fn PyDict_CheckExact, types.dict_type);
define_py_check!(fn PyDictKeys_Check, types.dict_keys_type);
define_py_check!(fn PyDictValues_Check, types.dict_values_type);
define_py_check!(fn PyDictItems_Check, types.dict_items_type);

fn any_dict(dict: &PyObject, vm: &VirtualMachine) -> PyResult<PyAnyDictRef> {
    PyAnyDictRef::from_object(dict)
        .ok_or_else(|| vm.new_system_error("bad argument to internal function"))
}

fn writable_dict<'a>(
    dict: &'a PyObject,
    operation: &str,
    vm: &VirtualMachine,
) -> PyResult<&'a Py<PyDict>> {
    dict.downcast_ref::<PyDict>().ok_or_else(|| {
        if dict.downcast_ref::<PyFrozenDict>().is_some() {
            vm.new_type_error(format!(
                "frozendict object does not support item {operation}"
            ))
        } else {
            vm.new_system_error("bad argument to internal function")
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyDict_New() -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_dict())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Clear(dict: *mut PyObject) {
    with_vm(|_vm| {
        if let Some(dict) = unsafe { dict.assume_borrowed() }.downcast_ref::<PyDict>() {
            dict.clear();
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItem(
    dict: *mut PyObject,
    key: *mut PyObject,
    val: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let key = unsafe { key.assume_borrowed() };
        let value = unsafe { val.assume_borrowed() }.to_owned();
        dict.inner_setitem(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_SetItemString(
    dict: *mut PyObject,
    key: *const c_char,
    val: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let key = unsafe { key.try_as_str(vm) }?;
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let value = unsafe { val.assume_borrowed() }.to_owned();
        dict.inner_setitem(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItem(dict: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(dict) = PyAnyDictRef::from_object(unsafe { dict.assume_borrowed() }) else {
            return core::ptr::null_mut();
        };
        let key = unsafe { key.assume_borrowed() };

        match dict.inner_getitem_opt(key, vm) {
            Ok(Some(value)) => value.as_object().as_raw().cast_mut(),
            Ok(None) | Err(_) => core::ptr::null_mut(),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemWithError(
    dict: *mut PyObject,
    key: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let dict = any_dict(unsafe { dict.assume_borrowed() }, vm)?;
        let key = unsafe { key.assume_borrowed() };

        if let Some(value) = dict.inner_getitem_opt(key, vm)? {
            Ok(value.as_object().as_raw().cast_mut())
        } else {
            Ok(core::ptr::null_mut())
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemString(
    dict: *mut PyObject,
    key: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let Ok(key) = (unsafe { key.try_as_str(vm) }) else {
            return core::ptr::null_mut();
        };
        let Some(dict) = PyAnyDictRef::from_object(unsafe { dict.assume_borrowed() }) else {
            return core::ptr::null_mut();
        };

        match dict.inner_getitem_opt(key, vm) {
            Ok(Some(value)) => value.as_object().as_raw().cast_mut(),
            Ok(None) | Err(_) => core::ptr::null_mut(),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_GetItemStringRef(
    dict: *mut PyObject,
    key: *const c_char,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *result = core::ptr::null_mut();
        }
        let key = unsafe { key.try_as_str(vm) }?;
        let dict = any_dict(unsafe { dict.assume_borrowed() }, vm)?;

        if let Some(value) = dict.inner_getitem_opt(key, vm)? {
            unsafe {
                *result = value.into_raw().as_ptr();
            }
            Ok(true)
        } else {
            Ok(false)
        }
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
        let dict = any_dict(unsafe { dict.assume_borrowed() }, vm)?;
        let key = unsafe { key.assume_borrowed() };

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
pub unsafe extern "C" fn PyDict_SetDefaultRef(
    dict: *mut PyObject,
    key: *mut PyObject,
    default_value: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let result = NonNull::new(result);
        if let Some(result) = result {
            unsafe {
                result.write(core::ptr::null_mut());
            }
        }
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let key = unsafe { key.assume_borrowed() };

        if let Some(value) = dict.inner_getitem_opt(key, vm)? {
            if let Some(result) = result {
                unsafe {
                    result.write(value.into_raw().as_ptr());
                }
            }
            Ok(true)
        } else {
            let value = unsafe { default_value.assume_borrowed() }.to_owned();
            dict.inner_setitem(key, value.clone(), vm)?;
            if let Some(result) = result {
                unsafe {
                    result.write(value.into_raw().as_ptr());
                }
            }
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Size(dict: *mut PyObject) -> isize {
    with_vm(|vm| {
        let dict = unsafe { dict.assume_borrowed_or_opt() }
            .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
        let dict = any_dict(dict, vm)?;
        Ok(dict.__len__())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Contains(dict: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let dict = any_dict(unsafe { dict.assume_borrowed() }, vm)?;
        let key = unsafe { key.assume_borrowed() };
        Ok(dict.inner_getitem_opt(key, vm)?.is_some())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Copy(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { dict.assume_borrowed_or_opt() }
            .and_then(|dict| dict.downcast_ref::<PyDict>())
            .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
        Ok(dict.copy().into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItem(dict: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let dict = unsafe { dict.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };
        if dict.downcast_ref::<PyFrozenDict>().is_some() {
            PyFrozenDict::key_hash(key, vm)?;
        }
        let dict = writable_dict(dict, "deletion", vm)?;
        dict.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_DelItemString(dict: *mut PyObject, key: *const c_char) -> c_int {
    with_vm(|vm| {
        let key = unsafe { key.try_as_str(vm) }?;
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "deletion", vm)?;
        dict.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Items(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { dict.assume_borrowed_or_opt() }
            .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
        let dict = any_dict(dict, vm)?;
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
        let dict = unsafe { dict.assume_borrowed_or_opt() }
            .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
        let dict = any_dict(dict, vm)?;
        Ok(vm.ctx.new_list(dict.keys_vec()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDict_Values(dict: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let dict = unsafe { dict.assume_borrowed_or_opt() }
            .ok_or_else(|| vm.new_system_error("bad argument to internal function"))?;
        let dict = any_dict(dict, vm)?;
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
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let other = unsafe { other.assume_borrowed() }.to_owned();
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
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let other = unsafe { other.assume_borrowed() }.to_owned();
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
        let dict = writable_dict(unsafe { dict.assume_borrowed() }, "assignment", vm)?;
        let seq2 = unsafe { seq2.assume_borrowed() };
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
    with_vm(|_vm| {
        let Some(dict) = PyAnyDictRef::from_object(unsafe { dict.assume_borrowed() }) else {
            return false;
        };
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
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{IntoPyDict, PyDict, PyDictMethods, PyInt, PyList};

    #[test]
    fn create_empty_dict() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            assert!(dict.is_instance_of::<PyDict>());
        })
    }

    #[test]
    fn create_dict_with_items() {
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
    fn dict_iter() {
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
    // PyO3 has no frozendict wrapper yet. Keep object ownership in Bound while
    // exercising the existing limited dictionary API with either dictionary type.
    struct DictApi<'py>(Bound<'py, PyAny>);

    impl<'py> DictApi<'py> {
        fn list(
            &self,
            api: unsafe extern "C" fn(*mut pyo3::ffi::PyObject) -> *mut pyo3::ffi::PyObject,
        ) -> PyResult<Bound<'py, PyAny>> {
            unsafe { Bound::from_owned_ptr_or_err(self.0.py(), api(self.0.as_ptr())) }
        }

        fn lookup(
            &self,
            key: &Bound<'py, PyAny>,
            with_error: bool,
        ) -> PyResult<Option<Bound<'py, PyAny>>> {
            let value = unsafe {
                if with_error {
                    pyo3::ffi::PyDict_GetItemWithError(self.0.as_ptr(), key.as_ptr())
                } else {
                    pyo3::ffi::PyDict_GetItem(self.0.as_ptr(), key.as_ptr())
                }
            };
            if let Some(error) = PyErr::take(self.0.py()) {
                return Err(error);
            }
            Ok(unsafe { Bound::from_borrowed_ptr_or_opt(self.0.py(), value) })
        }

        fn lookup_ref(&self, key: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
            let mut value = core::ptr::null_mut();
            let status =
                unsafe { pyo3::ffi::PyDict_GetItemRef(self.0.as_ptr(), key.as_ptr(), &mut value) };
            if status < 0 {
                assert!(value.is_null());
                return Err(PyErr::fetch(self.0.py()));
            }
            Ok(unsafe { Bound::from_owned_ptr_or_opt(self.0.py(), value) })
        }

        fn lookup_string(
            &self,
            key: &core::ffi::CStr,
            owned: bool,
        ) -> PyResult<Option<Bound<'py, PyAny>>> {
            if owned {
                let mut value = core::ptr::null_mut();
                let status = unsafe {
                    pyo3::ffi::PyDict_GetItemStringRef(self.0.as_ptr(), key.as_ptr(), &mut value)
                };
                if status < 0 {
                    assert!(value.is_null());
                    return Err(PyErr::fetch(self.0.py()));
                }
                Ok(unsafe { Bound::from_owned_ptr_or_opt(self.0.py(), value) })
            } else {
                let value =
                    unsafe { pyo3::ffi::PyDict_GetItemString(self.0.as_ptr(), key.as_ptr()) };
                if let Some(error) = PyErr::take(self.0.py()) {
                    return Err(error);
                }
                Ok(unsafe { Bound::from_borrowed_ptr_or_opt(self.0.py(), value) })
            }
        }

        fn size(&self) -> PyResult<usize> {
            let size = unsafe { pyo3::ffi::PyDict_Size(self.0.as_ptr()) };
            if size < 0 {
                Err(PyErr::fetch(self.0.py()))
            } else {
                Ok(size as usize)
            }
        }

        fn contains(&self, key: &Bound<'py, PyAny>) -> PyResult<bool> {
            let status = unsafe { pyo3::ffi::PyDict_Contains(self.0.as_ptr(), key.as_ptr()) };
            if status < 0 {
                Err(PyErr::fetch(self.0.py()))
            } else {
                Ok(status != 0)
            }
        }

        fn entries(&self) -> Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
            let mut entries = Vec::new();
            let mut pos = 0;
            let mut key = core::ptr::null_mut();
            let mut value = core::ptr::null_mut();
            while unsafe { pyo3::ffi::PyDict_Next(self.0.as_ptr(), &mut pos, &mut key, &mut value) }
                != 0
            {
                entries.push(unsafe {
                    (
                        Bound::from_borrowed_ptr(self.0.py(), key),
                        Bound::from_borrowed_ptr(self.0.py(), value),
                    )
                });
            }
            assert!(!PyErr::occurred(self.0.py()));
            entries
        }

        fn clear(&self) {
            unsafe { pyo3::ffi::PyDict_Clear(self.0.as_ptr()) };
            assert!(!PyErr::occurred(self.0.py()));
        }

        fn mutate(
            &self,
            operation: &str,
            key: &Bound<'py, PyAny>,
            value: &Bound<'py, PyAny>,
        ) -> PyResult<()> {
            let status = unsafe {
                match operation {
                    "set" => {
                        pyo3::ffi::PyDict_SetItem(self.0.as_ptr(), key.as_ptr(), value.as_ptr())
                    }
                    "set_string" => pyo3::ffi::PyDict_SetItemString(
                        self.0.as_ptr(),
                        c"x".as_ptr(),
                        value.as_ptr(),
                    ),
                    "delete" => pyo3::ffi::PyDict_DelItem(self.0.as_ptr(), key.as_ptr()),
                    "delete_string" => {
                        pyo3::ffi::PyDict_DelItemString(self.0.as_ptr(), c"x".as_ptr())
                    }
                    "setdefault" => pyo3::ffi::PyDict_SetDefaultRef(
                        self.0.as_ptr(),
                        key.as_ptr(),
                        value.as_ptr(),
                        core::ptr::null_mut(),
                    ),
                    "update" => pyo3::ffi::PyDict_Update(self.0.as_ptr(), value.as_ptr()),
                    "merge" => pyo3::ffi::PyDict_Merge(self.0.as_ptr(), value.as_ptr(), 0),
                    "merge_sequence" => {
                        pyo3::ffi::PyDict_MergeFromSeq2(self.0.as_ptr(), value.as_ptr(), 0)
                    }
                    _ => panic!("unknown dictionary operation"),
                }
            };
            if status < 0 {
                Err(PyErr::fetch(self.0.py()))
            } else {
                Ok(())
            }
        }
    }

    fn frozen_dicts(py: Python<'_>) -> Vec<DictApi<'_>> {
        let globals = PyDict::new(py);
        py.run(
            c"class FrozenSubclass(frozendict):\n    def __getitem__(self, key):\n        raise AssertionError('must bypass overrides')\n    def keys(self):\n        raise AssertionError('must bypass overrides')\n    def __len__(self):\n        raise AssertionError('must bypass overrides')",
            Some(&globals),
            None,
        ).unwrap();
        [
            py.import("builtins")
                .unwrap()
                .getattr("frozendict")
                .unwrap(),
            globals.get_item("FrozenSubclass").unwrap().unwrap(),
        ]
        .into_iter()
        .map(|class| {
            let items = [("x", 1), ("y", 2)].into_py_dict(py).unwrap();
            DictApi(class.call1((items,)).unwrap())
        })
        .collect()
    }

    #[test]
    fn frozendict_read_apis() {
        Python::attach(|py| {
            let key = "x".into_pyobject(py).unwrap().into_any();
            let missing = "missing".into_pyobject(py).unwrap().into_any();
            let unhashable = PyList::empty(py).into_any();
            for dict in frozen_dicts(py) {
                assert!(!dict.0.is_instance_of::<PyDict>());
                assert_eq!(dict.size().unwrap(), 2);
                assert!(dict.contains(&key).unwrap());
                assert!(!dict.contains(&missing).unwrap());
                for with_error in [false, true] {
                    assert_eq!(
                        dict.lookup(&key, with_error)
                            .unwrap()
                            .unwrap()
                            .extract::<i32>()
                            .unwrap(),
                        1
                    );
                    assert!(dict.lookup(&missing, with_error).unwrap().is_none());
                }
                assert_eq!(
                    dict.lookup_ref(&key)
                        .unwrap()
                        .unwrap()
                        .extract::<i32>()
                        .unwrap(),
                    1
                );
                assert!(dict.lookup_ref(&missing).unwrap().is_none());
                assert!(dict.lookup(&unhashable, false).unwrap().is_none());
                assert!(
                    dict.lookup(&unhashable, true)
                        .unwrap_err()
                        .is_instance_of::<pyo3::exceptions::PyTypeError>(py)
                );
                assert!(
                    dict.lookup_ref(&unhashable)
                        .unwrap_err()
                        .is_instance_of::<pyo3::exceptions::PyTypeError>(py)
                );
                for owned in [false, true] {
                    assert_eq!(
                        dict.lookup_string(c"x", owned)
                            .unwrap()
                            .unwrap()
                            .extract::<i32>()
                            .unwrap(),
                        1
                    );
                    assert!(dict.lookup_string(c"missing", owned).unwrap().is_none());
                }
                assert_eq!(
                    dict.list(pyo3::ffi::PyDict_Keys)
                        .unwrap()
                        .extract::<Vec<String>>()
                        .unwrap(),
                    ["x", "y"]
                );
                assert_eq!(
                    dict.list(pyo3::ffi::PyDict_Values)
                        .unwrap()
                        .extract::<Vec<i32>>()
                        .unwrap(),
                    [1, 2]
                );
                assert_eq!(
                    dict.list(pyo3::ffi::PyDict_Items)
                        .unwrap()
                        .extract::<Vec<(String, i32)>>()
                        .unwrap(),
                    [("x".to_owned(), 1), ("y".to_owned(), 2)]
                );
                let entries: Vec<(String, i32)> = dict
                    .entries()
                    .into_iter()
                    .map(|(key, value)| (key.extract().unwrap(), value.extract().unwrap()))
                    .collect();
                assert_eq!(entries, [("x".to_owned(), 1), ("y".to_owned(), 2)]);
            }
        })
    }

    #[test]
    fn frozendict_write_apis() {
        Python::attach(|py| {
            let key = "x".into_pyobject(py).unwrap().into_any();
            let value = PyDict::new(py).into_any();
            let unhashable = PyList::empty(py).into_any();
            for dict in frozen_dicts(py) {
                for operation in [
                    "set",
                    "set_string",
                    "setdefault",
                    "update",
                    "merge",
                    "merge_sequence",
                    "delete",
                    "delete_string",
                ] {
                    let error = dict.mutate(operation, &key, &value).unwrap_err();
                    assert!(error.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
                    let action = if operation.starts_with("delete") {
                        "deletion"
                    } else {
                        "assignment"
                    };
                    assert_eq!(
                        error.value(py).str().unwrap().to_str().unwrap(),
                        format!("frozendict object does not support item {action}")
                    );
                }
                let error = dict.mutate("delete", &unhashable, &value).unwrap_err();
                assert_eq!(
                    error.value(py).str().unwrap().to_str().unwrap(),
                    "cannot use 'list' as a frozendict key (unhashable type: 'list')"
                );
                assert!(
                    dict.list(pyo3::ffi::PyDict_Copy)
                        .unwrap_err()
                        .is_instance_of::<pyo3::exceptions::PySystemError>(py)
                );
                dict.clear();
                assert_eq!(dict.size().unwrap(), 2);
            }
        })
    }

    #[test]
    fn frozendict_merge_source() {
        Python::attach(|py| {
            let source = py.eval(c"frozendict(x=1, y=2)", None, None).unwrap();
            let source = source.cast::<pyo3::types::PyMapping>().unwrap();
            let target = [("x", 0)].into_py_dict(py).unwrap();
            target.update_if_missing(source).unwrap();
            assert_eq!(
                target
                    .get_item("x")
                    .unwrap()
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                0
            );
            assert_eq!(
                target
                    .get_item("y")
                    .unwrap()
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                2
            );
            target.update(source).unwrap();
            assert_eq!(
                target
                    .get_item("x")
                    .unwrap()
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                1
            );
        })
    }
}

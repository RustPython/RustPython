use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use core::ffi::c_int;
use rustpython_vm::builtins::PyList;

define_py_check!(fn PyList_Check, types.list_type);
define_py_check!(exact fn PyList_CheckExact, types.list_type);

#[unsafe(no_mangle)]
pub extern "C" fn PyList_New(size: isize) -> *mut PyObject {
    with_vm(|vm| {
        let capacity = size
            .try_into()
            .map_err(|_| vm.new_system_error("Negative size passed to PyList_New"))?;
        Ok(vm.ctx.new_list(Vec::with_capacity(capacity)))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let list = unsafe { obj.assume_borrowed_and_cast::<PyList>(vm) }?;
        Ok(list.len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetItemRef(obj: *mut PyObject, index: isize) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { obj.assume_borrowed_and_cast::<PyList>(vm) }?;
        // A negative index wraps past the end and is out of range.
        list.get_item(index as usize, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetItem(
    list: *mut PyObject,
    index: isize,
    item: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        let item = unsafe { item.assume_owned() };
        // A negative index wraps past the end and is out of range.
        let index = index as usize;
        {
            let mut list_mut = list.borrow_vec_mut();
            // This is somewhat a hack, we assume that we are populating a list right after PyList_New
            if index == list_mut.len() && list_mut.capacity() > index {
                list_mut.push(item);
                return Ok(());
            }
        }
        list.set_item(index, item, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Append(list: *mut PyObject, item: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        let item = unsafe { item.assume_borrowed() }.to_owned();
        list.append(item);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Insert(
    list: *mut PyObject,
    index: isize,
    item: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        let item = unsafe { item.assume_borrowed() }.to_owned();
        list.insert(index, item);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Reverse(list: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        list.reverse();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_AsTuple(list: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        Ok(list.to_tuple(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetSlice(
    list: *mut PyObject,
    low: isize,
    high: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        Ok(list.get_slice(low, high, vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetSlice(
    list: *mut PyObject,
    low: isize,
    high: isize,
    itemlist: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        let Some(itemlist) = (unsafe { itemlist.assume_borrowed_or_opt() }) else {
            list.del_slice(low, high);
            return Ok(());
        };
        list.set_slice(low, high, itemlist, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Sort(list: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { list.assume_borrowed_and_cast::<PyList>(vm) }?;
        list.sort(vm)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::exceptions::PyIndexError;
    use pyo3::ffi;
    use pyo3::prelude::*;
    use pyo3::types::{PyList, PyListMethods};

    #[test]
    fn create_list() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            assert_eq!(list.len(), 3);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
            assert_eq!(list.get_item(1).unwrap().extract::<u32>().unwrap(), 2);
            assert_eq!(list.get_item(2).unwrap().extract::<u32>().unwrap(), 3);
            assert!(list.get_item(3).is_err());
        })
    }

    #[test]
    fn replace_item_in_list() {
        Python::attach(|py| {
            let list = PyList::new(py, [1]).unwrap();
            assert_eq!(list.len(), 1);
            list.set_item(0, 2).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 2);
        })
    }

    #[test]
    fn set_item_out_of_range() {
        Python::attach(|py| {
            let list = PyList::empty(py);
            assert!(
                list.set_item(0, 1)
                    .unwrap_err()
                    .is_instance_of::<PyIndexError>(py)
            );
        })
    }

    #[test]
    fn list_append() {
        Python::attach(|py| {
            let list = PyList::empty(py);
            assert_eq!(list.len(), 0);
            list.append(1).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
        })
    }

    #[test]
    fn list_insert() {
        Python::attach(|py| {
            let list = PyList::empty(py);
            assert_eq!(list.len(), 0);
            list.insert(0, 1).unwrap();
            assert_eq!(list.len(), 1);
            list.insert(2, 3).unwrap();
            assert_eq!(list.get_item(1).unwrap().extract::<u32>().unwrap(), 3);
        })
    }

    #[test]
    fn list_reverse() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            list.reverse().unwrap();
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 3);
            assert_eq!(list.get_item(2).unwrap().extract::<u32>().unwrap(), 1);
        })
    }

    #[test]
    fn list_as_tuple() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            let tuple = list.to_tuple();
            assert_eq!(tuple.len(), 3);
            assert_eq!(tuple.get_item(0).unwrap().extract::<u32>().unwrap(), 1);

            list.set_item(0, 9).unwrap();
            assert_eq!(tuple.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
        })
    }

    #[test]
    fn list_get_slice() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3, 4]).unwrap();
            let slice = list.get_slice(1, 10);
            assert_eq!(slice.len(), 3);
            assert_eq!(slice.get_item(0).unwrap().extract::<u32>().unwrap(), 2);
            assert_eq!(slice.get_item(2).unwrap().extract::<u32>().unwrap(), 4);
        })
    }

    #[test]
    fn list_set_slice() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3, 4]).unwrap();
            let repl = PyList::new(py, [8, 9]).unwrap();
            list.set_slice(1, 3, repl.as_any()).unwrap();

            assert_eq!(list.len(), 4);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
            assert_eq!(list.get_item(1).unwrap().extract::<u32>().unwrap(), 8);
            assert_eq!(list.get_item(2).unwrap().extract::<u32>().unwrap(), 9);
            assert_eq!(list.get_item(3).unwrap().extract::<u32>().unwrap(), 4);
        })
    }

    #[test]
    fn list_sort() {
        Python::attach(|py| {
            let list = PyList::new(py, [3, 1, 2]).unwrap();
            list.sort().unwrap();
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
            assert_eq!(list.get_item(1).unwrap().extract::<u32>().unwrap(), 2);
            assert_eq!(list.get_item(2).unwrap().extract::<u32>().unwrap(), 3);
        })
    }

    #[test]
    fn list_del_slice() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3, 4]).unwrap();
            list.del_slice(1, 3).unwrap();
            assert_eq!(list.extract::<Vec<u32>>().unwrap(), [1, 4]);
        })
    }

    #[test]
    fn list_negative_indices() {
        Python::attach(|py| unsafe {
            let list = PyList::new(py, [1, 2, 3, 4]).unwrap();

            assert!(ffi::PyList_GetItemRef(list.as_ptr(), -1).is_null());
            assert!(PyErr::take(py).unwrap().is_instance_of::<PyIndexError>(py));

            let slice = Bound::from_owned_ptr(py, ffi::PyList_GetSlice(list.as_ptr(), -1, 2));
            assert_eq!(slice.extract::<Vec<u32>>().unwrap(), [1, 2]);

            let repl = PyList::new(py, [9]).unwrap();
            assert_eq!(ffi::PyList_SetSlice(list.as_ptr(), -1, 1, repl.as_ptr()), 0);
            assert_eq!(list.extract::<Vec<u32>>().unwrap(), [9, 2, 3, 4]);
        })
    }

    #[test]
    fn list_set_slice_self() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2]).unwrap();
            list.set_slice(2, 2, list.as_any()).unwrap();
            assert_eq!(list.extract::<Vec<u32>>().unwrap(), [1, 2, 1, 2]);
        })
    }
}

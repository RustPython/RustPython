use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::c_int;
use core::ptr::NonNull;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyList;

#[unsafe(no_mangle)]
pub extern "C" fn PyList_New(size: isize) -> *mut PyObject {
    with_vm(|vm| vm.ctx.new_list(Vec::with_capacity(size as usize)))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyList_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let list = unsafe { &*obj }.try_downcast_ref::<PyList>(vm)?;
        Ok(list.__len__())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyList_GetItemRef(obj: *mut PyObject, index: isize) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { &*obj }.try_downcast_ref::<PyList>(vm)?;

        list.borrow_vec()
            .get(index as usize)
            .ok_or_else(|| vm.new_index_error(format!("list index out of range: {index}")))
            .map(ToOwned::to_owned)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyList_SetItem(list: *mut PyObject, index: isize, item: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(item)) };

        let mut list_mut = list.borrow_vec_mut();
        match index - list_mut.len() as isize {
            ..0 => {
                list_mut[index as usize] = item;
                Ok(())
            }
            // This is somewhat a hack, we assume that we are populating a list right after PyList_New
            0 if list_mut.capacity() > index as usize => {
                list_mut.push(item);
                Ok(())
            }
            0.. => Err(vm.new_index_error(format!("list assignment index out of range: {index}"))),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyList_Append(list: *mut PyObject, item: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { &*item }.to_owned();
        Ok(list.borrow_vec_mut().push(item))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyList_Insert(list: *mut PyObject, index: isize, item: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { &*item }.to_owned();
        let mut vec = list.borrow_vec_mut();
        if index as usize > vec.len() {
            Err(vm.new_index_error(format!("list index out of range: {index}")))
        } else {
            Ok(vec.insert(index as _, item))
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::exceptions::PyIndexError;
    use pyo3::prelude::*;
    use pyo3::types::PyList;

    #[test]
    fn test_create_list() {
        Python::attach(|py| {
            let list = PyList::new(py, &[1, 2, 3]).unwrap();
            assert_eq!(list.len(), 3);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
            assert_eq!(list.get_item(1).unwrap().extract::<u32>().unwrap(), 2);
            assert_eq!(list.get_item(2).unwrap().extract::<u32>().unwrap(), 3);
            assert!(list.get_item(3).is_err());
        })
    }

    #[test]
    fn test_replace_item_in_list() {
        Python::attach(|py| {
            let list = PyList::new(py, &[1]).unwrap();
            assert_eq!(list.len(), 1);
            list.set_item(0, 2).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 2);
        })
    }

    #[test]
    fn test_set_item_out_of_range() {
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
    fn test_list_append() {
        Python::attach(|py| {
            let list = PyList::empty(py);
            assert_eq!(list.len(), 0);
            list.append(1).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list.get_item(0).unwrap().extract::<u32>().unwrap(), 1);
        })
    }

    #[test]
    fn test_list_insert() {
        Python::attach(|py| {
            let list = PyList::empty(py);
            assert_eq!(list.len(), 0);
            list.insert(0, 1).unwrap();
            assert_eq!(list.len(), 1);
            assert!(list.insert(2, 3).is_err());
        })
    }
}

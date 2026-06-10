use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use core::ptr::NonNull;
use rustpython_vm::AsObject;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyList;
use rustpython_vm::sliceable::{SaturatedSlice, SliceableSequenceMutOp, SliceableSequenceOp};

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
        let list = unsafe { &*obj }.try_downcast_ref::<PyList>(vm)?;
        Ok(list.__len__())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetItemRef(obj: *mut PyObject, index: isize) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { &*obj }.try_downcast_ref::<PyList>(vm)?;
        index
            .try_into()
            .ok()
            .and_then(|index: usize| list.borrow_vec().get(index).map(ToOwned::to_owned))
            .ok_or_else(|| vm.new_index_error(format!("list index out of range: {index}")))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_SetItem(
    list: *mut PyObject,
    index: isize,
    item: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(item)) };
        let index_error =
            || vm.new_index_error(format!("list assignment index out of range: {index}"));
        if index < 0 {
            return Err(index_error());
        }

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
            0.. => Err(index_error()),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Append(list: *mut PyObject, item: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { &*item }.to_owned();
        list.borrow_vec_mut().push(item);
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
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let item = unsafe { &*item }.to_owned();
        let mut vec = list.borrow_vec_mut();
        let index = if index < 0 {
            index + vec.len() as isize
        } else {
            index
        }
        .clamp(0, vec.len() as isize) as usize;
        vec.insert(index, item);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Reverse(list: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        list.borrow_vec_mut().reverse();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_AsTuple(list: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        Ok(vm.ctx.new_tuple(list.borrow_vec().to_vec()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_GetSlice(
    list: *mut PyObject,
    low: isize,
    high: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let vec = list.borrow_vec();
        let sliced = vec.getitem_by_slice(vm, SaturatedSlice::from_parts(low, high, 1))?;
        Ok(vm.ctx.new_list(sliced))
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
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        let slice = SaturatedSlice::from_parts(low, high, 1);
        let mut vec = list.borrow_vec_mut();

        if itemlist.is_null() {
            vec.delitem_by_slice(vm, slice)?;
            return Ok(());
        }

        let items: Vec<PyObjectRef> = unsafe { &*itemlist }.try_to_value(vm)?;
        vec.setitem_by_slice(vm, slice, &items)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyList_Sort(list: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let list = unsafe { &*list }.try_downcast_ref::<PyList>(vm)?;
        vm.call_method(list.as_object(), "sort", ())?;
        Ok(())
    })
}

#[cfg(false)]
mod tests {
    use pyo3::exceptions::PyIndexError;
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
}

use crate::{PyObject, pystate::with_vm};
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.try_mapping(vm)?.length(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Keys(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let keys = obj.try_mapping(vm)?.keys(vm)?;
        let iter = keys.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Values(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let values = obj.try_mapping(vm)?.values(vm)?;
        let iter = values.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMapping_Items(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let items = obj.try_mapping(vm)?.items(vm)?;
        let iter = items.get_iter(vm)?;
        Ok(vm.ctx.new_list(iter.try_to_value(vm)?))
    })
}

#[cfg(false)]
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

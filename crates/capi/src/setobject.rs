use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use itertools::process_results;
use rustpython_vm::AsObject;
use rustpython_vm::PyPayload;
use rustpython_vm::TryFromObject;
use rustpython_vm::builtins::{PyFrozenSet, PySet};
use rustpython_vm::function::ArgIterable;

define_py_check!(fn PySet_Check, types.set_type);
define_py_check!(fn PyFrozenSet_Check, types.frozenset_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_New(iterable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        if iterable.is_null() {
            return Ok(PySet::default().into_ref(&vm.ctx));
        }

        let iterable = ArgIterable::try_from_object(vm, unsafe { &*iterable }.to_owned())?;
        let set = PySet::default().into_ref(&vm.ctx);
        for item in iterable.iter(vm)? {
            set.add(item?, vm)?;
        }
        Ok(set)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyFrozenSet_New(iterable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        if iterable.is_null() {
            return Ok(vm.ctx.empty_frozenset.to_owned());
        }

        let iterable = ArgIterable::try_from_object(vm, unsafe { &*iterable }.to_owned())?;
        let set = process_results(iterable.iter(vm)?, |it| PyFrozenSet::from_iter(vm, it))??;
        Ok(set.into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Add(set: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let set = unsafe { &*set }.try_downcast_ref::<PySet>(vm)?;
        let key = unsafe { &*key }.to_owned();
        set.add(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Clear(set: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let set = unsafe { &*set }.try_downcast_ref::<PySet>(vm)?;
        set.clear();
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Contains(anyset: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let anyset = unsafe { &*anyset };
        let key = unsafe { &*key };

        if let Some(set) = anyset.downcast_ref::<PySet>() {
            set.__contains__(key, vm)
        } else if let Some(frozenset) = anyset.downcast_ref::<PyFrozenSet>() {
            frozenset.__contains__(key, vm)
        } else {
            Err(vm.new_type_error(format!(
                "expected set or frozenset, got '{}'",
                anyset.class().name()
            )))
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Discard(set: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let set = unsafe { &*set }.try_downcast_ref::<PySet>(vm)?;
        let key = unsafe { &*key };
        let had_item = set.__contains__(key, vm)?;
        if had_item {
            set.discard(key.to_owned(), vm)?;
        }
        Ok(had_item)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Pop(set: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let set = unsafe { &*set }.try_downcast_ref::<PySet>(vm)?;
        set.pop(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySet_Size(anyset: *mut PyObject) -> isize {
    with_vm(|vm| {
        let anyset = unsafe { &*anyset };
        if let Some(set) = anyset.downcast_ref::<PySet>() {
            set.as_object().length(vm)
        } else if let Some(frozenset) = anyset.downcast_ref::<PyFrozenSet>() {
            frozenset.as_object().length(vm)
        } else {
            Err(vm.new_type_error(format!(
                "expected set or frozenset, got '{}'",
                anyset.class().name()
            )))
        }
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyFrozenSet, PyInt, PySet};

    #[test]
    fn new_and_size() {
        Python::attach(|py| {
            let set = PySet::empty(py).unwrap();
            assert!(set.is_instance_of::<PySet>());
            assert_eq!(set.len(), 0);

            let frozen = PyFrozenSet::empty(py).unwrap();
            assert!(frozen.is_instance_of::<PyFrozenSet>());
            assert_eq!(frozen.len(), 0);
        })
    }

    #[test]
    fn add_contains_discard() {
        Python::attach(|py| {
            let set = PySet::empty(py).unwrap();
            let item = PyInt::new(py, 42);

            set.add(&item).unwrap();
            assert!(set.contains(&item).unwrap());
            set.discard(&item).unwrap();
            assert!(!set.contains(&item).unwrap());
        })
    }

    #[test]
    fn pop_reduces_size() {
        Python::attach(|py| {
            let set = PySet::empty(py).unwrap();
            set.add(7).unwrap();
            assert_eq!(set.len(), 1);

            let popped = set.pop().unwrap();
            assert_eq!(popped.extract::<i32>().unwrap(), 7);
            assert_eq!(set.len(), 0);
        })
    }
}

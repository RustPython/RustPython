use crate::{PyObject, pystate::with_vm};
use core::ffi::c_int;
use rustpython_vm::PyObjectRef;
use rustpython_vm::builtins::PyGenerator;
use rustpython_vm::protocol::{PyIter, PyIterReturn};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Check(obj: *mut PyObject) -> c_int {
    with_vm(|_vm| Ok(PyIter::check(unsafe { &*obj })))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetIter(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.get_iter(vm).map(PyObjectRef::from)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_NextItem(iter: *mut PyObject, item: *mut *mut PyObject) -> c_int {
    with_vm(|vm| {
        unsafe {
            *item = core::ptr::null_mut();
        }

        let iter = PyIter::new(unsafe { &*iter });
        match iter.next(vm)? {
            PyIterReturn::Return(next_item) => {
                unsafe {
                    *item = next_item.into_raw().as_ptr();
                };
                Ok(true)
            }
            PyIterReturn::StopIteration(_) => Ok(false),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Next(iter: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let iter = PyIter::new(unsafe { &*iter });
        match iter.next(vm)? {
            PyIterReturn::Return(next_item) => Ok(next_item.into_raw().as_ptr()),
            PyIterReturn::StopIteration(_) => Ok(core::ptr::null_mut()),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIter_Send(
    iter: *mut PyObject,
    arg: *mut PyObject,
    presult: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *presult = core::ptr::null_mut();
        }

        let iter_obj = unsafe { &*iter };
        let arg_obj = unsafe { &*arg };

        let ret = if vm.is_none(arg_obj) {
            PyIter::new(iter_obj).next(vm)?
        } else {
            iter_obj
                .try_downcast_ref::<PyGenerator>(vm)?
                .as_coro()
                .send(iter_obj, arg_obj.to_owned(), vm)?
        };

        match ret {
            PyIterReturn::Return(next_item) => {
                unsafe {
                    *presult = next_item.into_raw().as_ptr();
                };
                Ok(true)
            }
            PyIterReturn::StopIteration(ret_val) => {
                let ret_val = ret_val.unwrap_or_else(|| vm.ctx.none());
                unsafe {
                    *presult = ret_val.into_raw().as_ptr();
                };
                Ok(false)
            }
        }
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyAnyMethods, PyIterator, PyList, PySendResult};

    #[test]
    fn next_item() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            let iter = list.try_iter().unwrap();
            let items: Vec<i32> = iter.map(|x| x.unwrap().extract::<i32>().unwrap()).collect();
            assert_eq!(items, vec![1, 2, 3]);
        })
    }

    #[test]
    fn send_generator() {
        Python::attach(|py| {
            let generator = py
                .eval(c"(x for x in (1, 2))", None, None)
                .unwrap()
                .cast_into::<PyIterator>()
                .unwrap();

            let first = generator.send(py.None().bind(py)).unwrap();
            assert!(matches!(
                first,
                PySendResult::Next(value) if value.extract::<i32>().unwrap() == 1
            ));

            let second = generator.send(py.None().bind(py)).unwrap();
            assert!(matches!(
                second,
                PySendResult::Next(value) if value.extract::<i32>().unwrap() == 2
            ));
        })
    }
}

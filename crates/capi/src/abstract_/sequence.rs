use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::c_int;
use rustpython_vm::AsObject;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Check(obj: *mut PyObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { obj.assume_borrowed() };
        Ok(obj.sequence_unchecked().check())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Concat(
    obj1: *mut PyObject,
    obj2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj1 = unsafe { obj1.assume_borrowed() };
        let obj2 = unsafe { obj2.assume_borrowed() };
        obj1.try_sequence(vm)?.concat(obj2, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Count(obj: *mut PyObject, value: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() };
        obj.try_sequence(vm)?.count(value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelItem(obj: *mut PyObject, index: isize) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.del_item(index, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_DelSlice(obj: *mut PyObject, low: isize, high: isize) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.del_slice(low, high, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetItem(obj: *mut PyObject, index: isize) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.get_item(index, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_GetSlice(
    obj: *mut PyObject,
    low: isize,
    high: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.get_slice(low, high, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceConcat(
    obj1: *mut PyObject,
    obj2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj1 = unsafe { obj1.assume_borrowed() };
        let obj2 = unsafe { obj2.assume_borrowed() };
        obj1.try_sequence(vm)?.inplace_concat(obj2, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_InPlaceRepeat(
    obj: *mut PyObject,
    count: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.inplace_repeat(count, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Index(obj: *mut PyObject, value: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() };
        obj.try_sequence(vm)?.index(value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_List(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        Ok(obj.try_sequence(vm)?.list(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Repeat(obj: *mut PyObject, count: isize) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.repeat(count, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetItem(
    obj: *mut PyObject,
    index: isize,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() };
        obj.try_sequence(vm)?.set_item(index, value.to_owned(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_SetSlice(
    obj: *mut PyObject,
    low: isize,
    high: isize,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() };
        obj.try_sequence(vm)?
            .set_slice(low, high, value.to_owned(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.try_sequence(vm)?.length(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Length(obj: *mut PyObject) -> isize {
    unsafe { PySequence_Size(obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Tuple(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        Ok(obj.try_sequence(vm)?.tuple(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Contains(obj: *mut PyObject, value: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() };
        obj.sequence_unchecked().contains(value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_In(obj: *mut PyObject, value: *mut PyObject) -> c_int {
    unsafe { PySequence_Contains(obj, value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PySequence_Fast(
    obj: *mut PyObject,
    m: *const core::ffi::c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        if obj.class().is(vm.ctx.types.list_type) || obj.class().is(vm.ctx.types.tuple_type) {
            return Ok(obj.to_owned());
        }
        let iter = match obj.to_owned().get_iter(vm) {
            Ok(iter) => iter,
            Err(err) => {
                if !m.is_null() && err.fast_isinstance(vm.ctx.exceptions.type_error) {
                    let msg = unsafe { core::ffi::CStr::from_ptr(m) }
                        .to_str()
                        .unwrap_or("not a sequence");
                    return Err(vm.new_type_error(msg.to_owned()));
                }
                return Err(err);
            }
        };
        let elements = iter
            .into_iter_sized::<rustpython_vm::PyObjectRef>(vm)?
            .collect::<rustpython_vm::PyResult<Vec<_>>>()?;
        Ok(vm.ctx.new_list(elements).into())
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyAnyMethods, PyDict, PyList, PySequence, PySequenceMethods, PyTuple};

    #[test]
    fn item_and_size_ops() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            let seq = list.cast_into::<PySequence>().unwrap();

            assert_eq!(seq.len().unwrap(), 3);
            assert_eq!(seq.get_item(1).unwrap().extract::<i32>().unwrap(), 2);

            seq.set_item(1, 4).unwrap();
            assert_eq!(seq.get_item(1).unwrap().extract::<i32>().unwrap(), 4);

            seq.del_item(1).unwrap();
            assert_eq!(seq.get_item(1).unwrap().extract::<i32>().unwrap(), 3);
        });
    }

    #[test]
    fn slice_ops() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3, 4]).unwrap();
            let seq = list.cast_into::<PySequence>().unwrap();

            let sub = seq.get_slice(1, 3).unwrap();
            assert_eq!(sub.get_item(0).unwrap().extract::<i32>().unwrap(), 2);
            assert_eq!(sub.get_item(1).unwrap().extract::<i32>().unwrap(), 3);

            let repl = PyList::new(py, [8, 9]).unwrap();
            seq.set_slice(1, 3, &repl).unwrap();
            assert_eq!(seq.get_item(1).unwrap().extract::<i32>().unwrap(), 8);
            assert_eq!(seq.get_item(2).unwrap().extract::<i32>().unwrap(), 9);

            seq.del_slice(1, 3).unwrap();
            assert_eq!(seq.get_item(0).unwrap().extract::<i32>().unwrap(), 1);
            assert_eq!(seq.get_item(1).unwrap().extract::<i32>().unwrap(), 4);
        });
    }

    #[test]
    fn concat_repeat_and_inplace_ops() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2]).unwrap();
            let seq = list.cast_into::<PySequence>().unwrap();
            let rhs = PyList::new(py, [3])
                .unwrap()
                .cast_into::<PySequence>()
                .unwrap();

            let concat = seq.concat(&rhs).unwrap();
            assert_eq!(concat.get_item(2).unwrap().extract::<i32>().unwrap(), 3);

            let repeat = seq.repeat(2).unwrap();
            assert_eq!(repeat.get_item(2).unwrap().extract::<i32>().unwrap(), 1);

            let iadd_rhs = PyList::new(py, [4])
                .unwrap()
                .cast_into::<PySequence>()
                .unwrap();
            seq.in_place_concat(&iadd_rhs).unwrap();
            assert_eq!(seq.get_item(2).unwrap().extract::<i32>().unwrap(), 4);

            seq.in_place_repeat(2).unwrap();
            assert_eq!(seq.get_item(5).unwrap().extract::<i32>().unwrap(), 4);
        });
    }

    #[test]
    fn count_index_contains_and_convert_ops() {
        Python::attach(|py| {
            let tuple = PyTuple::new(py, [1, 2, 1])
                .unwrap()
                .cast_into::<PySequence>()
                .unwrap();

            assert_eq!(tuple.count(1).unwrap(), 2);
            assert_eq!(tuple.index(2).unwrap(), 1);
            assert!(tuple.contains(1).unwrap());

            let as_list = tuple.to_list().unwrap().cast_into::<PySequence>().unwrap();
            assert_eq!(as_list.get_item(0).unwrap().extract::<i32>().unwrap(), 1);

            let as_tuple = as_list
                .to_tuple()
                .unwrap()
                .cast_into::<PySequence>()
                .unwrap();
            assert_eq!(as_tuple.get_item(2).unwrap().extract::<i32>().unwrap(), 1);
        });
    }

    #[test]
    fn contains_works_for_dict() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("k", 1).unwrap();

            let any = dict.into_any();
            assert!(any.contains("k").unwrap());
            assert!(!any.contains("missing").unwrap());
        });
    }

    #[test]
    fn sequence_fast() {
        Python::attach(|py| unsafe {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            let fast_list = super::PySequence_Fast(list.as_ptr().cast(), core::ptr::null());
            assert_eq!(fast_list, list.as_ptr().cast());
            crate::refcount::_Py_DecRef(fast_list);

            let tuple = PyTuple::new(py, [1, 2, 3]).unwrap();
            let fast_tuple = super::PySequence_Fast(tuple.as_ptr().cast(), core::ptr::null());
            assert_eq!(fast_tuple, tuple.as_ptr().cast());
            crate::refcount::_Py_DecRef(fast_tuple);

            let set = py.eval(c"{10, 20}", None, None).unwrap();
            let fast_set = super::PySequence_Fast(set.as_ptr().cast(), core::ptr::null());
            assert!(!fast_set.is_null());
            assert_ne!(fast_set, set.as_ptr().cast());
            crate::refcount::_Py_DecRef(fast_set);

            let num = py.eval(c"123", None, None).unwrap();
            let fast_num = super::PySequence_Fast(num.as_ptr().cast(), c"custom error".as_ptr());
            assert!(fast_num.is_null());
            let err = pyo3::PyErr::take(py).unwrap();
            assert_eq!(err.to_string(), "TypeError: custom error");

            let globals = PyDict::new(py);
            py.run(
                c"def failing_gen():\n    yield 1\n    raise ValueError('boom')",
                Some(&globals),
                None,
            )
            .unwrap();
            let failing_iter = py.eval(c"failing_gen()", Some(&globals), None).unwrap();
            let fast_gen =
                super::PySequence_Fast(failing_iter.as_ptr().cast(), c"custom error".as_ptr());
            assert!(fast_gen.is_null());
            let err_gen = pyo3::PyErr::take(py).unwrap();
            assert_eq!(err_gen.to_string(), "ValueError: boom");
        });
    }
}

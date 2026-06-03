use crate::{PyObject, pystate::with_vm};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Add(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._add(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Index(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*obj }.try_index(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Lshift(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._lshift(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Or(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._or(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Rshift(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._rshift(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Subtract(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._sub(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;

    #[test]
    fn add() {
        Python::attach(|py| {
            let lhs = 40i64.into_pyobject(py).unwrap();
            let rhs = 2i64.into_pyobject(py).unwrap();
            let out = lhs.add(rhs).unwrap();
            assert_eq!(out.extract::<i64>().unwrap(), 42);
        })
    }

    #[test]
    fn lshift() {
        Python::attach(|py| {
            let lhs = 1i64.into_pyobject(py).unwrap();
            let rhs = 5i64.into_pyobject(py).unwrap();
            let out = lhs.lshift(rhs).unwrap();
            assert_eq!(out.extract::<i64>().unwrap(), 32);
        })
    }

    #[test]
    fn bit_or() {
        Python::attach(|py| {
            let lhs = 0b1010i64.into_pyobject(py).unwrap();
            let rhs = 0b0110i64.into_pyobject(py).unwrap();
            let out = lhs.bitor(rhs).unwrap();
            assert_eq!(out.extract::<i64>().unwrap(), 0b1110);
        })
    }

    #[test]
    fn rshift() {
        Python::attach(|py| {
            let lhs = 128i64.into_pyobject(py).unwrap();
            let rhs = 3i64.into_pyobject(py).unwrap();
            let out = lhs.rshift(rhs).unwrap();
            assert_eq!(out.extract::<i64>().unwrap(), 16);
        })
    }

    #[test]
    fn subtract() {
        Python::attach(|py| {
            let lhs = 50i64.into_pyobject(py).unwrap();
            let rhs = 8i64.into_pyobject(py).unwrap();
            let out = lhs.sub(rhs).unwrap();
            assert_eq!(out.extract::<i64>().unwrap(), 42);
        })
    }
}

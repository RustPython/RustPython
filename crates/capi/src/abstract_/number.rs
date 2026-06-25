use crate::{PyObject, pystate::with_vm};
use core::ffi::c_int;
use rustpython_vm::protocol::PyNumber;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Add(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._add(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyIndex_Check(obj: *mut PyObject) -> c_int {
    with_vm(|_vm| unsafe { obj.as_ref() }.is_some_and(|obj| obj.number().is_index()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Absolute(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._abs(unsafe { &*o }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_And(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._and(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Check(o: *mut PyObject) -> c_int {
    with_vm(|_vm| unsafe { o.as_ref() }.is_some_and(PyNumber::check))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Divmod(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._divmod(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Float(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*o }.try_float(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_FloorDivide(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._floordiv(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceAdd(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._iadd(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceAnd(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._iand(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceFloorDivide(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._ifloordiv(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceLshift(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._ilshift(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceMatrixMultiply(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._imatmul(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceMultiply(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._imul(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceOr(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._ior(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlacePower(
    o1: *mut PyObject,
    o2: *mut PyObject,
    o3: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._ipow(unsafe { &*o1 }, unsafe { &*o2 }, unsafe { &*o3 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceRemainder(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._imod(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceRshift(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._irshift(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceSubtract(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._isub(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceTrueDivide(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._itruediv(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_InPlaceXor(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._ixor(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Invert(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._invert(unsafe { &*o }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Index(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*obj }.try_index(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_MatrixMultiply(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._matmul(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Multiply(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._mul(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Negative(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._neg(unsafe { &*o }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Positive(o: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._pos(unsafe { &*o }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Power(
    o1: *mut PyObject,
    o2: *mut PyObject,
    o3: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._pow(unsafe { &*o1 }, unsafe { &*o2 }, unsafe { &*o3 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Remainder(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._mod(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_TrueDivide(
    o1: *mut PyObject,
    o2: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| vm._truediv(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Xor(o1: *mut PyObject, o2: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| vm._xor(unsafe { &*o1 }, unsafe { &*o2 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyNumber_Long(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*obj }.try_int(vm))
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

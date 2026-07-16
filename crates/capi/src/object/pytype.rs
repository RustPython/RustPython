use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::FfiPtrExt;
use core::ffi::{c_int, c_ulong};
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::{AsObject, Py, PyObject};

pub type PyTypeObject = Py<PyType>;

define_py_check!(fn PyType_Check, types.type_type);
define_py_check!(exact fn PyType_CheckExact, types.type_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    unsafe { op.assume_borrowed() }.class()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { op.assume_borrowed() };
        let ty = unsafe { ty.assume_borrowed() };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ptr: *mut PyTypeObject) -> c_ulong {
    let ty = unsafe { ptr.assume_borrowed() };
    ty.slots.flags.bits() as u32 as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_IsSubtype(a: *mut PyTypeObject, b: *mut PyTypeObject) -> c_int {
    with_vm(move |_vm| {
        let a = unsafe { a.assume_borrowed() };
        let b = unsafe { b.assume_borrowed() };
        Ok(a.is_subtype(b))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__name__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetQualName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__qualname__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__module__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFullyQualifiedName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| {
        let ty = unsafe { ptr.assume_borrowed() };
        let qualname = ty.__qualname__(vm).try_downcast::<PyStr>(vm)?;
        let module = ty.__module__(vm);

        if let Some(module) = module.downcast_ref::<PyStr>()
            && module.as_wtf8() != "builtins"
        {
            Ok(vm.ctx.new_str(format!("{module}.{qualname}")))
        } else {
            Ok(qualname)
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyInt, PyString, PyTypeMethods};

    #[test]
    fn type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_str().unwrap(), "str");
        })
    }

    #[test]
    fn type_get_module_name() {
        Python::attach(|py| {
            assert_eq!(
                py.get_type::<PyInt>().module().unwrap().to_str().unwrap(),
                "builtins"
            );
        })
    }
}

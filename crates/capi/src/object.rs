use crate::{PyObject, with_vm};
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::mem::MaybeUninit;
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::{AsObject, Context, Py};

const PY_TPFLAGS_LONG_SUBCLASS: c_ulong = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: c_ulong = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: c_ulong = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: c_ulong = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: c_ulong = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: c_ulong = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: c_ulong = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: c_ulong = 1 << 31;

pub type PyTypeObject = Py<PyType>;

#[unsafe(no_mangle)]
pub static mut PyType_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyBaseObject_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyLong_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyTuple_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyUnicode_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyBool_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyDict_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub static mut PyComplex_Type: MaybeUninit<&'static PyTypeObject> = MaybeUninit::uninit();

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    // SAFETY: The caller must guarantee that `op` is a valid pointer to a `PyObject`.
    unsafe { (*op).class() }
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { &*op };
        let ty = unsafe { &*ty };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn PyType_GetFlags(ptr: *const PyTypeObject) -> c_ulong {
    let ctx = Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;

    // SAFETY: The caller must guarantee that `ptr` is a valid pointer to a `PyType` object.
    let ty = unsafe { &*ptr };
    let mut flags = ty.slots.flags.bits();

    if ty.is_subtype(zoo.int_type) {
        flags |= PY_TPFLAGS_LONG_SUBCLASS;
    }
    if ty.is_subtype(zoo.list_type) {
        flags |= PY_TPFLAGS_LIST_SUBCLASS
    }
    if ty.is_subtype(zoo.tuple_type) {
        flags |= PY_TPFLAGS_TUPLE_SUBCLASS;
    }
    if ty.is_subtype(zoo.bytes_type) {
        flags |= PY_TPFLAGS_BYTES_SUBCLASS;
    }
    if ty.is_subtype(zoo.str_type) {
        flags |= PY_TPFLAGS_UNICODE_SUBCLASS;
    }
    if ty.is_subtype(zoo.dict_type) {
        flags |= PY_TPFLAGS_DICT_SUBCLASS;
    }
    if ty.is_subtype(exp_zoo.base_exception_type) {
        flags |= PY_TPFLAGS_BASE_EXC_SUBCLASS;
    }
    if ty.is_subtype(zoo.type_type) {
        flags |= PY_TPFLAGS_TYPE_SUBCLASS;
    }

    flags
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*ptr };
    with_vm(move |vm| ty.__name__(vm))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetQualName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*ptr };
    with_vm(move |vm| ty.__qualname__(vm))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetFullyQualifiedName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*ptr };
    with_vm(move |vm| {
        let module = ty.__module__(vm).downcast::<PyStr>().unwrap();
        let qualname = ty.__qualname__(vm).downcast::<PyStr>().unwrap();
        let fully_qualified_name = format!(
            "{}.{}",
            module.to_string_lossy(),
            qualname.to_string_lossy()
        );
        vm.ctx.new_str(fully_qualified_name)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_IsSubtype(a: *const PyTypeObject, b: *const PyTypeObject) -> c_int {
    with_vm(move |_vm| {
        let a = unsafe { &*a };
        let b = unsafe { &*b };
        Ok(a.is_subtype(b))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetAttr(obj: *mut PyObject, name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_SetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { CStr::from_ptr(attr_name) }
            .to_str()
            .expect("attribute name must be valid UTF-8");
        let value = unsafe { &*value }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_SetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        let value = unsafe { &*value }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Repr(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(obj) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.repr(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Str(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(obj) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.str(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut PyObject {
    with_vm(|vm| {
        let ctx = &vm.ctx;
        match constant_id {
            0 => ctx.none.as_object(),
            1 => ctx.false_value.as_object(),
            2 => ctx.true_value.as_object(),
            3 => ctx.ellipsis.as_object(),
            4 => ctx.not_implemented.as_object(),
            _ => panic!("Invalid constant_id passed to Py_GetConstantBorrowed"),
        }
        .as_raw()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.to_owned().is_true(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GenericGetDict(
    obj: *mut PyObject,
    _context: *mut c_void,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.get_attr("__dict__", vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GenericSetDict(
    obj: *mut PyObject,
    value: *mut PyObject,
    _context: *mut c_void,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let value = unsafe { &*value }.to_owned();
        obj.set_attr("__dict__", value, vm)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyDict, PyInt, PyNone, PyString};

    #[test]
    fn test_is_truthy() {
        Python::attach(|py| {
            assert!(!py.None().is_truthy(py).unwrap());
        })
    }

    #[test]
    fn test_is_none() {
        Python::attach(|py| {
            assert!(py.None().is_none(py));
        })
    }

    #[test]
    fn test_bool() {
        Python::attach(|py| {
            assert!(PyBool::new(py, true).is_truthy().unwrap());
            assert!(!PyBool::new(py, false).is_truthy().unwrap());
        })
    }

    #[test]
    fn test_type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_str().unwrap(), "str");
        })
    }

    #[test]
    #[ignore = "Instance checking on static type pointers is yet supported"]
    fn test_static_type_pointers() {
        Python::attach(|py| {
            assert!(py.None().bind(py).is_instance_of::<PyNone>());
            assert!(PyBool::new(py, true).is_instance_of::<PyBool>());
        })
    }

    #[test]
    fn test_repr() {
        Python::attach(|py| {
            let module = py.import("sys").unwrap();
            assert_eq!(module.repr().unwrap(), "<module 'sys' (built-in)>");
        })
    }

    #[test]
    fn test_obj_to_str() {
        Python::attach(|py| {
            let number = PyInt::new(py, 42);
            assert_eq!(number.str().unwrap(), "42");
        })
    }

    #[test]
    fn test_get_attr() {
        Python::attach(|py| {
            let sys = py.import("sys").unwrap();
            let implementation = sys
                .getattr("implementation")
                .unwrap()
                .getattr("name")
                .unwrap()
                .str()
                .unwrap();

            assert_eq!(implementation, "rustpython");
        })
    }

    #[test]
    fn test_generic_get_dict() {
        Python::attach(|py| {
            let globals = PyDict::new(py);
            py.run(c"class MyClass: ...", None, Some(&globals)).unwrap();
            let my_class = globals.get_item("MyClass").unwrap().unwrap();
            let instance = my_class.call0().unwrap();
            instance.setattr("foo", 42).unwrap();
            let dict = unsafe {
                Bound::from_owned_ptr_or_err(
                    py,
                    pyo3::ffi::PyObject_GenericGetDict(instance.as_ptr(), std::ptr::null_mut()),
                )
            }
            .unwrap();
            assert!(dict.get_item("foo").is_ok());
        })
    }
}

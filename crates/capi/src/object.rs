use crate::PyObject;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyStr, PyType, object_generic_set_dict, object_get_dict};
use rustpython_vm::bytecode::ComparisonOperator;
use rustpython_vm::function::PySetterValue;
use rustpython_vm::{AsObject, Py, PyPayload};

pub type PyTypeObject = Py<PyType>;

macro_rules! define_py_check {
    (fn $name:ident, $($ctx_path:ident).+) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(obj: *mut crate::PyObject) -> core::ffi::c_int {
            crate::pystate::with_vm(|vm| unsafe {
                obj
                .as_ref()
                .map(|obj| obj.class().is_subtype(vm.ctx.$($ctx_path).+))
                .unwrap_or_default()
            })
        }
    };
    (exact fn $name:ident, $($ctx_path:ident).+) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(obj: *mut crate::PyObject) -> core::ffi::c_int {
            use rustpython_vm::AsObject;
            crate::pystate::with_vm(|vm| unsafe {
                obj
                .as_ref()
                .map(|obj| obj.class().is(vm.ctx.$($ctx_path).+))
                .unwrap_or_default()
            })
        }
    };
}

pub(crate) use define_py_check;
define_py_check!(fn PyType_Check, types.type_type);
define_py_check!(exact fn PyType_CheckExact, types.type_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    unsafe { (*op).class() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { &*op };
        let ty = unsafe { &*ty };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ptr: *const PyTypeObject) -> c_ulong {
    let ty = unsafe { &*ptr };
    ty.slots.flags.bits() as u32 as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_IsSubtype(a: *const PyTypeObject, b: *const PyTypeObject) -> c_int {
    with_vm(move |_vm| {
        let a = unsafe { &*a };
        let b = unsafe { &*b };
        Ok(a.is_subtype(b))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetName(ptr: *const PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*ptr }.__name__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetQualName(ptr: *const PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*ptr }.__qualname__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleName(ptr: *const PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*ptr }.__module__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFullyQualifiedName(ptr: *const PyTypeObject) -> *mut PyObject {
    with_vm(|vm| {
        let ty = unsafe { &*ptr };
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

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut PyObject {
    with_vm(|vm| {
        let ctx = &vm.ctx;
        let constant = match constant_id {
            0 => ctx.none.as_object(),
            1 => ctx.false_value.as_object(),
            2 => ctx.true_value.as_object(),
            3 => ctx.ellipsis.as_object(),
            4 => ctx.not_implemented.as_object(),
            _ => {
                return Err(
                    vm.new_system_error("Invalid constant ID passed to Py_GetConstantBorrowed")
                );
            }
        }
        .as_raw();
        Ok(constant)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe {
            CStr::from_ptr(attr_name)
                .to_str()
                .expect("attribute name must be valid UTF-8")
        };
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetOptionalAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *result = core::ptr::null_mut();
        }
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        if let Some(attr) = vm.get_attribute_opt(obj.to_owned(), name)? {
            unsafe {
                *result = attr.into_raw().as_ptr();
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetAttrString(
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
pub unsafe extern "C" fn PyObject_SetAttr(
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
pub unsafe extern "C" fn PyObject_HasAttrWithError(
    obj: *mut PyObject,
    attr_name: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*attr_name }.try_downcast_ref::<PyStr>(vm)?;
        obj.has_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        obj.generic_getattr(name, vm)
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
pub unsafe extern "C" fn PyObject_RichCompare(
    left: *mut PyObject,
    right: *mut PyObject,
    op: c_int,
) -> *mut PyObject {
    with_vm(|vm| {
        let op = match op {
            0 => ComparisonOperator::Less,
            1 => ComparisonOperator::LessOrEqual,
            2 => ComparisonOperator::Equal,
            3 => ComparisonOperator::NotEqual,
            4 => ComparisonOperator::Greater,
            5 => ComparisonOperator::GreaterOrEqual,
            _ => return Err(vm.new_system_error("invalid comparison operator")),
        };
        let left = unsafe { &*left };
        let right = unsafe { &*right };
        left.to_owned()
            .rich_compare(right.to_owned(), op.into(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCallable_Check(obj: *mut PyObject) -> c_int {
    with_vm(|_vm| unsafe { obj.as_ref().is_some_and(PyObject::is_callable) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_ClearWeakRefs(obj: *mut PyObject) {
    with_vm(|_vm| unsafe { &*obj }.clear_weak_refs())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Dir(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        unsafe { obj.as_ref() }
            .map_or_else(|| vm.dir(None), |obj| obj.to_owned().dir(vm))
            .map(|list| list.into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.to_owned().is_true(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericGetDict(
    obj: *mut PyObject,
    _context: *mut c_void,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        object_get_dict(obj.to_owned(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GenericSetDict(
    obj: *mut PyObject,
    value: *mut PyObject,
    _context: *mut c_void,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let value = match NonNull::new(value) {
            Some(value) => PySetterValue::Assign(unsafe { value.as_ref() }.to_owned()),
            None => PySetterValue::Delete,
        };
        object_generic_set_dict(obj.to_owned(), value, vm)
    })
}

#[cfg(false)]
mod tests {
    use pyo3::class::basic::CompareOp;
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyDict, PyInt, PyString, PyTypeMethods};

    #[test]
    fn is_truthy() {
        Python::attach(|py| {
            assert!(!py.None().is_truthy(py).unwrap());
        })
    }

    #[test]
    fn is_none() {
        Python::attach(|py| {
            assert!(py.None().is_none(py));
        })
    }

    #[test]
    fn bool() {
        Python::attach(|py| {
            assert!(PyBool::new(py, true).is_truthy().unwrap());
            assert!(!PyBool::new(py, false).is_truthy().unwrap());
        })
    }

    #[test]
    fn type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_str().unwrap(), "str");
        })
    }

    #[test]
    fn repr() {
        Python::attach(|py| {
            let module = py.import("sys").unwrap();
            assert_eq!(module.repr().unwrap(), "<module 'sys' (built-in)>");
        })
    }

    #[test]
    fn obj_to_str() {
        Python::attach(|py| {
            let number = PyInt::new(py, 42);
            assert_eq!(number.str().unwrap(), "42");
        })
    }

    #[test]
    fn get_attr() {
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
    fn callable_check() {
        Python::attach(|py| {
            let int_type = py.get_type::<PyInt>();
            assert!(int_type.is_callable());
            assert!(!PyInt::new(py, 42).is_callable());
        })
    }

    #[test]
    fn object_dir() {
        Python::attach(|py| {
            assert!(PyInt::new(py, 42).dir().unwrap().len() > 0);
        })
    }

    #[test]
    fn get_optional_attr() {
        Python::attach(|py| {
            let number = PyInt::new(py, 42);
            assert!(number.getattr_opt("real").unwrap().is_some());
            assert!(
                number
                    .getattr_opt("attribute_that_should_not_exist")
                    .unwrap()
                    .is_none()
            );
        })
    }

    #[test]
    fn rich_compare() {
        Python::attach(|py| {
            let lower = PyInt::new(py, 1);
            let upper = PyInt::new(py, 2);
            assert!(
                lower
                    .rich_compare(upper, CompareOp::Lt)
                    .unwrap()
                    .is_truthy()
                    .unwrap()
            );
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

    #[test]
    fn generic_get_dict() {
        Python::attach(|py| {
            let globals = PyDict::new(py);
            py.run(c"class MyClass: ...", None, Some(&globals)).unwrap();
            let my_class = globals.get_item("MyClass").unwrap().unwrap();
            let instance = my_class.call0().unwrap();
            instance.setattr("foo", 42).unwrap();
            let dict = unsafe {
                Bound::from_owned_ptr_or_err(
                    py,
                    pyo3::ffi::PyObject_GenericGetDict(instance.as_ptr(), core::ptr::null_mut()),
                )
            }
            .unwrap();
            assert!(dict.get_item("foo").is_ok());
        })
    }
}

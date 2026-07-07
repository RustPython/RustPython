use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::{c_int, c_ulong};
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::{AsObject, Py, PyObject};
use std::ffi::c_void;

pub type PyTypeObject = Py<PyType>;

pub struct PyTypeSlot {
    pub slot: c_int,
    pub pfunc: *mut c_void,
}

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
#[cfg(false)]
pub extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    with_vm(|_vm| {
        let ty = unsafe { &*ty };
        let slot: u8 = slot
            .try_into()
            .expect("slot number out of range for SlotAccessor");
        let slot_accessor: SlotAccessor = slot
            .try_into()
            .expect("invalid slot number for SlotAccessor");

        match slot_accessor {
            SlotAccessor::TpNew => {
                extern "C" fn newfunc_wrapper(
                    subtype: *mut PyTypeObject,
                    args: *mut PyObject,
                    kwargs: *mut PyObject,
                ) -> *mut PyObject {
                    with_vm(|vm| {
                        let subtype = unsafe { &*subtype };
                        let mut func_args = FuncArgs::default();

                        if let Some(args_obj) = unsafe { args.as_ref() } {
                            let tuple = args_obj.try_downcast_ref::<PyTuple>(vm)?;
                            func_args
                                .args
                                .extend(tuple.iter().map(|arg| arg.to_owned()));
                        }

                        if let Some(kwargs_obj) = unsafe { kwargs.as_ref() } {
                            let kwargs = kwargs_obj.try_downcast_ref::<PyDict>(vm)?;
                            for (key, value) in kwargs.items_vec() {
                                let key = key.try_downcast::<PyStr>(vm)?;
                                func_args
                                    .kwargs
                                    .insert(key.to_string_lossy().into_owned(), value);
                            }
                        }

                        subtype
                            .slots
                            .new
                            .load()
                            .expect("tp_new slot function pointer is null")(
                            subtype.to_owned(),
                            func_args,
                            vm,
                        )
                    })
                }

                if let Some(vtable) = ty.get_type_data::<TypeVTable>() {
                    vtable.new_func.map(|newfunc| newfunc as *mut c_void)
                } else {
                    ty.slots.new.load().map(|_| newfunc_wrapper as *mut c_void)
                }
            }
            _ => {
                todo!("Slot {slot_accessor:?} for {ty:?} is not yet implemented in PyType_GetSlot")
            }
        }
        .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    // TODO: Implement immutable type freezing semantics.
    0
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyInt, PyString, PyTypeMethods};

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

    #[test]
    #[cfg(false)]
    fn test_rust_class() {
        #[pyclass]
        struct MyClass {
            #[pyo3(get)]
            num: i32,
        }

        #[pymethods]
        impl MyClass {
            #[new]
            fn new(value: i32) -> Self {
                MyClass { num: value }
            }

            fn method1(&self) -> PyResult<i32> {
                Ok(self.num + 10)
            }

            fn method2(&self, a: i32) -> PyResult<i32> {
                Ok(self.num + a)
            }

            #[staticmethod]
            fn static_method1(a: i32, b: i32) -> PyResult<i32> {
                Ok(a + b)
            }

            #[staticmethod]
            fn static_method2() -> PyResult<i32> {
                Ok(0)
            }

            #[classmethod]
            fn cls_method(cls: &Bound<'_, PyType>) -> PyResult<i32> {
                assert!(cls.is_subclass_of::<MyClass>()?);
                Ok(10)
            }
        }

        Python::attach(|py| {
            let obj = Bound::new(py, MyClass { num: 3 }).unwrap();

            let globals = PyDict::new(py);
            globals.set_item("instance", &obj).unwrap();
            py.run(c"assert instance.num == 3", Some(&globals), None)
                .unwrap();

            assert_eq!(
                obj.call_method1("method1", ())
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                13
            );

            assert_eq!(
                obj.call_method1("method2", (5,))
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                8
            );

            assert_eq!(
                obj.call_method1("static_method1", (5, 8))
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                13
            );

            assert_eq!(
                obj.call_method1("static_method2", ())
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                0
            );

            assert_eq!(
                obj.call_method1("cls_method", ())
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                10
            );
        });
    }
}

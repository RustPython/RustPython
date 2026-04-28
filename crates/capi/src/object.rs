use crate::methodobject::{PyMethodDef, build_method_def};
use crate::{PyObject, with_vm};
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyDict, PyStr, PyTuple, PyType};
use rustpython_vm::convert::IntoObject;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::types::{PyTypeFlags, PyTypeSlots, SlotAccessor};
use rustpython_vm::{AsObject, Context, Py, PyObjectRef, PyResult, VirtualMachine};

const PY_TPFLAGS_LONG_SUBCLASS: c_ulong = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: c_ulong = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: c_ulong = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: c_ulong = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: c_ulong = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: c_ulong = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: c_ulong = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: c_ulong = 1 << 31;

pub type PyTypeObject = Py<PyType>;

macro_rules! define_py_check {
    ($name:ident, $type_name:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(obj: *mut PyObject) -> c_int {
            with_vm(|vm| unsafe { (*obj).is_instance(vm.ctx.types.$type_name.as_object(), vm) })
        }
    };
    (exact $name:ident, $type_name:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(obj: *mut PyObject) -> c_int {
            with_vm(|vm| unsafe { (*obj).class().is(vm.ctx.types.$type_name) })
        }
    };
}

define_py_check!(PyFloat_Check, float_type);
define_py_check!(PyModule_Check, module_type);
define_py_check!(PyBool_Check, bool_type);

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
pub extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    with_vm(|_vm| -> Option<*mut c_void> {
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
    })
}

#[repr(C)]
pub struct PyType_Slot {
    slot: c_int,
    pfunc: *mut c_void,
}

#[repr(C)]
pub struct PyType_Spec {
    name: *const c_char,
    basicsize: c_int,
    itemsize: c_int,
    flags: c_uint,
    slots: *mut PyType_Slot,
}

#[repr(C)]
pub struct PyGetSetDef {
    name: *const c_char,
    get: extern "C" fn(*mut PyObject, usize) -> *mut PyObject,
    set: Option<extern "C" fn(*mut PyObject, *mut PyObject, usize) -> c_int>,
    doc: *const c_char,
    closure: usize,
}

#[derive(Default)]
struct TypeVTable {
    new_func: Option<newfunc>,
}

type newfunc = unsafe extern "C" fn(
    ty: *mut PyTypeObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject;

#[unsafe(no_mangle)]
pub extern "C" fn PyType_FromSpec(spec: *mut PyType_Spec) -> *mut PyObject {
    with_vm(|vm| -> PyResult {
        let spec = unsafe { &*spec };
        let class_name = unsafe {
            CStr::from_ptr(spec.name)
                .to_str()
                .expect("type name must be valid UTF-8")
        };
        let mut base = vm.ctx.types.object_type;
        let mut slots = PyTypeSlots::heap_default();

        slots.basicsize = spec.basicsize as _;
        slots.itemsize = spec.itemsize as _;
        slots.flags = PyTypeFlags::from_bits(spec.flags as u64).expect("invalid flags value");

        let mut attributes: &[PyGetSetDef] = &[];
        let mut methods: &[PyMethodDef] = &[];
        let mut vtable = TypeVTable::default();
        let mut slot_ptr = spec.slots;
        while let slot = unsafe { &*slot_ptr }
            && slot.slot != 0
        {
            let accessor = SlotAccessor::try_from(slot.slot as u8)
                .expect("invalid slot number in PyType_Spec");

            match accessor {
                SlotAccessor::TpDealloc => {
                    slots.del.store(Some(|ty, _vm| {
                        todo!("tp_dealloc is not yet implemented in PyType_FromSpec for {ty:?}")
                    }));
                }
                SlotAccessor::TpBase => base = unsafe { &*slot.pfunc.cast::<PyTypeObject>() },
                SlotAccessor::TpGetset => {
                    let start = slot.pfunc.cast::<PyGetSetDef>();
                    let mut end = start;
                    while unsafe { !(*end).name.is_null() } {
                        end = unsafe { end.add(1) }
                    }
                    attributes = unsafe {
                        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
                    };
                }
                SlotAccessor::TpMethods => {
                    let start = slot.pfunc.cast::<PyMethodDef>();
                    let mut end = start;
                    while unsafe { !(*end).ml_name.is_null() } {
                        end = unsafe { end.add(1) }
                    }
                    methods = unsafe {
                        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
                    };
                }
                SlotAccessor::TpNew => {
                    vtable.new_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.new.store(Some(|ty, args, vm| {
                        let new_func = ty.get_type_data::<TypeVTable>().unwrap().new_func.unwrap();
                        let kwargs = vm.ctx.new_dict();
                        for (name, value) in &args.kwargs {
                            kwargs.set_item(&*vm.ctx.new_str(name.clone()), value.clone(), vm)?;
                        }
                        let args = vm.ctx.new_tuple(args.args);
                        let result = unsafe {
                            new_func(
                                (&*ty) as *const _ as *mut _,
                                args.as_object().as_raw().cast_mut(),
                                kwargs.as_object().as_raw().cast_mut(),
                            )
                        };

                        unsafe { Ok(PyObjectRef::from_raw(NonNull::new(result).unwrap())) }
                    }));
                }
                SlotAccessor::TpDoc => {
                    let doc = unsafe {
                        CStr::from_ptr(slot.pfunc.cast::<c_char>())
                            .to_str()
                            .expect("tp_doc must be a valid UTF-8 string")
                    };
                    slots.doc = Some(doc);
                }
                _ => todo!("Slot {accessor:?} is not yet supported in PyType_FromSpec"),
            }

            slot_ptr = unsafe { slot_ptr.add(1) };
        }

        let class = vm.ctx.new_class(None, class_name, base.to_owned(), slots);
        class.init_type_data(vtable).unwrap();
        for attribute in attributes {
            let name = unsafe {
                CStr::from_ptr(attribute.name)
                    .to_str()
                    .expect("attribute name must be valid UTF-8")
            };
            let closure = attribute.closure;
            let getter = attribute.get;
            let getset = if let Some(setter) = attribute.set {
                todo!();
                unsafe {
                    vm.ctx.new_getset(
                        name,
                        &class,
                        |obj: PyObjectRef, vm: &VirtualMachine| {},
                        |obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine| {},
                    )
                }
            } else {
                let class = unsafe { &*((&*class) as *const _) };
                vm.ctx.new_readonly_getset(
                    name,
                    &class,
                    move |obj: PyObjectRef, vm: &VirtualMachine| {
                        let result = getter(obj.as_raw().cast_mut(), closure);
                        unsafe {
                            PyObjectRef::from_raw(
                                NonNull::new(result).expect("TODO handle error from c function"),
                            )
                        }
                    },
                )
            };
            class
                .attributes
                .write()
                .insert(vm.ctx.intern_str(name), getset.into_object());
        }
        for method in methods {
            let class_static = unsafe { &*((&*class) as *const _) };
            let name = unsafe {
                CStr::from_ptr(method.ml_name)
                    .to_str()
                    .expect("method name must be valid UTF-8")
            };
            let method = build_method_def(vm, method, None).build_method(class_static, vm);
            class
                .attributes
                .write()
                .insert(vm.ctx.intern_str(name), method.into());
        }
        Ok(class.into())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    // TODO: Implement immutable type freezing semantics.
    0
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
pub extern "C" fn PyObject_GetAttrString(
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
    use pyo3::types::{PyBool, PyDict, PyInt, PyNone, PyString, PyType};

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

    #[test]
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

            #[staticmethod]
            fn static_method(a: i32, b: i32) -> PyResult<i32> {
                Ok(a + b)
            }

            #[classmethod]
            fn cls_method(cls: &Bound<'_, PyType>) -> PyResult<i32> {
                // assert!(cls.is_subclass_of::<MyClass>()?);
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
                obj.call_method1("static_method", (5, 8))
                    .unwrap()
                    .extract::<i32>()
                    .unwrap(),
                13
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

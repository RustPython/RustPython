use crate::descrobject::{PyGetSetDef, PyMemberDef};
use crate::methodobject::PyMethodDef;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::slots::{PySlot, PySlotKind, PySlotType};
use crate::util::CStrExt;
use crate::util::FfiPtrExt;
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr;
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::function::{ItemDoc, PyMethodFlags};
use rustpython_vm::types::{
    AtomicPyTypeFlags, CNewFunc, CSlotId, CSlots, PyTypeFlags, PyTypeSlots,
    SlotAccessor::{TpBase, TpDealloc, TpDoc, TpGetset, TpMembers, TpMethods, TpNew, TpTraverse},
};
use rustpython_vm::{AsObject, Py, PyObject, PyPayload, PyResult, VirtualMachine, identifier};

pub type PyTypeObject = Py<PyType>;

define_py_check!(fn PyType_Check, types.type_type);
define_py_check!(exact fn PyType_CheckExact, types.type_type);

#[repr(C)]
pub struct PyType_Slot {
    pub slot: c_int,
    pub pfunc: *mut c_void,
}

#[repr(C)]
pub struct PyType_Spec {
    pub name: *const c_char,
    pub basicsize: c_int,
    pub itemsize: c_int,
    pub flags: c_uint,
    pub slots: *mut PyType_Slot,
}

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
    ty.slots.flags.load().bits() as u32 as c_ulong
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
        let module = ty.__module__(vm)?;

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
pub unsafe extern "C" fn PyType_FromSlots(slots: *const PySlot) -> *mut PyObject {
    with_vm(|vm| {
        let mut name = None;
        let mut base = None;
        let mut methods = Vec::new();
        let mut type_slots: PyTypeSlots = Default::default();
        let attrs = Default::default();
        let mut getsets = Vec::new();
        let mut members = Vec::new();
        let mut metaclass = vm.ctx.types.type_type.to_owned();
        let mut tp_new: Option<CNewFunc> = None;

        for slot in PySlot::iter(slots) {
            match slot.as_kind(vm)? {
                PySlotKind::Type(type_slot) => {
                    match type_slot {
                        PySlotType::Name(value) => name = Some(value),
                        PySlotType::Metaclass(meta) => metaclass = meta.to_owned(),
                        PySlotType::Module(_) => {}
                        PySlotType::Flags(value) => {
                            let flags = PyTypeFlags::from_bits(value).ok_or_else(|| {
                                vm.new_value_error(format!(
                                    "Invalid type flags: {value:#x} for PyType_FromSlots"
                                ))
                            })?;
                            type_slots.flags = AtomicPyTypeFlags::from_plain(flags);
                        }
                        PySlotType::BasicSize(size)
                        | PySlotType::ExtraBasicSize(size)
                        | PySlotType::ItemSize(size) => {
                            if size != 0 {
                                return Err(vm.new_not_implemented_error(
                                    "PyType_FromSlots with non-zero size is not yet supported",
                                ));
                            }
                        }
                        PySlotType::TypeSlot(TpDoc, pfunc) => {
                            let doc = unsafe { pfunc.cast::<c_char>().try_as_str_opt(vm)? }.map_or(
                                ItemDoc::NONE,
                                |doc| {
                                    let text: &'static str =
                                        Box::leak(doc.to_owned().into_boxed_str());
                                    ItemDoc::static_text(text)
                                },
                            );
                            type_slots.doc = doc;
                        }
                        PySlotType::TypeSlot(TpBase, pfunc) => {
                            base = unsafe { Some(&*pfunc.cast::<PyTypeObject>()) }
                        }
                        PySlotType::TypeSlot(TpMembers, pfunc) => {
                            members.extend(PyMemberDef::iter(pfunc.cast()));
                        }
                        PySlotType::TypeSlot(TpGetset, pfunc) => {
                            getsets.extend(PyGetSetDef::iter(pfunc.cast()));
                        }
                        PySlotType::TypeSlot(TpMethods, pfunc) => {
                            for def in PyMethodDef::iter(pfunc.cast()) {
                                let name = unsafe { def.ml_name.try_as_str(vm)? };
                                let is_static = PyMethodFlags::from_bits_retain(def.ml_flags as _)
                                    .contains(PyMethodFlags::STATIC);
                                let method = def.build_method_def(vm)?;
                                methods.push((name, is_static, method));
                            }
                        }
                        PySlotType::TypeSlot(TpNew, pfunc) => {
                            tp_new = Some(unsafe {
                                core::mem::transmute::<*mut c_void, CNewFunc>(pfunc)
                            });
                        }
                        PySlotType::TypeSlot(TpDealloc | TpTraverse | _, _) => {
                            // TODO
                        }
                    }
                }
                PySlotKind::Module(_) => {
                    return Err(
                        vm.new_system_error("Got module slot while type slots are expected")
                    );
                }
                PySlotKind::Unknown { .. } => {}
            }
        }

        let bases = if let Some(base) = base {
            vec![base.to_owned()]
        } else {
            vec![vm.ctx.types.object_type.to_owned()]
        };

        let Some(name) = name else {
            return Err(vm.new_system_error("PyType_FromSlots requires a name slot"));
        };

        let class = PyType::new_heap(name, bases, attrs, type_slots, metaclass, &vm.ctx).map_err(
            |msg| vm.new_system_error(format!("Failed to create type from slots: {msg}")),
        )?;
        if let Some(tp_new) = tp_new {
            set_tp_new(vm, &class, tp_new)?;
        }

        let attrs = &class.attributes;
        let class_static = unsafe { &*((&*class) as *const _) };
        for (name, is_static, method) in methods {
            let method = if is_static {
                // Static functions bind the defining class for metadata, not as
                // a Python argument. They must not use a method descriptor,
                // whose call protocol consumes its first argument as self.
                let function = method.build_function(vm, Some(class.to_owned().into()));
                if let Some(module) = attrs.get(identifier!(vm, __module__)) {
                    function.as_object().set_attr("__module__", module, vm)?;
                }
                rustpython_vm::builtins::PyStaticMethod::new(function.into())
                    .into_ref(&vm.ctx)
                    .into()
            } else {
                method.build_method(class_static, vm).into()
            };
            attrs.insert(vm.ctx.intern_str(name), method);
        }
        for getset in getsets {
            let name = unsafe { getset.name.try_as_str(vm)? };
            attrs.insert(
                vm.ctx.intern_str(name),
                getset.build(class_static, vm)?.into(),
            );
        }
        for member in members {
            let name = unsafe { member.name.try_as_str(vm)? };
            attrs.insert(
                vm.ctx.intern_str(name),
                member.build(class_static, vm)?.into(),
            );
        }

        Ok(class)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromMetaclass(
    metaclass: *mut PyTypeObject,
    module: *mut PyObject,
    spec: *mut PyType_Spec,
    bases: *mut PyObject,
) -> *mut PyObject {
    let &PyType_Spec {
        name,
        basicsize,
        itemsize,
        flags,
        slots: type_slots,
    } = unsafe { &*spec };
    let mut slots = [PySlot::SLOT_END; 9];
    let mut write_slot = {
        let mut iter = slots.iter_mut();
        move |slot| {
            debug_assert!(
                iter.len() >= 2,
                "PyType_FromMetaclass: last slot is reserved for SLOT_END"
            );
            let entry = iter
                .next()
                .expect("Too many slots for PyType_FromMetaclass");
            *entry = slot;
        }
    };

    write_slot(PySlot::new(95, name.cast_mut()));
    write_slot(PySlot::new(96, basicsize as isize));
    write_slot(PySlot::new(98, itemsize as isize));
    write_slot(PySlot::new(99, flags as u64));
    write_slot(PySlot::new(93, type_slots));

    if !bases.is_null() {
        write_slot(PySlot::new(49, bases));
    }

    if !metaclass.is_null() {
        write_slot(PySlot::new(107, metaclass));
    }

    if !module.is_null() {
        write_slot(PySlot::new(108, module));
    }

    unsafe { PyType_FromSlots(slots.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromModuleAndSpec(
    module: *mut PyObject,
    spec: *mut PyType_Spec,
    bases: *mut PyObject,
) -> *mut PyObject {
    unsafe { PyType_FromMetaclass(ptr::null_mut(), module, spec, bases) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpecWithBases(
    spec: *mut PyType_Spec,
    bases: *mut PyObject,
) -> *mut PyObject {
    unsafe { PyType_FromMetaclass(ptr::null_mut(), ptr::null_mut(), spec, bases) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSpec(spec: *mut PyType_Spec) -> *mut PyObject {
    unsafe { PyType_FromMetaclass(ptr::null_mut(), ptr::null_mut(), spec, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetTypeData(
    obj: *mut PyObject,
    cls: *mut PyTypeObject,
) -> *mut c_void {
    let cls = unsafe { cls.assume_borrowed() };
    let base_basicsize = cls.base.deref().map_or(0, |base| base.slots.basicsize);
    let own_basicsize = cls.slots.basicsize.saturating_sub(base_basicsize);

    if own_basicsize == 0 {
        obj.cast()
    } else {
        todo!("PyObject_GetTypeData for non-zero sized types is not yet implemented")
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetTypeData_DuringGC(
    obj: *mut PyObject,
    cls: *mut PyTypeObject,
) -> *mut c_void {
    unsafe { PyObject_GetTypeData(obj, cls) }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    0
}

// C level type slots.
//
// Slots cross the boundary in both directions. A `newfunc` from an extension
// goes into the type's `CSlots` table and `new` gets the trampoline that
// reads it; the table pointer is inherited alongside the trampoline. A slot
// implemented in Rust is paired with a static C function of that same
// implementation, and `PyType_GetSlot` returns it — for a subclass, the
// base's function — so the caller can run that body with its own subtype.
//
// `__new__` is the same wrapper a native type gets, so a call through it is
// checked by `PyType::__new__` before it reaches the slot.

#[allow(non_camel_case_types)]
pub type newfunc = CNewFunc;

#[allow(non_upper_case_globals)]
pub const Py_tp_new: c_int = CSlotId::TpNew as c_int;

/// Install a C `newfunc` as the type's tp_new.
///
/// Fails on a static type, on a type that already has C slots, and on a type
/// that defines `__new__` itself, since the C slot would then be shadowed.
pub fn set_tp_new(vm: &VirtualMachine, ty: &Py<PyType>, tp_new: newfunc) -> PyResult<()> {
    if ty.attributes.contains(identifier!(vm, __new__)) {
        return Err(vm.new_type_error(format!("type '{}' already defines __new__", ty.name())));
    }
    let c_slots = CSlots::new();
    c_slots.new.store(Some(tp_new));
    ty.set_c_slots(c_slots, vm)?;

    // The wrapper that reaches the slot the checked way, as a native type gets
    // from `extend_class`. Stored without the attribute protocol so that
    // `update_slot` does not read it back as a definition of `__new__`.
    let def = vm.ctx.new_method_def(
        "__new__",
        rustpython_vm::function::ReceiverFunction::<_, true>(PyType::__new__),
        PyMethodFlags::METHOD,
        ItemDoc::NONE,
    );
    let wrapper = def.build_function(vm, Some(ty.to_owned().into()));
    ty.set_attr(identifier!(vm, __new__), wrapper.into());
    Ok(())
}

/// The C function installed in `slot`.
///
/// `Py_tp_new` is reported for a function installed from C and for a Rust
/// `tp_new`, including one a subclass inherited. Any other id reads as empty.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    let ty = unsafe { &*ty };
    match CSlotId::from_raw(slot) {
        Some(CSlotId::TpNew) => ty.c_tp_new().map_or(ptr::null_mut(), |f| f as *mut c_void),
        None => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use pyo3::IntoPyObjectExt;
    use pyo3::exceptions::PyValueError;
    use pyo3::ffi;
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyInt, PyList, PyString, PyType, PyTypeMethods};

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
    #[ignore]
    fn rust_class() {
        #[pyclass]
        struct MyClass {
            #[pyo3(get)]
            num: i32,
        }

        #[pymethods]
        impl MyClass {
            #[new]
            fn new(value: i32) -> Self {
                Self { num: value }
            }

            fn method1(&self) -> i32 {
                self.num + 10
            }

            fn method2(&self, a: i32) -> i32 {
                self.num + a
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
        });
    }

    #[test]
    #[ignore]
    fn rust_class_with_member() {
        #[pyclass(frozen)]
        struct MyClass {
            #[pyo3(get)]
            value: Py<PyAny>,
        }

        Python::attach(|py| {
            let obj = Bound::new(
                py,
                MyClass {
                    value: 1.into_bound_py_any(py).unwrap().unbind(),
                },
            )
            .unwrap();

            let globals = PyDict::new(py);
            globals.set_item("instance", &obj).unwrap();
            py.run(c"assert instance.value is None", Some(&globals), None)
                .unwrap();
        });
    }

    #[test]
    fn zero_sized_class() {
        #[pyclass(frozen)]
        struct MyEmptyClass {}

        #[pymethods]
        impl MyEmptyClass {
            #[new]
            fn new() -> Self {
                Self {}
            }

            #[staticmethod]
            fn static_method1(a: i32, b: i32) -> i32 {
                a + b
            }

            #[staticmethod]
            fn static_method2() -> i32 {
                0
            }

            #[classmethod]
            fn cls_method(cls: &Bound<'_, PyType>) -> PyResult<i32> {
                assert!(cls.is_subclass_of::<Self>()?);
                Ok(10)
            }
        }

        Python::attach(|py| {
            let obj = Bound::new(py, MyEmptyClass {}).unwrap();

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

    /// A `#[new]` with arguments runs through `Py_tp_new` from Python code:
    /// the result has the pyclass type, its error is raised, and a Python
    /// subclass of a final class is refused.
    #[test]
    fn pyclass_new_from_python() {
        #[pyclass(frozen)]
        struct Probe {}

        #[pymethods]
        impl Probe {
            #[new]
            fn new(x: i32) -> PyResult<Self> {
                if x < 0 {
                    return Err(PyValueError::new_err("negative"));
                }
                Ok(Self {})
            }
        }

        Python::attach(|py| {
            let globals = PyDict::new(py);
            globals.set_item("Probe", py.get_type::<Probe>()).unwrap();
            py.run(
                c"obj = Probe(1)\nassert type(obj) is Probe",
                Some(&globals),
                None,
            )
            .unwrap();

            let err = py.run(c"Probe(-1)", Some(&globals), None).unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py));
            assert_eq!(err.value(py).to_string(), "negative");

            let err = py
                .run(c"class Sub(Probe): pass", Some(&globals), None)
                .unwrap_err();
            assert!(err.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        });
    }

    #[test]
    fn subclass_with_property() {
        #[pyclass(frozen, extends=PyList)]
        struct MyList {}

        #[pymethods]
        impl MyList {
            #[new]
            fn new() -> Self {
                Self {}
            }

            #[getter]
            fn name(&self) -> &'static str {
                "Some list"
            }
        }

        Python::attach(|py| {
            let obj = Bound::new(py, MyList {}).unwrap();

            assert!(obj.is_instance_of::<PyList>());

            assert_eq!(
                obj.getattr("name")
                    .unwrap()
                    .cast_into::<PyString>()
                    .unwrap(),
                "Some list"
            );
        });
    }

    #[test]
    fn type_from_spec() {
        Python::attach(|py| {
            let mut slots = [ffi::PyType_Slot {
                slot: 0,
                pfunc: core::ptr::null_mut(),
            }];
            let mut spec = ffi::PyType_Spec {
                name: c"somemodule.MySpecClass".as_ptr(),
                basicsize: 0,
                itemsize: 0,
                flags: ffi::Py_TPFLAGS_DEFAULT as _,
                slots: slots.as_mut_ptr(),
            };

            let type_ptr = unsafe { ffi::PyType_FromSpec(&mut spec) };
            let type_obj = unsafe {
                Bound::from_borrowed_ptr_or_err(py, type_ptr)
                    .unwrap()
                    .cast_into::<PyType>()
                    .unwrap()
            };

            assert_eq!(type_obj.qualname().unwrap(), "somemodule.MySpecClass");
        });
    }

    #[test]
    fn fast_subclass_check_using_flags() {
        Python::attach(|py| {
            assert!(unsafe {
                ffi::PyType_HasFeature(
                    py.get_type::<PyDict>().as_type_ptr(),
                    ffi::Py_TPFLAGS_DICT_SUBCLASS,
                ) != 0
            });
            assert!(unsafe {
                ffi::PyType_HasFeature(
                    py.get_type::<PyValueError>().as_type_ptr(),
                    ffi::Py_TPFLAGS_BASE_EXC_SUBCLASS,
                ) != 0
            });
        })
    }
}

#[cfg(test)]
mod c_slot_tests {
    use super::*;
    use crate::PyObject;
    use core::ptr::NonNull;
    use pyo3::Python;
    use rustpython_vm::builtins::{PyInt, PyStrRef, PyTuple, PyTypeRef};
    use rustpython_vm::function::{FuncArgs, KwArgs};
    use rustpython_vm::vm::thread::{current_vm_is_set, with_current_vm};
    use rustpython_vm::{AsObject, PyObjectRef, PyRef};

    fn load_tp_new(ty: &PyTypeObject) -> newfunc {
        let slot = unsafe { PyType_GetSlot(ty, Py_tp_new) };
        assert!(!slot.is_null());
        unsafe { core::mem::transmute::<*mut c_void, newfunc>(slot) }
    }

    fn call_tp_new(tp_new: newfunc, subtype: &PyTypeObject, args: PyRef<PyTuple>) -> PyObjectRef {
        let ptr = unsafe {
            tp_new(
                core::ptr::from_ref(subtype).cast_mut(),
                args.as_object().as_raw().cast_mut(),
                core::ptr::null_mut(),
            )
        };
        let Some(ptr) = NonNull::new(ptr) else {
            let msg = with_current_vm(|vm| {
                vm.take_raised_exception()
                    .and_then(|exc| exc.as_object().str(vm).ok())
                    .map_or_else(|| "no exception".to_owned(), |msg| msg.to_string())
            });
            panic!("tp_new returned NULL: {msg}");
        };
        unsafe { PyObjectRef::from_raw(ptr) }
    }

    /// `PyType_GetSlot(base, Py_tp_new)(subtype, args, NULL)`.
    ///
    /// A missing slot sets TypeError ("base type without tp_new") and returns NULL.
    /// The pointer from the base, including NULL, is returned unchanged.
    fn call_base_tp_new(
        vm: &VirtualMachine,
        base: &PyTypeObject,
        subtype: *mut PyTypeObject,
        args: &PyRef<PyTuple>,
    ) -> *mut PyObject {
        let slot = unsafe { PyType_GetSlot(base, Py_tp_new) };
        if slot.is_null() {
            vm.set_raised_exception(Some(vm.new_type_error("base type without tp_new")));
            return core::ptr::null_mut();
        }
        let tp_new = unsafe { core::mem::transmute::<*mut c_void, newfunc>(slot) };
        unsafe {
            tp_new(
                subtype,
                args.as_object().as_raw().cast_mut(),
                core::ptr::null_mut(),
            )
        }
    }

    /// Returns `(subtype.__name__, args, kwds is NULL)`.
    unsafe extern "C" fn echo_new(
        subtype: *mut PyTypeObject,
        args: *mut PyObject,
        kwds: *mut PyObject,
    ) -> *mut PyObject {
        assert!(current_vm_is_set());
        with_current_vm(|vm| {
            let subtype = unsafe { &*subtype };
            let args = unsafe { &*args }.to_owned();
            let echo = vm.ctx.new_tuple(vec![
                vm.ctx.new_str(subtype.name().to_string()).into(),
                args,
                vm.ctx.new_bool(kwds.is_null()).into(),
            ]);
            PyObjectRef::from(echo).into_raw().as_ptr()
        })
    }

    /// Returns an empty tuple, so it is told apart from `echo_new` by result.
    unsafe extern "C" fn other_new(
        _subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            PyObjectRef::from(vm.ctx.new_tuple(vec![]))
                .into_raw()
                .as_ptr()
        })
    }

    unsafe extern "C" fn new_with_object_base(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            call_base_tp_new(vm, vm.ctx.types.object_type, subtype, &vm.ctx.empty_tuple)
        })
    }

    unsafe extern "C" fn new_with_int_base(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            call_base_tp_new(vm, vm.ctx.types.int_type, subtype, &vm.ctx.empty_tuple)
        })
    }

    /// Same as [`new_with_int_base`], but the base receives `("nope",)`.
    unsafe extern "C" fn new_with_int_base_nope(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("nope").into()]);
            call_base_tp_new(vm, vm.ctx.types.int_type, subtype, &args)
        })
    }

    fn heap_type(name: &str, base: &Py<PyType>, vm: &VirtualMachine) -> PyTypeRef {
        PyType::new_simple_heap(name, base, &vm.ctx).unwrap()
    }

    fn call(ty: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyRef<PyTuple> {
        ty.as_object()
            .call(args, vm)
            .unwrap()
            .downcast::<PyTuple>()
            .unwrap()
    }

    fn echoed_name(echo: &PyTuple) -> String {
        let name: PyStrRef = echo[0].clone().downcast().unwrap();
        name.to_string()
    }

    #[test]
    fn c_tp_new_is_called() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CType", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ty, echo_new).unwrap();

                let echo = call(&ty, FuncArgs::default(), vm);
                assert_eq!(echoed_name(&echo), "CType");
                assert_eq!(echo[1].clone().downcast::<PyTuple>().unwrap().len(), 0);
                // No keywords were passed, so kwds must be NULL.
                assert!(echo[2].clone().try_to_bool(vm).unwrap());

                let echo = call(&ty, FuncArgs::from(vec![vm.ctx.new_int(1).into()]), vm);
                assert_eq!(echo[1].clone().downcast::<PyTuple>().unwrap().len(), 1);

                let kwargs: KwArgs =
                    core::iter::once(("k".to_owned(), vm.ctx.new_int(2).into())).collect();
                let echo = call(&ty, FuncArgs::new(vec![], kwargs), vm);
                assert!(!echo[2].clone().try_to_bool(vm).unwrap());
            })
        })
    }

    /// `Py_tp_new` in a type spec lands in the type's table: the type
    /// instantiates through it, `__new__` reaches it the checked way, and
    /// `PyType_GetSlot` hands the same function back.
    #[test]
    fn spec_tp_new_is_installed() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let mut slots = [
                    pyo3::ffi::PyType_Slot {
                        slot: pyo3::ffi::Py_tp_new,
                        pfunc: echo_new as *mut c_void,
                    },
                    pyo3::ffi::PyType_Slot {
                        slot: 0,
                        pfunc: core::ptr::null_mut(),
                    },
                ];
                let mut spec = pyo3::ffi::PyType_Spec {
                    name: c"SpecNew".as_ptr(),
                    basicsize: 0,
                    itemsize: 0,
                    flags: pyo3::ffi::Py_TPFLAGS_DEFAULT as _,
                    slots: slots.as_mut_ptr(),
                };
                let ptr = unsafe { pyo3::ffi::PyType_FromSpec(&mut spec) };
                let ptr = NonNull::new(ptr.cast::<PyObject>()).expect("PyType_FromSpec failed");
                let ty: PyTypeRef = unsafe { PyObjectRef::from_raw(ptr) }.downcast().unwrap();

                assert_eq!(echoed_name(&call(&ty, FuncArgs::default(), vm)), "SpecNew");
                assert_eq!(
                    unsafe { PyType_GetSlot(&*ty, Py_tp_new) },
                    echo_new as *mut c_void
                );

                let via_dunder = vm
                    .call_method(
                        ty.as_object(),
                        "__new__",
                        FuncArgs::from(vec![ty.clone().into()]),
                    )
                    .unwrap()
                    .downcast::<PyTuple>()
                    .unwrap();
                assert_eq!(echoed_name(&via_dunder), "SpecNew");
            })
        })
    }

    /// A subclass reaches the C slot through the inherited slot pair, and the
    /// type it is instantiated with is the one handed to the slot.
    #[test]
    fn c_tp_new_is_inherited() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("CBase", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("CSub", &base, vm);

                assert!(sub.slots.c_slots().and_then(|c| c.new.load()).is_some());
                assert_eq!(echoed_name(&call(&sub, FuncArgs::default(), vm)), "CSub");
            })
        })
    }

    /// Every C type reaches its slot through one shared trampoline, so the
    /// "is not safe" check has to compare the C functions behind it, not the
    /// trampoline. Without that, `CBase.__new__(CSub)` would silently run
    /// CSub's tp_new where the caller asked for CBase's.
    #[test]
    fn cross_type_dunder_new_call_is_rejected() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("COuter", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("CInner", &base, vm);
                set_tp_new(vm, &sub, other_new).unwrap();

                // Each type reaches its own C function.
                assert_eq!(echoed_name(&call(&base, FuncArgs::default(), vm)), "COuter");
                assert!(call(&sub, FuncArgs::default(), vm).is_empty());

                // But COuter.__new__(CInner) must not reach CInner's.
                let dunder_new = base
                    .as_object()
                    .get_attr(identifier!(vm, __new__), vm)
                    .unwrap();
                let err = dunder_new.call((sub,), vm).unwrap_err();
                let msg = err.as_object().str(vm).unwrap().to_string();
                assert!(
                    msg.contains("is not safe"),
                    "expected an is-not-safe error, got {msg}"
                );
            })
        })
    }

    /// `__new__` reaches the slot through `PyType::__new__`, so a direct call
    /// is argument-checked instead of handing the raw pointer to the callee.
    #[test]
    fn direct_dunder_new_call_is_checked() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CChecked", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ty, echo_new).unwrap();
                let dunder_new = ty
                    .as_object()
                    .get_attr(identifier!(vm, __new__), vm)
                    .unwrap();

                // No type argument at all.
                assert!(dunder_new.call((), vm).is_err());
                // First argument is not a type.
                assert!(dunder_new.call((vm.ctx.new_int(42),), vm).is_err());
                // First argument is a type, but not a subtype of this one.
                assert!(
                    dunder_new
                        .call((vm.ctx.types.dict_type.to_owned(),), vm)
                        .is_err()
                );

                // The type itself and a subclass of it are accepted.
                let echo = dunder_new
                    .call((ty.clone(),), vm)
                    .unwrap()
                    .downcast::<PyTuple>()
                    .unwrap();
                assert_eq!(echoed_name(&echo), "CChecked");

                let sub = heap_type("CCheckedSub", &ty, vm);
                let echo = dunder_new
                    .call((sub,), vm)
                    .unwrap()
                    .downcast::<PyTuple>()
                    .unwrap();
                assert_eq!(echoed_name(&echo), "CCheckedSub");
            })
        })
    }

    /// A Python-level `__new__` on a subclass replaces the inherited C slot.
    #[test]
    fn python_subclass_new_overrides_the_slot() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("COverBase", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("COverSub", &base, vm);

                let py_new = vm.ctx.new_method_def(
                    "__new__",
                    |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                        Ok(vm.ctx.new_str("from python").into())
                    },
                    PyMethodFlags::STATIC,
                    ItemDoc::NONE,
                );
                let py_new = py_new.build_function(vm, None);
                sub.as_object()
                    .set_attr(identifier!(vm, __new__), py_new, vm)
                    .unwrap();

                assert!(sub.slots.c_slots().is_none());
                let obj = sub.as_object().call((), vm).unwrap();
                let obj: PyStrRef = obj.downcast().unwrap();
                assert_eq!(obj.to_string(), "from python");
            })
        })
    }

    /// A type whose own dict defines `__new__` keeps it: installing a C
    /// tp_new under it is refused instead of being silently shadowed.
    #[test]
    fn set_tp_new_rejects_a_type_defining_dunder_new() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CHasNew", vm.ctx.types.object_type, vm);
                let py_new = vm.ctx.new_method_def(
                    "__new__",
                    |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                        Ok(vm.ctx.new_str("from python").into())
                    },
                    PyMethodFlags::STATIC,
                    ItemDoc::NONE,
                );
                let py_new = py_new.build_function(vm, None);
                ty.as_object()
                    .set_attr(identifier!(vm, __new__), py_new, vm)
                    .unwrap();

                let err = set_tp_new(vm, &ty, echo_new).unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.type_error));
                assert!(ty.slots.c_slots().is_none());
                // The slot still routes to the Python `__new__`, not to C.
                assert_ne!(
                    unsafe { PyType_GetSlot(&*ty, Py_tp_new) },
                    echo_new as *mut c_void
                );

                let obj = ty.as_object().call((), vm).unwrap();
                let obj: PyStrRef = obj.downcast().unwrap();
                assert_eq!(obj.to_string(), "from python");

                // Installing twice is refused as well.
                let ty = heap_type("CTwice", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ty, echo_new).unwrap();
                let err = set_tp_new(vm, &ty, other_new).unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.type_error));
                assert_eq!(
                    unsafe { PyType_GetSlot(&*ty, Py_tp_new) },
                    echo_new as *mut c_void
                );
            })
        })
    }

    #[test]
    fn get_slot_round_trips() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CGet", vm.ctx.types.object_type, vm);
                // Inherits object's Rust tp_new until a C function is installed.
                assert_eq!(unsafe { PyType_GetSlot(&*ty, Py_tp_new) }, unsafe {
                    PyType_GetSlot(vm.ctx.types.object_type, Py_tp_new)
                },);

                set_tp_new(vm, &ty, echo_new).unwrap();
                assert_eq!(
                    unsafe { PyType_GetSlot(&*ty, Py_tp_new) },
                    echo_new as *mut c_void
                );

                // Inherited by pointer, as tp_new is.
                let sub = heap_type("CGetSub", &ty, vm);
                assert_eq!(
                    unsafe { PyType_GetSlot(&*sub, Py_tp_new) },
                    echo_new as *mut c_void
                );
            })
        })
    }

    #[test]
    fn rust_int_tp_new_builds_an_int_and_a_subclass() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let tp_new = load_tp_new(int_type);

                let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("42").into()]);
                let value: PyRef<PyInt> = call_tp_new(tp_new, int_type, args).downcast().unwrap();
                assert_eq!(value.as_bigint().to_string(), "42");

                let sub = heap_type("SubInt", int_type, vm);
                let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("42").into()]);
                let obj = call_tp_new(tp_new, &sub, args);
                assert!(obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn rust_object_tp_new_is_callable() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let object_type = vm.ctx.types.object_type;
                let tp_new = load_tp_new(object_type);
                let obj = call_tp_new(tp_new, object_type, vm.ctx.new_tuple(vec![]));
                assert!(obj.class().is(object_type));
            })
        })
    }

    #[test]
    fn inherited_int_tp_new_matches_int() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let int_slot = unsafe { PyType_GetSlot(int_type, Py_tp_new) };
                let sub = heap_type("InheritedInt", int_type, vm);
                assert!(!int_slot.is_null());
                assert_eq!(unsafe { PyType_GetSlot(&*sub, Py_tp_new) }, int_slot);
            })
        })
    }

    #[test]
    fn python_level_new_is_callable_from_c() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let sub = heap_type("PyNew", vm.ctx.types.object_type, vm);
                let py_new = vm.ctx.new_method_def(
                    "__new__",
                    |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                        Ok(vm.ctx.new_str("from python").into())
                    },
                    PyMethodFlags::STATIC,
                    ItemDoc::NONE,
                );
                let py_new = py_new.build_function(vm, None);
                sub.as_object()
                    .set_attr(identifier!(vm, __new__), py_new, vm)
                    .unwrap();

                let tp_new = load_tp_new(&sub);
                let obj = call_tp_new(tp_new, &sub, vm.ctx.new_tuple(vec![]));
                let obj: PyStrRef = obj.downcast().unwrap();
                assert_eq!(obj.to_string(), "from python");
            })
        })
    }

    #[test]
    fn rust_int_tp_new_sets_value_error() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let tp_new = load_tp_new(int_type);
                let args = vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_str("not a number").into()]);
                let ptr = unsafe {
                    tp_new(
                        core::ptr::from_ref(int_type).cast_mut(),
                        args.as_object().as_raw().cast_mut(),
                        core::ptr::null_mut(),
                    )
                };
                assert!(ptr.is_null());
                let exc = vm.take_raised_exception().expect("exception pending");
                assert!(exc.fast_isinstance(vm.ctx.exceptions.value_error));
            })
        })
    }

    /// pyo3's `PyNativeTypeInitializer` calls the base `tp_new` with the subtype
    /// it was given.
    #[test]
    fn extension_tp_new_reaches_object_tp_new_with_its_subtype() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ext, new_with_object_base).unwrap();

                let obj = ext.as_object().call((), vm).unwrap();
                assert!(obj.class().is(&ext));

                let sub = heap_type("ExtSub", &ext, vm);
                let sub_obj = sub.as_object().call((), vm).unwrap();
                assert!(sub_obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn extension_tp_new_reaches_int_tp_new_with_its_subtype() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.int_type, vm);
                set_tp_new(vm, &ext, new_with_int_base).unwrap();

                let obj = ext.as_object().call((), vm).unwrap();
                assert!(obj.class().is(&ext));
                let value: PyRef<PyInt> = obj.downcast().unwrap();
                assert_eq!(value.as_bigint().to_string(), "0");

                let sub = heap_type("ExtIntSub", &ext, vm);
                let sub_obj = sub.as_object().call((), vm).unwrap();
                assert!(sub_obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn extension_type_reports_its_own_c_tp_new_not_the_base() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let object_type = vm.ctx.types.object_type;
                let ext = heap_type("Ext", object_type, vm);
                set_tp_new(vm, &ext, new_with_object_base).unwrap();

                let ext_slot = unsafe { PyType_GetSlot(&*ext, Py_tp_new) };
                let base_slot = unsafe { PyType_GetSlot(object_type, Py_tp_new) };
                assert_eq!(ext_slot, new_with_object_base as *mut c_void);
                assert_ne!(ext_slot, base_slot);
                assert!(!base_slot.is_null());
            })
        })
    }

    #[test]
    fn extension_tp_new_propagates_base_error() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.int_type, vm);
                set_tp_new(vm, &ext, new_with_int_base_nope).unwrap();

                let err = ext.as_object().call((), vm).unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.value_error));
            })
        })
    }
}

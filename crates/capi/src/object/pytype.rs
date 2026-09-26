use crate::abstract_::{dict_to_kwargs, tuple_to_args};
use crate::descrobject::{PyGetSetDef, PyMemberDef};
use crate::methodobject::{PyMethodDef, build_method_def};
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::slots::{PySlot, PySlotKind, PySlotType};
use crate::util::CStrExt;
use crate::util::FfiPtrExt;
use core::ffi::{c_char, c_int, c_ulong, c_void};
use dashmap::DashMap;
use rustpython_vm::builtins::{PyDict, PyStr, PyType, PyTypeRef};
use rustpython_vm::function::{FuncArgs, PyMethodFlags};
use rustpython_vm::types::{PyTypeFlags, PyTypeSlots, SlotAccessor};
use rustpython_vm::{AsObject, Py, PyObject, PyResult, VirtualMachine};
use std::any::{Any, TypeId};
use std::sync::LazyLock;

pub type PyTypeObject = Py<PyType>;

define_py_check!(fn PyType_Check, types.type_type);
define_py_check!(exact fn PyType_CheckExact, types.type_type);

#[repr(C)]
pub struct PyType_Slot {
    pub slot: c_int,
    pub pfunc: *mut c_void,
}

impl PyType_Slot {
    pub(crate) fn iter<'a>(mut slots: *const Self) -> impl Iterator<Item = &'a Self> {
        core::iter::from_fn(move || {
            let slot = unsafe { &*slots };
            if slot.slot == 0 {
                None
            } else {
                slots = unsafe { slots.add(1) };
                Some(slot)
            }
        })
    }
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

fn get_c_tp_new<F: Any + Send + Sync>(
    rust_tp_new: F,
) -> extern "C" fn(*mut PyTypeObject, *mut PyObject, *mut PyObject) -> *mut PyObject
where
    F: Fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult,
{
    extern "C" fn trampoline<F: Any + Send + Sync>(
        subtype: *mut PyTypeObject,
        args: *mut PyObject,
        kwargs: *mut PyObject,
    ) -> *mut PyObject
    where
        F: Fn(PyTypeRef, FuncArgs, &VirtualMachine) -> PyResult,
    {
        let entry = SLOT_CACHE
            .get(&TypeId::of::<F>())
            .expect("could not find tp_new trampoline");
        let func = entry.downcast_ref::<F>().expect("trampoline type mismatch");
        with_vm(|vm| unsafe {
            let args = FuncArgs::new(
                tuple_to_args((&*args).try_downcast_ref(vm)?),
                kwargs
                    .as_ref()
                    .map(|kwargs| dict_to_kwargs(vm, kwargs.try_downcast_ref::<PyDict>(vm)?))
                    .transpose()?
                    .unwrap_or_default(),
            );
            func((&*subtype).to_owned(), args, vm)
        })
    }

    static SLOT_CACHE: LazyLock<DashMap<TypeId, Box<dyn Any + Send + Sync>>> =
        LazyLock::new(DashMap::new);
    SLOT_CACHE
        .entry(TypeId::of::<F>())
        .or_insert_with(|| Box::new(rust_tp_new));
    trampoline::<F>
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    with_vm(|_vm| {
        let ty = unsafe { &*ty };
        let slot: u8 = slot
            .try_into()
            .expect("slot number out of range for SlotAccessor");
        let slot_accessor: SlotAccessor = slot
            .try_into()
            .expect("invalid slot number for SlotAccessor");

        match slot_accessor {
            SlotAccessor::TpNew => ty
                .slots
                .new
                .load()
                .map(|rust_tp_new| get_c_tp_new(rust_tp_new) as *mut c_void),
            _ => {
                todo!("Slot {slot_accessor:?} for {ty:?} is not yet implemented in PyType_GetSlot")
            }
        }
        .unwrap_or_default()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_FromSlots(slots: *const PySlot) -> *mut PyObject {
    with_vm(|vm| {
        let mut name = None;
        let mut base = None;
        let mut methods = Vec::new();
        let mut type_slots: PyTypeSlots = Default::default();
        let attrs = Default::default();
        let mut getsets = Vec::new();
        let mut members = Vec::new();

        for slot in PySlot::iter(slots) {
            match slot.as_kind(vm)? {
                kind @ PySlotKind::Type(type_slot) => {
                    match type_slot {
                        PySlotType::Name(value) => name = Some(value),
                        PySlotType::Flags(value) => {
                            type_slots.flags = PyTypeFlags::from_bits(value).ok_or_else(|| {
                                vm.new_value_error(format!(
                                    "Invalid type flags: {value:#x} for PyType_FromSlots"
                                ))
                            })?;
                        }
                        PySlotType::BasicSize(size) | PySlotType::ExtraBasicSize(size) => {
                            if size != 0 {
                                return Err(vm.new_not_implemented_error(
                                    "PyType_FromSlots with non-zero size is not yet supported",
                                ));
                            }
                        }
                        PySlotType::Slots { value, .. } => {
                            for slot in PyType_Slot::iter(value) {
                                let slot_id: u8 = slot.slot.try_into().unwrap();
                                match slot_id.try_into().unwrap() {
                                    SlotAccessor::TpDoc => {
                                        let doc = unsafe {
                                            slot.pfunc.cast::<c_char>().try_as_str_opt(vm)?
                                        };
                                        type_slots.doc = doc;
                                    }
                                    SlotAccessor::TpNew => {
                                        type_slots.new.store(Some(|ty, _args, vm| {
                                            Err(vm.new_not_implemented_error(format!("tp_new is not yet implemented in PyType_FromSlots for {ty:?}")))
                                        }));
                                    }
                                    SlotAccessor::TpBase => {
                                        base = unsafe { Some(&*slot.pfunc.cast::<PyTypeObject>()) }
                                    }
                                    SlotAccessor::TpDealloc => {
                                        // type_slots.del.store(Some(|_ty, _vm| {
                                        //     // TODO
                                        //     Ok(())
                                        // }));
                                    }
                                    SlotAccessor::TpMethods => {
                                        for def in PyMethodDef::iter(slot.pfunc.cast()) {
                                            let name = unsafe { def.ml_name.try_as_str(vm)? };
                                            let is_static =
                                                PyMethodFlags::from_bits_retain(def.ml_flags as _)
                                                    .contains(PyMethodFlags::STATIC);
                                            let method = build_method_def(vm, def, !is_static)?;
                                            methods.push((name, method));
                                        }
                                    }
                                    SlotAccessor::TpGetset => {
                                        getsets.extend(PyGetSetDef::iter(slot.pfunc.cast()));
                                    }
                                    SlotAccessor::TpMembers => {
                                        members.extend(PyMemberDef::iter(slot.pfunc.cast()));
                                    }
                                    slot => {
                                        return Err(vm.new_not_implemented_error(format!(
                                            "PyType_FromSlots with PyType_Slot {slot:?} not implemented yet"
                                        )));
                                    }
                                }
                            }
                        }
                        _ => {
                            return Err(vm.new_not_implemented_error(format!(
                                "PyType_FromSlots with slot {kind:?} not implemented yet"
                            )));
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

        let metaclass = vm.ctx.types.type_type.to_owned();
        let class = PyType::new_heap(name.unwrap(), bases, attrs, type_slots, metaclass, &vm.ctx)
            .map_err(|msg| {
            vm.new_system_error(format!("Failed to create type from slots: {msg}"))
        })?;

        let attrs = &class.attributes;
        let class_static = unsafe { &*((&*class) as *const _) };
        for (name, method) in methods {
            attrs.insert(
                vm.ctx.intern_str(name),
                method.build_method(class_static, vm).into(),
            );
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
pub unsafe extern "C" fn PyObject_GetTypeData(
    obj: *mut PyObject,
    cls: *mut PyTypeObject,
) -> *mut c_void {
    if unsafe { &*cls }.slots.basicsize == 0 {
        obj.cast()
    } else {
        todo!("PyObject_GetTypeData for non-zero sized types is not yet implemented")
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    0
}

#[cfg(test)]
mod tests {
    use pyo3::IntoPyObjectExt;
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyInt, PyString, PyType, PyTypeMethods};

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
}

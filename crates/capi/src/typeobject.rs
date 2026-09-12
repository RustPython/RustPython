//! C level type slots.
//!
//! The slot table keeps its Rust signatures, so a `newfunc` from an extension
//! goes into the type's [`CSlots`] table and `new` gets the trampoline that
//! reads it. The table pointer is inherited alongside the trampoline, so a
//! subclass reaches the C function without a lookup, the way an inherited
//! `tp_new` pointer does.
//!
//! `__new__` is the same wrapper a native type gets, so a call through it is
//! checked by `PyType::__new__` before it reaches the slot.

use crate::object::PyTypeObject;
use core::ffi::{c_int, c_void};
use core::ptr;
use rustpython_vm::builtins::PyType;
use rustpython_vm::function::PyMethodFlags;
use rustpython_vm::types::{CNewFunc, CSlotId, CSlots};
use rustpython_vm::{Py, PyResult, VirtualMachine, identifier};

#[allow(non_camel_case_types)]
pub type newfunc = CNewFunc;

#[allow(non_upper_case_globals)]
pub const Py_tp_new: c_int = CSlotId::TpNew as c_int;

/// Install a C `newfunc` as the type's tp_new.
///
/// `ty` must be a heap type that does not already define `__new__` itself.
pub fn set_tp_new(vm: &VirtualMachine, ty: &Py<PyType>, tp_new: newfunc) -> PyResult<()> {
    let c_slots = CSlots::new();
    c_slots.new.store(Some(tp_new));
    ty.set_c_slots(c_slots, vm)?;

    // The wrapper that reaches the slot the checked way, as a native type gets
    // from `extend_class`. Stored without the attribute protocol so that
    // `update_slot` does not read it back as a definition of `__new__`.
    let def = vm
        .ctx
        .new_method_def("__new__", PyType::__new__, PyMethodFlags::METHOD, None);
    let wrapper = def.build_function(vm, Some(ty.to_owned().into()));
    ty.set_attr(identifier!(vm, __new__), wrapper.into());
    Ok(())
}

/// Only slots installed from C are reported. A slot backed by a Rust function
/// has no C ABI entry point yet and reads as empty, as does a slot id this
/// layer does not handle.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    let ty = unsafe { &*ty };
    let Some(c_slots) = ty.slots.c_slots() else {
        return ptr::null_mut();
    };
    match CSlotId::from_raw(slot) {
        Some(CSlotId::TpNew) => c_slots
            .new
            .load()
            .map_or(ptr::null_mut(), |f| f as *mut c_void),
        None => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PyObject;
    use pyo3::Python;
    use rustpython_vm::builtins::{PyStrRef, PyTuple, PyTypeRef};
    use rustpython_vm::function::{FuncArgs, KwArgs};
    use rustpython_vm::vm::thread::{current_vm_is_set, with_current_vm};
    use rustpython_vm::{AsObject, PyObjectRef, PyRef};

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
                    None,
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

    #[test]
    fn get_slot_round_trips() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CGet", vm.ctx.types.object_type, vm);
                assert!(unsafe { PyType_GetSlot(&*ty, Py_tp_new) }.is_null());

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
}

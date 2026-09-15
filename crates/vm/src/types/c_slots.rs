//! Type slots supplied by an extension module.
//!
//! The slot table keeps its Rust signatures. A slot filled from C is held here
//! instead, and the matching Rust slot gets a trampoline that marshals the
//! arguments and calls it. A type that has any of these owns one [`CSlots`],
//! and [`PyTypeSlots::c_slots`] points at it; a subclass inherits the pointer
//! alongside the trampoline, so it reaches the C function without a lookup the
//! way an inherited slot pointer does.
//!
//! Only the slots that can currently be filled from C are listed. Adding one
//! means adding a field here, a trampoline beside the ones below, and a
//! [`CSlotId`] entry.

use crate::{
    AsObject, PyObject, PyObjectRef, PyRef, PyResult,
    builtins::{PyDictRef, PyTuple, PyTypeRef},
    function::FuncArgs,
    types::CNewFunc,
    vm::VirtualMachine,
};
use core::ptr::NonNull;
use crossbeam_utils::atomic::AtomicCell;

/// The C functions an extension supplied for one type.
///
/// Each entry is written once, when the type is built, and read through a
/// shared pointer afterwards.
#[derive(Default)]
pub struct CSlots {
    /// tp_new. Reached through [`c_new_trampoline`], never called directly.
    pub new: AtomicCell<Option<CNewFunc>>,
}

impl CSlots {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Slot ids from `typeslots.h`. Only the ones that can be installed are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CSlotId {
    TpNew = 65,
}

impl CSlotId {
    /// The id an extension passes in a `PyType_Slot`, or `None` when this layer
    /// does not handle it.
    #[must_use]
    pub const fn from_raw(id: i32) -> Option<Self> {
        match id {
            65 => Some(Self::TpNew),
            _ => None,
        }
    }
}

/// Split arguments into the `(args, kwds)` pair a keyword-taking C function
/// takes. A call without keywords yields no dict: the convention is to pass a
/// NULL kwds, which is what a function that rejects keywords tests for.
pub fn split_args(
    vm: &VirtualMachine,
    args: FuncArgs,
) -> PyResult<(PyRef<PyTuple>, Option<PyDictRef>)> {
    let arg_tuple = vm.ctx.new_tuple(args.args);
    if args.kwargs.is_empty() {
        return Ok((arg_tuple, None));
    }
    let dict = vm.ctx.new_dict();
    for (k, v) in args.kwargs {
        dict.set_item(&*k, v, vm)?;
    }
    Ok((arg_tuple, Some(dict)))
}

#[must_use]
pub fn kwargs_ptr(kwargs: Option<&PyDictRef>) -> *mut PyObject {
    kwargs.map_or(core::ptr::null_mut(), |d| d.as_object().as_raw().cast_mut())
}

/// Turn what a C function returned into a `PyResult`. A NULL return means the
/// exception it raised is pending.
pub fn ret_ptr_to_pyresult(vm: &VirtualMachine, ret_ptr: *mut PyObject) -> PyResult {
    let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("Native function returned NULL, but there was no exception set")
    })?;
    Ok(unsafe { PyObjectRef::from_raw(ret_ptr) })
}

/// Drives the C tp_new of the type it is called with. Installed in the `new`
/// slot, which every instantiation path goes through.
///
/// The C function is read off `cls` rather than captured, so a subclass that
/// inherited this trampoline reaches the same function and passes itself as
/// `subtype`, matching `type->tp_new(subtype, ...)`.
pub fn c_new_trampoline(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
    let tp_new = cls.slots.c_slots().and_then(|c| c.new.load());
    let tp_new = tp_new.ok_or_else(|| {
        vm.new_system_error(format!("type '{}' has no C tp_new to call", cls.name()))
    })?;
    let (arg_tuple, kwargs) = split_args(vm, args)?;
    let ret_ptr = unsafe {
        tp_new(
            core::ptr::from_ref(&*cls).cast_mut(),
            arg_tuple.as_object().as_raw().cast_mut(),
            kwargs_ptr(kwargs.as_ref()),
        )
    };
    ret_ptr_to_pyresult(vm, ret_ptr)
}

/// A pointer to the [`CSlots`] a type reaches, shared with every type that
/// inherited from its owner.
///
/// The table is owned by the heap type that installed it and lives as long as
/// that type. A type that holds this pointer keeps the owner alive through its
/// `mro`, so the target outlives every reader.
pub(crate) type CSlotsPtr = NonNull<CSlots>;

/// Wrappers a type's C slots are reached through, one per slot that can be
/// filled from C. Recognising these is how a slot that is only a trampoline is
/// told apart from one a type implements itself.
pub(crate) fn is_c_trampoline(func: crate::types::NewFunc) -> bool {
    crate::types::fn_addr(func) == crate::types::fn_addr(c_new_trampoline as crate::types::NewFunc)
}

/// The table a type owns. Boxed so the pointer handed to subclasses stays
/// valid for the owner's lifetime.
pub struct OwnedCSlots(Box<CSlots>);

impl OwnedCSlots {
    #[must_use]
    pub fn new(slots: CSlots) -> Self {
        Self(Box::new(slots))
    }

    #[must_use]
    pub(crate) fn as_ptr(&self) -> CSlotsPtr {
        CSlotsPtr::from(&*self.0)
    }
}

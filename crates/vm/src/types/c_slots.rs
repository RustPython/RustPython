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
//!
//! The reverse direction pairs a Rust slot function with an `extern "C"`
//! function instantiated for it in [`StaticCSlots`]. The pair is stored
//! on the type that defines the slot. [`PyType::c_tp_new`] walks the MRO and
//! returns that C function, so a subclass reports the base implementation
//! rather than its own.

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult,
    builtins::{PyDict, PyDictRef, PyStr, PyTuple, PyType, PyTypeRef},
    function::{FuncArgs, KwArgs},
    types::{CNewFunc, NewFunc},
    vm::{VirtualMachine, thread::with_current_vm},
};
use core::marker::PhantomData;
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

/// Build [`FuncArgs`] from the `(args, kwds)` pair a C caller passes.
///
/// `kwds == NULL` means there are no keywords. A keyword name that is not a
/// `str` is a `TypeError`.
///
/// # Safety
/// `args` must point at a live object and `kwds` must be NULL or point at a
/// live object.
pub(crate) unsafe fn func_args_from_ptrs(
    vm: &VirtualMachine,
    args: *mut PyObject,
    kwds: *mut PyObject,
) -> PyResult<FuncArgs> {
    let pos_args = unsafe { &*args }
        .try_downcast_ref::<PyTuple>(vm)?
        .iter()
        .cloned()
        .collect();
    let kwargs = if kwds.is_null() {
        KwArgs::default()
    } else {
        kwargs_from_dict(vm, unsafe { &*kwds }.try_downcast_ref::<PyDict>(vm)?)?
    };
    Ok(FuncArgs {
        args: pos_args,
        kwargs,
    })
}

fn kwargs_from_dict(vm: &VirtualMachine, dict: &Py<PyDict>) -> PyResult<KwArgs> {
    dict.items_vec()
        .into_iter()
        .map(|(key, value)| {
            // Keep WTF-8 so a lone-surrogate key round-trips.
            let key = key
                .downcast_ref::<PyStr>()
                .map(|key| key.as_wtf8().to_owned())
                .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
            Ok((key, value))
        })
        .collect()
}

/// Hand a `PyResult` back to C. `NULL` means `exc` is now the pending exception.
pub(crate) fn pyresult_to_ret_ptr(vm: &VirtualMachine, result: PyResult) -> *mut PyObject {
    match result {
        Ok(obj) => obj.into_raw().as_ptr(),
        Err(exc) => {
            vm.set_exception(Some(exc));
            core::ptr::null_mut()
        }
    }
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

/// A Rust `tp_new` that has a single statically instantiated C entry point.
pub trait StaticNew {
    fn call(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult;
}

/// [`StaticNew`] for a payload whose `tp_new` is [`Constructor::slot_new`].
pub struct ViaConstructor<T>(PhantomData<T>);

impl<T> StaticNew for ViaConstructor<T>
where
    T: crate::types::Constructor,
{
    fn call(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        <T as crate::types::Constructor>::slot_new(cls, args, vm)
    }
}

/// [`StaticNew`] for a Python-level `__new__`, dispatched through [`new_wrapper`].
pub struct PythonNew;

impl StaticNew for PythonNew {
    fn call(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        super::slot::new_wrapper(cls, args, vm)
    }
}

/// C ABI `newfunc` for the Rust implementation `S`.
///
/// Instantiated once per `S`, so the function pointer identifies that
/// implementation. A caller passes `subtype` through; `S` must run its own
/// body with that class rather than reading `subtype`'s slot again.
///
/// # Safety
/// `subtype` must point at a live type, `args` at a tuple, and `kwds` must be
/// NULL or a dict. The current thread must have a VM attached.
pub unsafe extern "C" fn c_new_for<S: StaticNew>(
    subtype: *mut Py<PyType>,
    args: *mut PyObject,
    kwds: *mut PyObject,
) -> *mut PyObject {
    with_current_vm(|vm| {
        let result = (|| {
            let cls = unsafe { &*subtype }.to_owned();
            let func_args = unsafe { func_args_from_ptrs(vm, args, kwds) }?;
            S::call(cls, func_args, vm)
        })();
        pyresult_to_ret_ptr(vm, result)
    })
}

/// One Rust slot function and the C function that calls it.
#[derive(Copy, Clone)]
pub struct CSlotPair<R, C> {
    pub rust: R,
    pub c: C,
}

/// The Rust slot functions a type defines, each paired with its C twin.
///
/// Another slot is another field. Only `new` is filled in today.
pub struct StaticCSlots {
    pub new: Option<CSlotPair<NewFunc, CNewFunc>>,
}

/// `new_wrapper` and the C function that calls it.
///
/// A heap type whose `new` slot is the Python `__new__` dispatcher resolves
/// here when no defining type recorded that function in its own table.
pub static PYTHON_C_SLOTS: StaticCSlots = StaticCSlots {
    new: Some(CSlotPair {
        rust: super::slot::new_wrapper as NewFunc,
        c: c_new_for::<PythonNew> as CNewFunc,
    }),
};

/// Associated [`StaticCSlots`] for the payload that defines a Rust `tp_new`.
///
/// Generated next to `#[pyclass(with(Constructor))]`. The const is what
/// promotes the table to `'static`.
pub trait HasStaticCSlots {
    const TABLE: StaticCSlots;
}

fn c_new_matching(table: Option<&StaticCSlots>, addr: usize) -> Option<CNewFunc> {
    let pair = table?.new.as_ref()?;
    (super::slot::fn_addr(pair.rust) == addr).then_some(pair.c)
}

impl PyType {
    /// The C `newfunc` for this type's `tp_new`.
    ///
    /// A slot installed from C is the function stored in [`CSlots`]. A Rust
    /// slot is the C twin recorded for the type in the MRO that defines that
    /// same Rust function. A Python-level `__new__` resolves to
    /// [`PYTHON_C_SLOTS`].
    #[must_use]
    pub fn c_tp_new(&self) -> Option<CNewFunc> {
        let new = self.slots.new.load()?;
        if is_c_trampoline(new) {
            return self.slots.c_slots().and_then(|c| c.new.load());
        }
        let addr = super::slot::fn_addr(new);
        let defined = c_new_matching(self.slots.static_c_slots.load(), addr).or_else(|| {
            let mro = self.mro.read();
            mro.iter()
                .find_map(|cls| c_new_matching(cls.slots.static_c_slots.load(), addr))
        });
        defined.or_else(|| c_new_matching(Some(&PYTHON_C_SLOTS), addr))
    }
}

/// A pointer to the [`CSlots`] a type reaches, shared with every type that
/// inherited from its owner.
///
/// The table is owned by the heap type that installed it and lives as long as
/// that type. A type that holds this pointer keeps the owner alive through its
/// `mro`, so the target outlives every reader.
pub(crate) type CSlotsPtr = NonNull<CSlots>;

/// Wrappers a type's C slots are reached through, one per slot that can be
/// filled from C. Recognizing these is how a slot that is only a trampoline is
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

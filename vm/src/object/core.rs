//! Essential types for object models
//!
//! +-------------------------+--------------+-----------------------+
//! |       Management        |       Typed      |      Untyped      |
//! +-------------------------+------------------+-------------------+
//! | Interpreter-independent | [`Py<T>`]        | [`PyObject`]      |
//! | Reference-counted       | [`PyRef<T>`]     | [`PyObjectRef`]   |
//! | Weak                    | [`PyWeakRef<T>`] | [`PyRef<PyWeak>`] |
//! +-------------------------+--------------+-----------------------+
//!
//! [`PyRef<PyWeak>`] may looking like to be called as PyObjectWeak by the rule,
//! but not to do to remember it is a PyRef object.
use super::{
    PyAtomicRef, PyDefault, SuperDefault, SuperPyDefault,
    ext::{AsObject, PyRefExact, PyResult},
    payload::PyPayload,
};
use crate::{
    Context,
    builtins::{PyBaseExceptionRef, PyBaseObject},
    object::traverse::{Traverse, TraverseFn},
};
use crate::{
    builtins::{PyDictRef, PyType, PyTypeRef},
    common::{
        atomic::{OncePtr, PyAtomic, Radium},
        linked_list::{Link, LinkedList, Pointers},
        lock::{PyMutex, PyMutexGuard, PyRwLock},
        refcount::RefCount,
    },
    vm::VirtualMachine,
};
use crate::{object::traverse_object::PyObjVTable, types::PyTypeFlags};
use itertools::Itertools;
use std::{
    any::TypeId,
    borrow::Borrow,
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::{self, NonNull},
};

// so, PyObjectRef is basically equivalent to `PyRc<PyInner<dyn PyPayload>>`, except it's
// only one pointer in width rather than 2. We do that by manually creating a vtable, and putting
// a &'static reference to it inside the `PyRc` rather than adjacent to it, like trait objects do.
// This can lead to faster code since there's just less data to pass around, as well as because of
// some weird stuff with trait objects, alignment, and padding.
//
// So, every type has an alignment, which means that if you create a value of it it's location in
// memory has to be a multiple of it's alignment. e.g., a type with alignment 4 (like i32) could be
// at 0xb7befbc0, 0xb7befbc4, or 0xb7befbc8, but not 0xb7befbc2. If you have a struct and there are
// 2 fields whose sizes/alignments don't perfectly fit in with each other, e.g.:
// +-------------+-------------+---------------------------+
// |     u16     |      ?      |            i32            |
// | 0x00 | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 |
// +-------------+-------------+---------------------------+
// There has to be padding in the space between the 2 fields. But, if that field is a trait object
// (like `dyn PyPayload`) we don't *know* how much padding there is between the `payload`
// field and the previous field. So, Rust has to consult the vtable to know the exact offset of
// `payload` in `PyInner<dyn PyPayload>`, which has a huge performance impact when *every
// single payload access* requires a vtable lookup. Thankfully, we're able to avoid that because of
// the way we use PyObjectRef, in that whenever we want to access the payload we (almost) always
// access it from a generic function. So, rather than doing
//
// - check vtable for payload offset
// - get offset in PyInner struct
// - call as_any() method of PyPayload
// - call downcast_ref() method of Any
// we can just do
// - check vtable that typeid matches
// - pointer cast directly to *const PyInner<T>
//
// and at that point the compiler can know the offset of `payload` for us because **we've given it a
// concrete type to work with before we ever access the `payload` field**

pub(super) unsafe fn drop_dealloc_obj<T: PyPayload>(x: *mut PyObject) {
    drop(unsafe { Box::from_raw(x as *mut PyObjRepr<T>) });
}
pub(super) unsafe fn debug_obj<T: PyPayload>(
    x: &PyObject,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    let x = unsafe { &*(x as *const PyObject as *const PyObjRepr<T>) };
    fmt::Debug::fmt(x, f)
}

/// Call `try_trace` on payload
pub(super) unsafe fn try_trace_obj<T: PyPayload>(x: &PyObject, tracer_fn: &mut TraverseFn<'_>) {
    let x = unsafe { &*(x as *const PyObject as *const PyObjRepr<T>) };
    x.repr_try_traverse(tracer_fn)
}

/// The header of a python object.
///
/// Not public API; only `pub` because it's specified as
/// `<PyBaseObject as PyPayload>::Super` but it shouldn't be reexported from
/// `crate::object`.
#[doc(hidden)]
pub struct PyObjHeader {
    ref_count: RefCount,
    vtable: &'static PyObjVTable,

    typ: PyAtomicRef<PyType>, // __class__ member
    dict: Option<InstanceDict>,
    weak_list: WeakRefList,
    slots: Box<[PyRwLock<Option<PyObjectRef>>]>,
}

fn traverse_object_head(header: &PyObjHeader, tracer_fn: &mut TraverseFn<'_>) {
    // 1. trace `dict` and `slots` field(`typ` can't trace for it's a AtomicRef while is leaked by design)
    // 2. call vtable's trace function to trace payload
    // self.typ.trace(tracer_fn);
    header.dict.traverse(tracer_fn);
    // weak_list keeps a *pointer* to a struct for maintaince weak ref, so no ownership, no trace
    header.slots.traverse(tracer_fn);
}

/// The layout of an object and its superclass.
///
/// Is marked public, but is not public API.
#[repr(C)]
pub struct PyObjRepr<T: PyPayload> {
    sup: <T::Super as SuperPayload>::Repr,
    payload: T,
}

impl<T: PyPayload> Deref for PyObjRepr<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

impl<T: PyPayload + Default> From<PyObjHeader> for PyObjRepr<T>
where
    <T::Super as SuperPayload>::Repr: From<super::PyObjHeader>,
{
    fn from(header: PyObjHeader) -> Self {
        Self {
            sup: header.into(),
            payload: T::default(),
        }
    }
}

impl<T: Default + PyPayload<Super: SuperPyDefault>> SuperPyDefault for T {
    fn py_from_header(header: PyObjHeader, ctx: &Context) -> Self::Repr {
        PyObjRepr {
            sup: T::Super::py_from_header(header, ctx),
            payload: T::py_default(ctx),
        }
    }
}

impl SuperPyDefault for PyObjHeader {
    fn py_from_header(header: PyObjHeader, _ctx: &Context) -> Self::Repr {
        header
    }
}

impl<T: Default + PyPayload<Super: SuperDefault>> SuperDefault for T {
    fn from_header(header: PyObjHeader) -> Self::Repr {
        PyObjRepr {
            sup: T::Super::from_header(header),
            payload: T::default(),
        }
    }
}

impl SuperDefault for PyObjHeader {
    fn from_header(header: PyObjHeader) -> Self::Repr {
        header
    }
}

impl<T: PyPayload> PyObjRepr<T> {
    fn header(&self) -> &PyObjHeader {
        self.sup.as_header()
    }
}

/// A type that can be the supertype of a `PyPayload`.
///
/// This trait is a bit weird, as `PyPayload::Super` implies that every type
/// has a supertype, but obviously `object` does not. So, `PyBaseObject::Super`
/// is `ObjectHead`, which implements this trait but *not* `PyPayload`, and
/// thus stops that infinite chain.
#[doc(hidden)]
pub trait SuperPayload {
    /// The actual in-memory layout of this type. `PyObjRepr<Self>` for
    /// a `PyPayload`, and `ObjectHead` for `ObjectHead`.
    type Repr: super::core::PayloadRepr;
}

/// `PayloadRepr` represents the actual layout of a `SuperPayload`.
///
/// Mainly exists to unify `PyObjtRepr<T>` and `ObjectHead`.
///
/// # Safety
///
/// The implementing type's layout must have `ObjectHead` at the very start.
pub unsafe trait PayloadRepr {
    /// Access the header of this payload.
    ///
    /// Should always compile down to a no-op, since `ObjectHead` should always
    /// get laid out at the start of any given `PyObjRepr<T>`
    fn as_header(&self) -> &PyObjHeader;

    /// Like `MaybeTraverse::try_traverse`, except it doesn't traverse the
    /// object header - that's done separately. Once dict and weak_ref_list
    /// are no longer stored in the header, this can be simplified.
    fn repr_try_traverse(&self, tracer_fn: &mut TraverseFn<'_>);
}

impl<T: PyPayload> SuperPayload for T {
    type Repr = PyObjRepr<Self>;
}

impl SuperPayload for PyObjHeader {
    type Repr = Self;
}

// SAFETY: layout must start with `ObjectHead`:
// `PyObjRepr` is `repr(C)`, and its first field also implements `PayloadRepr`.
unsafe impl<T: PyPayload> PayloadRepr for PyObjRepr<T> {
    fn as_header(&self) -> &PyObjHeader {
        self.sup.as_header()
    }

    fn repr_try_traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.sup.repr_try_traverse(tracer_fn);
        self.payload.try_traverse(tracer_fn);
    }
}

// SAFETY: `ObjectHead` starts with `ObjectHead`
unsafe impl PayloadRepr for PyObjHeader {
    fn as_header(&self) -> &PyObjHeader {
        self
    }

    fn repr_try_traverse(&self, _tracer_fn: &mut TraverseFn<'_>) {}
}

impl<T: PyPayload + fmt::Debug> fmt::Debug for PyObjRepr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[PyObject {:?}]", &self.payload)
    }
}

unsafe impl<T: PyPayload> Traverse for Py<T> {
    /// DO notice that call `trace` on `Py<T>` means apply `tracer_fn` on `Py<T>`'s children,
    /// not like call `trace` on `PyRef<T>` which apply `tracer_fn` on `PyRef<T>` itself
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        traverse_object_head(self.header(), tracer_fn);
        self.0.repr_try_traverse(tracer_fn)
    }
}

unsafe impl Traverse for PyObject {
    /// DO notice that call `trace` on `PyObject` means apply `tracer_fn` on `PyObject`'s children,
    /// not like call `trace` on `PyObjectRef` which apply `tracer_fn` on `PyObjectRef` itself
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        traverse_object_head(&self.0, tracer_fn);
        if let Some(f) = self.header().vtable.trace {
            unsafe { f(self, tracer_fn) }
        }
    }
}

pub(super) struct WeakRefList {
    inner: OncePtr<PyMutex<WeakListInner>>,
}

impl fmt::Debug for WeakRefList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakRefList").finish_non_exhaustive()
    }
}

struct WeakListInner {
    list: LinkedList<WeakLink, Py<PyWeak>>,
    generic_weakref: Option<NonNull<Py<PyWeak>>>,
    obj: Option<NonNull<PyObject>>,
    // one for each live PyWeak with a reference to this, + 1 for the referent object if it's not dead
    ref_count: usize,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl Send for WeakListInner {}
        unsafe impl Sync for WeakListInner {}
    }
}

impl WeakRefList {
    pub fn new() -> Self {
        WeakRefList {
            inner: OncePtr::new(),
        }
    }

    /// returns None if there have never been any weakrefs in this list
    fn try_lock(&self) -> Option<PyMutexGuard<'_, WeakListInner>> {
        self.inner.get().map(|mu| unsafe { mu.as_ref().lock() })
    }

    fn add(
        &self,
        obj: &PyObject,
        cls: PyTypeRef,
        cls_is_weakref: bool,
        callback: Option<PyObjectRef>,
        dict: Option<PyDictRef>,
    ) -> PyRef<PyWeak> {
        let is_generic = cls_is_weakref && callback.is_none();
        let inner_ptr = self.inner.get_or_init(|| {
            Box::new(PyMutex::new(WeakListInner {
                list: LinkedList::default(),
                generic_weakref: None,
                obj: Some(NonNull::from(obj)),
                ref_count: 1,
            }))
        });
        let mut inner = unsafe { inner_ptr.as_ref().lock() };
        if is_generic {
            if let Some(generic_weakref) = inner.generic_weakref {
                let generic_weakref = unsafe { generic_weakref.as_ref() };
                if generic_weakref.header().ref_count.get() != 0 {
                    return generic_weakref.to_owned();
                }
            }
        }
        let obj = PyWeak {
            pointers: Pointers::new(),
            parent: inner_ptr,
            callback: UnsafeCell::new(callback),
            hash: Radium::new(crate::common::hash::SENTINEL),
        };
        let weak = PyRef::new_ref(obj, cls, dict);
        // SAFETY: we don't actually own the PyObjectWeak's inside `list`, and every time we take
        // one out of the list we immediately wrap it in ManuallyDrop or forget it
        inner.list.push_front(unsafe { ptr::read(&weak) });
        inner.ref_count += 1;
        if is_generic {
            inner.generic_weakref = Some(NonNull::from(&*weak));
        }
        weak
    }

    fn clear(&self) {
        let to_dealloc = {
            let ptr = match self.inner.get() {
                Some(ptr) => ptr,
                None => return,
            };
            let mut inner = unsafe { ptr.as_ref().lock() };
            inner.obj = None;
            // TODO: can be an arrayvec
            let mut v = Vec::with_capacity(16);
            loop {
                let inner2 = &mut *inner;
                let iter = inner2
                    .list
                    .drain_filter(|_| true)
                    .filter_map(|wr| {
                        // we don't have actual ownership of the reference counts in the list.
                        // but, now we do want ownership (and so incref these *while the lock
                        // is held*) to avoid weird things if PyWeakObj::drop happens after
                        // this but before we reach the loop body below
                        let wr = ManuallyDrop::new(wr);

                        if Some(NonNull::from(&**wr)) == inner2.generic_weakref {
                            inner2.generic_weakref = None
                        }

                        // if strong_count == 0 there's some reentrancy going on. we don't
                        // want to call the callback
                        (wr.as_object().strong_count() > 0).then(|| (*wr).clone())
                    })
                    .take(16);
                v.extend(iter);
                if v.is_empty() {
                    break;
                }
                PyMutexGuard::unlocked(&mut inner, || {
                    for wr in v.drain(..) {
                        let cb = unsafe { wr.callback.get().replace(None) };
                        if let Some(cb) = cb {
                            crate::vm::thread::with_vm(&cb, |vm| {
                                // TODO: handle unraisable exception
                                let _ = cb.call((wr.clone(),), vm);
                            });
                        }
                    }
                })
            }
            inner.ref_count -= 1;
            (inner.ref_count == 0).then_some(ptr)
        };
        if let Some(ptr) = to_dealloc {
            unsafe { WeakRefList::dealloc(ptr) }
        }
    }

    fn count(&self) -> usize {
        self.try_lock()
            // we assume the object is still alive (and this is only
            // called from PyObject::weak_count so it should be)
            .map(|inner| inner.ref_count - 1)
            .unwrap_or(0)
    }

    unsafe fn dealloc(ptr: NonNull<PyMutex<WeakListInner>>) {
        drop(unsafe { Box::from_raw(ptr.as_ptr()) });
    }

    fn get_weak_references(&self) -> Vec<PyRef<PyWeak>> {
        let inner = match self.try_lock() {
            Some(inner) => inner,
            None => return vec![],
        };
        let mut v = Vec::with_capacity(inner.ref_count - 1);
        v.extend(inner.iter().map(|wr| wr.to_owned()));
        v
    }
}

impl WeakListInner {
    fn iter(&self) -> impl Iterator<Item = &Py<PyWeak>> {
        self.list
            .iter()
            .filter(|wr| wr.header().ref_count.get() > 0)
    }
}

impl Default for WeakRefList {
    fn default() -> Self {
        Self::new()
    }
}

struct WeakLink;
unsafe impl Link for WeakLink {
    type Handle = PyRef<PyWeak>;

    type Target = Py<PyWeak>;

    #[inline(always)]
    fn as_raw(handle: &PyRef<PyWeak>) -> NonNull<Self::Target> {
        NonNull::from(&**handle)
    }

    #[inline(always)]
    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self::Handle {
        // SAFETY: requirements forwarded from caller
        unsafe { PyRef::from_raw(ptr.as_ptr()) }
    }

    #[inline(always)]
    unsafe fn pointers(target: NonNull<Self::Target>) -> NonNull<Pointers<Self::Target>> {
        // SAFETY: requirements forwarded from caller
        unsafe { NonNull::new_unchecked(&raw mut (*target.as_ptr()).0.payload.pointers) }
    }
}

#[pyclass(name = "weakref", module = false)]
#[derive(Debug)]
pub struct PyWeak {
    pointers: Pointers<Py<PyWeak>>,
    parent: NonNull<PyMutex<WeakListInner>>,
    // this is treated as part of parent's mutex - you must hold that lock to access it
    callback: UnsafeCell<Option<PyObjectRef>>,
    pub(crate) hash: PyAtomic<crate::common::hash::PyHash>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        #[allow(clippy::non_send_fields_in_send_ty)] // false positive?
        unsafe impl Send for PyWeak {}
        unsafe impl Sync for PyWeak {}
    }
}

impl PyWeak {
    pub(crate) fn upgrade(&self) -> Option<PyObjectRef> {
        let guard = unsafe { self.parent.as_ref().lock() };
        let obj_ptr = guard.obj?;
        unsafe {
            if !obj_ptr.as_ref().0.ref_count.safe_inc() {
                return None;
            }
            Some(PyObjectRef::from_raw(obj_ptr))
        }
    }

    pub(crate) fn is_dead(&self) -> bool {
        let guard = unsafe { self.parent.as_ref().lock() };
        guard.obj.is_none()
    }

    fn drop_inner(&self) {
        let dealloc = {
            let mut guard = unsafe { self.parent.as_ref().lock() };
            let offset = std::mem::offset_of!(PyObjRepr<PyWeak>, payload);
            let py_inner = (self as *const Self)
                .cast::<u8>()
                .wrapping_sub(offset)
                .cast::<PyObjRepr<Self>>();
            let node_ptr = unsafe { NonNull::new_unchecked(py_inner as *mut Py<Self>) };
            // the list doesn't have ownership over its PyRef<PyWeak>! we're being dropped
            // right now so that should be obvious!!
            std::mem::forget(unsafe { guard.list.remove(node_ptr) });
            guard.ref_count -= 1;
            if Some(node_ptr) == guard.generic_weakref {
                guard.generic_weakref = None;
            }
            guard.ref_count == 0
        };
        if dealloc {
            unsafe { WeakRefList::dealloc(self.parent) }
        }
    }
}

impl Drop for PyWeak {
    #[inline(always)]
    fn drop(&mut self) {
        // we do NOT have actual exclusive access!
        // no clue if doing this actually reduces chance of UB
        let me: &Self = self;
        me.drop_inner();
    }
}

impl Py<PyWeak> {
    #[inline(always)]
    pub fn upgrade(&self) -> Option<PyObjectRef> {
        PyWeak::upgrade(self)
    }
}

#[derive(Debug)]
pub(super) struct InstanceDict {
    pub(super) d: PyRwLock<PyDictRef>,
}

impl From<PyDictRef> for InstanceDict {
    #[inline(always)]
    fn from(d: PyDictRef) -> Self {
        Self::new(d)
    }
}

impl InstanceDict {
    #[inline]
    pub fn new(d: PyDictRef) -> Self {
        Self {
            d: PyRwLock::new(d),
        }
    }

    #[inline]
    pub fn get(&self) -> PyDictRef {
        self.d.read().clone()
    }

    #[inline]
    pub fn set(&self, d: PyDictRef) {
        self.replace(d);
    }

    #[inline]
    pub fn replace(&self, d: PyDictRef) -> PyDictRef {
        std::mem::replace(&mut self.d.write(), d)
    }
}

impl<T: PyPayload> PyObjRepr<T> {
    fn new(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> Box<Self>
    where
        T::Super: SuperDefault,
    {
        let member_count = typ.slots.member_count;
        let header = PyObjHeader {
            ref_count: RefCount::new(),
            vtable: PyObjVTable::of::<T>(),
            typ: PyAtomicRef::from(typ),
            dict: dict.map(InstanceDict::new),
            weak_list: WeakRefList::new(),
            slots: std::iter::repeat_with(|| PyRwLock::new(None))
                .take(member_count)
                .collect_vec()
                .into_boxed_slice(),
        };
        Box::new(PyObjRepr {
            sup: T::Super::from_header(header),
            payload,
        })
    }
}

#[doc(hidden)]
pub trait Builder<T: SuperPayload> {
    fn build_repr(self, header: PyObjHeader, ctx: &Context) -> T::Repr;
}

pub struct PyDefaultBuilder;

impl<T: SuperPyDefault> Builder<T> for PyDefaultBuilder {
    fn build_repr(self, header: PyObjHeader, ctx: &Context) -> T::Repr {
        T::py_from_header(header, ctx)
    }
}

pub struct PyObjectBuilder<Super: Builder<T::Super>, T: PyPayload> {
    sup: Super,
    payload: T,
}

impl<T: PyPayload<Super: SuperPyDefault>> PyObjectBuilder<PyDefaultBuilder, T> {
    pub fn new(payload: T) -> Self {
        Self {
            sup: PyDefaultBuilder,
            payload,
        }
    }
}

impl<Super, T> Builder<T> for PyObjectBuilder<Super, T>
where
    T: PyPayload,
    Super: Builder<T::Super>,
{
    fn build_repr(self, header: PyObjHeader, ctx: &Context) -> <T as SuperPayload>::Repr {
        let PyObjectBuilder { sup, payload } = self;
        PyObjRepr {
            sup: sup.build_repr(header, ctx),
            payload,
        }
    }
}

impl<Super, T> PyObjectBuilder<Super, T>
where
    T: PyPayload,
    Super: Builder<T::Super>,
{
    pub fn subclass<Sub: PyPayload<Super = T>>(self, payload: Sub) -> PyObjectBuilder<Self, Sub> {
        PyObjectBuilder { sup: self, payload }
    }
    fn _build(self, typ: PyTypeRef, ctx: &Context) -> PyRef<T> {
        let member_count = typ.slots.member_count;
        let dict = if typ.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(ctx.new_dict())
        } else {
            None
        };
        let header = PyObjHeader {
            ref_count: RefCount::new(),
            vtable: PyObjVTable::of::<T>(),
            typ: PyAtomicRef::from(typ),
            dict: dict.map(InstanceDict::new),
            weak_list: WeakRefList::new(),
            slots: std::iter::repeat_with(|| PyRwLock::new(None))
                .take(member_count)
                .collect_vec()
                .into_boxed_slice(),
        };
        PyRef::from_repr(Box::new(self.build_repr(header, ctx)))
    }

    pub fn build(self, ctx: &Context) -> PyRef<T> {
        self._build(T::class(&ctx).to_owned(), ctx)
    }

    pub fn build_exact(self, ctx: &Context) -> PyRefExact<T> {
        let obj = self.build(ctx);
        // build() provides T::class as the type, so it's always exact
        unsafe { PyRefExact::new_unchecked(obj) }
    }

    #[inline]
    pub fn build_with_type(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        let exact_class = T::class(&vm.ctx);
        if cls.fast_issubclass(exact_class) {
            // TODO: not checking this allows for unsoundness at the moment
            // assert!(cls.type_id == exact_class.type_id);
            Ok(self._build(cls, &vm.ctx))
        } else {
            #[cold]
            #[inline(never)]
            fn _build_with_type_error(
                vm: &VirtualMachine,
                cls: PyTypeRef,
                exact_class: &Py<PyType>,
            ) -> PyBaseExceptionRef {
                vm.new_type_error(format!(
                    "'{}' is not a subtype of '{}'",
                    &cls.name(),
                    exact_class.name()
                ))
            }

            Err(_build_with_type_error(vm, cls, exact_class))
        }
    }
}

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
#[repr(transparent)]
pub struct PyObjectRef {
    ptr: NonNull<PyObject>,
}

impl Clone for PyObjectRef {
    #[inline(always)]
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl Send for PyObjectRef {}
        unsafe impl Sync for PyObjectRef {}
    }
}

#[repr(transparent)]
pub struct PyObject(PyObjHeader);

impl Deref for PyObjectRef {
    type Target = PyObject;
    #[inline(always)]
    fn deref(&self) -> &PyObject {
        unsafe { self.ptr.as_ref() }
    }
}

impl ToOwned for PyObject {
    type Owned = PyObjectRef;

    #[inline(always)]
    fn to_owned(&self) -> Self::Owned {
        self.0.ref_count.inc();
        PyObjectRef {
            ptr: NonNull::from(self),
        }
    }
}

impl PyObjectRef {
    #[inline(always)]
    pub fn into_raw(self) -> NonNull<PyObject> {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// # Safety
    /// The raw pointer must have been previously returned from a call to
    /// [`PyObjectRef::into_raw`]. The user is responsible for ensuring that the inner data is not
    /// dropped more than once due to mishandling the reference count by calling this function
    /// too many times.
    #[inline(always)]
    pub unsafe fn from_raw(ptr: NonNull<PyObject>) -> Self {
        Self { ptr }
    }

    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    #[inline(always)]
    pub fn downcast<T: PyPayload>(self) -> Result<PyRef<T>, Self> {
        if self.payload_is::<T>() {
            Ok(unsafe { self.downcast_unchecked() })
        } else {
            Err(self)
        }
    }

    #[inline(always)]
    pub fn downcast_ref<T: PyPayload>(&self) -> Option<&Py<T>> {
        if self.payload_is::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { &*(self as *const PyObjectRef as *const PyRef<T>) })
        } else {
            None
        }
    }

    /// Force to downcast this reference to a subclass.
    ///
    /// # Safety
    /// T must be the exact payload type
    #[inline(always)]
    pub unsafe fn downcast_unchecked<T: PyPayload>(self) -> PyRef<T> {
        // PyRef::from_obj_unchecked(self)
        // manual impl to avoid assertion
        let obj = ManuallyDrop::new(self);
        PyRef {
            ptr: obj.ptr.cast(),
        }
    }

    // ideally we'd be able to define these in pyobject.rs, but method visibility rules are weird

    /// Attempt to downcast this reference to the specific class that is associated `T`.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    #[inline]
    pub fn downcast_exact<T: PyPayload + crate::PyPayload>(
        self,
        vm: &VirtualMachine,
    ) -> Result<PyRefExact<T>, Self> {
        if self.class().is(T::class(&vm.ctx)) {
            // TODO: is this always true?
            assert!(
                self.payload_is::<T>(),
                "obj.__class__ is T::class() but payload is not T"
            );
            // SAFETY: just asserted that payload_is::<T>()
            Ok(unsafe { PyRefExact::new_unchecked(PyRef::from_obj_unchecked(self)) })
        } else {
            Err(self)
        }
    }
}

impl PyObject {
    #[inline(always)]
    fn header(&self) -> &PyObjHeader {
        &self.0
    }

    #[inline(always)]
    fn weak_ref_list(&self) -> Option<&WeakRefList> {
        Some(&self.0.weak_list)
    }

    pub(crate) fn downgrade_with_weakref_typ_opt(
        &self,
        callback: Option<PyObjectRef>,
        // a reference to weakref_type **specifically**
        typ: PyTypeRef,
    ) -> Option<PyRef<PyWeak>> {
        self.weak_ref_list()
            .map(|wrl| wrl.add(self, typ, true, callback, None))
    }

    pub(crate) fn downgrade_with_typ(
        &self,
        callback: Option<PyObjectRef>,
        typ: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyWeak>> {
        let dict = if typ
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        let cls_is_weakref = typ.is(vm.ctx.types.weakref_type);
        let wrl = self.weak_ref_list().ok_or_else(|| {
            vm.new_type_error(format!(
                "cannot create weak reference to '{}' object",
                self.class().name()
            ))
        })?;
        Ok(wrl.add(self, typ, cls_is_weakref, callback, dict))
    }

    pub fn downgrade(
        &self,
        callback: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyWeak>> {
        self.downgrade_with_typ(callback, vm.ctx.types.weakref_type.to_owned(), vm)
    }

    pub fn get_weak_references(&self) -> Option<Vec<PyRef<PyWeak>>> {
        self.weak_ref_list().map(|wrl| wrl.get_weak_references())
    }

    #[inline(always)]
    pub fn payload_is<T: PyPayload>(&self) -> bool {
        self.class()
            .iter_base_chain()
            .any(|t| t.type_id == TypeId::of::<T>())
    }

    /// Force to return payload as T.
    ///
    /// # Safety
    /// The actual payload type must be T.
    #[inline(always)]
    pub unsafe fn payload_unchecked<T: PyPayload>(&self) -> &T {
        // we cast to a PyInner<T> first because we don't know T's exact offset because of
        // varying alignment, but once we get a PyInner<T> the compiler can get it for us
        let inner = unsafe { &*(&self.0 as *const PyObjHeader as *const PyObjRepr<T>) };
        &inner.payload
    }

    #[inline(always)]
    pub fn payload<T: PyPayload>(&self) -> Option<&T> {
        if self.payload_is::<T>() {
            Some(unsafe { self.payload_unchecked() })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn class(&self) -> &Py<PyType> {
        self.0.typ.deref()
    }

    pub fn set_class(&self, typ: PyTypeRef, vm: &VirtualMachine) -> Result<(), PyTypeRef> {
        if self.class().type_id != typ.type_id {
            return Err(typ);
        }
        self.0.typ.swap_to_temporary_refs(typ, vm);
        Ok(())
    }

    #[inline(always)]
    pub fn payload_if_exact<T: PyPayload + crate::PyPayload>(
        &self,
        vm: &VirtualMachine,
    ) -> Option<&T> {
        if self.class().is(T::class(&vm.ctx)) {
            self.payload()
        } else {
            None
        }
    }

    #[inline(always)]
    fn instance_dict(&self) -> Option<&InstanceDict> {
        self.0.dict.as_ref()
    }

    #[inline(always)]
    pub fn dict(&self) -> Option<PyDictRef> {
        self.instance_dict().map(|d| d.get())
    }

    /// Set the dict field. Returns `Err(dict)` if this object does not have a dict field
    /// in the first place.
    pub fn set_dict(&self, dict: PyDictRef) -> Result<(), PyDictRef> {
        match self.instance_dict() {
            Some(d) => {
                d.set(dict);
                Ok(())
            }
            None => Err(dict),
        }
    }

    #[inline(always)]
    pub fn payload_if_subclass<T: crate::PyPayload>(&self, vm: &VirtualMachine) -> Option<&T> {
        if self.class().fast_issubclass(T::class(&vm.ctx)) {
            self.payload()
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn downcast_ref<T: PyPayload>(&self) -> Option<&Py<T>> {
        if self.payload_is::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { self.downcast_unchecked_ref::<T>() })
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn downcast_ref_if_exact<T: PyPayload + crate::PyPayload>(
        &self,
        vm: &VirtualMachine,
    ) -> Option<&Py<T>> {
        self.class()
            .is(T::class(&vm.ctx))
            .then(|| unsafe { self.downcast_unchecked_ref::<T>() })
    }

    /// # Safety
    /// T must be the exact payload type
    #[inline(always)]
    pub unsafe fn downcast_unchecked_ref<T: PyPayload>(&self) -> &Py<T> {
        debug_assert!(self.payload_is::<T>());
        // SAFETY: requirements forwarded from caller. this is possibly a bit
        // sketchy because we're widening the range of the `&` reference from
        // just the `PyObjHeader` to the entire object, but under tree borrows
        // it's fine according to miri, since `RefCount` isn't `Freeze`.
        unsafe { &*(self as *const PyObject as *const Py<T>) }
    }

    #[inline(always)]
    pub fn strong_count(&self) -> usize {
        self.0.ref_count.get()
    }

    #[inline]
    pub fn weak_count(&self) -> Option<usize> {
        self.weak_ref_list().map(|wrl| wrl.count())
    }

    #[inline(always)]
    pub fn as_raw(&self) -> *const PyObject {
        self
    }

    #[inline(always)] // the outer function is never inlined
    fn drop_slow_inner(&self) -> Result<(), ()> {
        // __del__ is mostly not implemented
        #[inline(never)]
        #[cold]
        fn call_slot_del(
            zelf: &PyObject,
            slot_del: fn(&PyObject, &VirtualMachine) -> PyResult<()>,
        ) -> Result<(), ()> {
            let ret = crate::vm::thread::with_vm(zelf, |vm| {
                zelf.0.ref_count.inc();
                if let Err(e) = slot_del(zelf, vm) {
                    let del_method = zelf.get_class_attr(identifier!(vm, __del__)).unwrap();
                    vm.run_unraisable(e, None, del_method);
                }
                zelf.0.ref_count.dec()
            });
            match ret {
                // the decref right above set ref_count back to 0
                Some(true) => Ok(()),
                // we've been resurrected by __del__
                Some(false) => Err(()),
                None => {
                    warn!("couldn't run __del__ method for object");
                    Ok(())
                }
            }
        }

        // CPython-compatible drop implementation
        let del = self.class().mro_find_map(|cls| cls.slots.del.load());
        if let Some(slot_del) = del {
            call_slot_del(self, slot_del)?;
        }
        if let Some(wrl) = self.weak_ref_list() {
            wrl.clear();
        }

        Ok(())
    }

    /// Can only be called when ref_count has dropped to zero. `ptr` must be valid
    #[inline(never)]
    unsafe fn drop_slow(ptr: NonNull<PyObject>) {
        if let Err(()) = unsafe { ptr.as_ref().drop_slow_inner() } {
            // abort drop for whatever reason
            return;
        }
        let drop_dealloc = unsafe { ptr.as_ref().0.vtable.drop_dealloc };
        // call drop only when there are no references in scope - stacked borrows stuff
        unsafe { drop_dealloc(ptr.as_ptr()) }
    }

    /// # Safety
    /// This call will make the object live forever.
    pub(crate) unsafe fn mark_intern(&self) {
        self.0.ref_count.leak();
    }

    pub(crate) fn is_interned(&self) -> bool {
        self.0.ref_count.is_leaked()
    }

    pub(crate) fn get_slot(&self, offset: usize) -> Option<PyObjectRef> {
        self.0.slots[offset].read().clone()
    }

    pub(crate) fn set_slot(&self, offset: usize, value: Option<PyObjectRef>) {
        *self.0.slots[offset].write() = value;
    }
}

impl Borrow<PyObject> for PyObjectRef {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self
    }
}

impl AsRef<PyObject> for PyObjectRef {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self
    }
}

impl AsRef<PyObject> for PyObject {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self
    }
}

impl<'a, T: PyPayload> From<&'a Py<T>> for &'a PyObject {
    #[inline(always)]
    fn from(py_ref: &'a Py<T>) -> Self {
        py_ref.as_object()
    }
}

impl Drop for PyObjectRef {
    #[inline]
    fn drop(&mut self) {
        if self.0.ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr) }
        }
    }
}

impl fmt::Debug for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the vtable contains functions that accept payload types that always match up
        // with the payload of the object
        unsafe { (self.0.vtable.debug)(self, f) }
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_object().fmt(f)
    }
}

#[repr(transparent)]
pub struct Py<T: PyPayload>(PyObjRepr<T>);

impl<T: PyPayload> Py<T> {
    #[inline(always)]
    fn header(&self) -> &PyObjHeader {
        self.0.header()
    }

    #[allow(private_bounds)]
    pub fn super_(&self) -> &Py<T::Super>
    where
        T::Super: PyPayload,
        // this should really be superfluous - T::Super: PyPayload implies it,
        // but the current trait solver isn't smart enough
        T: PyPayload<Super: SuperPayload<Repr = PyObjRepr<T::Super>>>,
    {
        let sup: &PyObjRepr<T::Super> = &self.0.sup;
        // SAFETY:
        unsafe { &*(sup as *const PyObjRepr<T::Super> as *const Py<T::Super>) }
    }

    pub fn downgrade(
        &self,
        callback: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyWeakRef<T>> {
        Ok(PyWeakRef {
            weak: self.as_object().downgrade(callback, vm)?,
            _marker: PhantomData,
        })
    }
}

impl<T: PyPayload> ToOwned for Py<T> {
    type Owned = PyRef<T>;

    #[inline(always)]
    fn to_owned(&self) -> Self::Owned {
        self.header().ref_count.inc();
        PyRef {
            ptr: NonNull::from(self),
        }
    }
}

impl<T: PyPayload> Deref for Py<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0.payload
    }
}

impl<T: PyPayload> Borrow<PyObject> for Py<T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        unsafe { &*(&self.0 as *const PyObjRepr<T> as *const PyObject) }
    }
}

impl<T> std::hash::Hash for Py<T>
where
    T: std::hash::Hash + PyPayload,
{
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T> PartialEq for Py<T>
where
    T: PartialEq + PyPayload,
{
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<T> Eq for Py<T> where T: Eq + PyPayload {}

impl<T> AsRef<PyObject> for Py<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.borrow()
    }
}

impl<T: PyPayload> fmt::Debug for Py<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// A reference to a Python object.
///
/// Note that a `PyRef<T>` can only deref to a shared / immutable reference.
/// It is the payload type's responsibility to handle (possibly concurrent)
/// mutability with locks or concurrent data structures if required.
///
/// A `PyRef<T>` can be directly returned from a built-in function to handle
/// situations (such as when implementing in-place methods such as `__iadd__`)
/// where a reference to the same object must be returned.
#[repr(transparent)]
pub struct PyRef<T: PyPayload> {
    ptr: NonNull<Py<T>>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl<T: PyPayload> Send for PyRef<T> {}
        unsafe impl<T: PyPayload> Sync for PyRef<T> {}
    }
}

impl<T: PyPayload> fmt::Debug for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: PyPayload> Drop for PyRef<T> {
    #[inline]
    fn drop(&mut self) {
        if self.header().ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr.cast::<PyObject>()) }
        }
    }
}

impl<T: PyPayload> Clone for PyRef<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

impl<T: PyPayload> PyRef<T> {
    #[inline(always)]
    pub(crate) unsafe fn from_raw(raw: *const Py<T>) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(raw as *mut _) },
        }
    }

    /// Safety: payload type of `obj` must be `T`
    #[inline(always)]
    unsafe fn from_obj_unchecked(obj: PyObjectRef) -> Self {
        debug_assert!(obj.payload_is::<T>());
        let obj = ManuallyDrop::new(obj);
        Self {
            ptr: obj.ptr.cast(),
        }
    }

    #[inline(always)]
    pub fn new_ref(payload: T, typ: crate::builtins::PyTypeRef, dict: Option<PyDictRef>) -> Self
    where
        T::Super: SuperDefault,
    {
        Self::from_repr(PyObjRepr::new(payload, typ, dict))
    }

    #[inline(always)]
    fn from_repr(repr: Box<PyObjRepr<T>>) -> Self {
        let inner = Box::into_raw(repr);
        Self {
            ptr: unsafe { NonNull::new_unchecked(inner.cast::<Py<T>>()) },
        }
    }

    pub fn leak(pyref: Self) -> &'static Py<T> {
        let ptr = pyref.ptr;
        std::mem::forget(pyref);
        unsafe { ptr.as_ref() }
    }
}

impl<T> Borrow<PyObject> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        (**self).as_object()
    }
}

impl<T> AsRef<PyObject> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        self.borrow()
    }
}

impl<T> From<PyRef<T>> for PyObjectRef
where
    T: PyPayload,
{
    #[inline]
    fn from(value: PyRef<T>) -> Self {
        let me = ManuallyDrop::new(value);
        PyObjectRef { ptr: me.ptr.cast() }
    }
}

impl<T> Borrow<Py<T>> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn borrow(&self) -> &Py<T> {
        self
    }
}

impl<T> AsRef<Py<T>> for PyRef<T>
where
    T: PyPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &Py<T> {
        self
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyPayload,
{
    type Target = Py<T>;

    #[inline(always)]
    fn deref(&self) -> &Py<T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> std::hash::Hash for PyRef<T>
where
    T: std::hash::Hash + PyPayload,
{
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl<T> PartialEq for PyRef<T>
where
    T: PartialEq + PyPayload,
{
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<T> Eq for PyRef<T> where T: Eq + PyPayload {}

#[repr(transparent)]
pub struct PyWeakRef<T: PyPayload> {
    weak: PyRef<PyWeak>,
    _marker: PhantomData<T>,
}

impl<T: PyPayload> PyWeakRef<T> {
    pub fn upgrade(&self) -> Option<PyRef<T>> {
        self.weak
            .upgrade()
            // SAFETY: PyWeakRef<T> was always created from a PyRef<T>, so the object is T
            .map(|obj| unsafe { PyRef::from_obj_unchecked(obj) })
    }
}

/// Partially initialize a struct, ensuring that all fields are
/// either given values or explicitly left uninitialized
macro_rules! partially_init {
    (
        $ty:path {$($($init_field:ident).+: $init_value:expr),*$(,)?},
        Uninit { $($uninit_field:ident),*$(,)? }$(,)?
    ) => {{
        // FIXME: figure out a way to check that all the fields have been mentioned
        let mut m = ::std::mem::MaybeUninit::<$ty>::uninit();
        #[allow(unused_unsafe)]
        unsafe {
            $(::std::ptr::write(&raw mut (*m.as_mut_ptr()).$($init_field).+, $init_value);)*
        }
        m
    }};
}

pub(crate) fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef, PyTypeRef) {
    use crate::{builtins::object, class::PyClassImpl};
    use std::mem::MaybeUninit;

    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        // We cast between these 2 types, so make sure (at compile time) that there's no change in
        // layout when we wrap PyInner<PyTypeObj> in MaybeUninit<>
        static_assertions::assert_eq_size!(MaybeUninit<PyObjRepr<PyType>>, PyObjRepr<PyType>);
        static_assertions::assert_eq_align!(MaybeUninit<PyObjRepr<PyType>>, PyObjRepr<PyType>);

        let type_payload = PyType {
            base: None,
            type_id: TypeId::of::<PyType>(),
            bases: PyRwLock::default(),
            mro: PyRwLock::default(),
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(Default::default()),
            slots: PyType::make_slots(),
            heaptype_ext: None,
        };
        let object_payload = PyType {
            base: None,
            type_id: TypeId::of::<PyBaseObject>(),
            bases: PyRwLock::default(),
            mro: PyRwLock::default(),
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(Default::default()),
            slots: object::PyBaseObject::make_slots(),
            heaptype_ext: None,
        };
        let type_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyObjRepr::<PyType> {
                sup.payload: PyBaseObject,
                sup.sup.ref_count: RefCount::new(),
                sup.sup.vtable: PyObjVTable::of::<PyType>(),
                sup.sup.dict: None,
                sup.sup.weak_list: WeakRefList::new(),
                payload: type_payload,
                sup.sup.slots: Box::new([]),
            },
            Uninit { typ }
        )));
        let object_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyObjRepr::<PyType> {
                sup.payload: PyBaseObject,
                sup.sup.ref_count: RefCount::new(),
                sup.sup.vtable: PyObjVTable::of::<PyType>(),
                sup.sup.dict: None,
                sup.sup.weak_list: WeakRefList::new(),
                payload: object_payload,
                sup.sup.slots: Box::new([]),
            },
            Uninit { typ },
        )));

        let object_type_ptr = object_type_ptr as *mut PyObjRepr<PyType>;
        let type_type_ptr = type_type_ptr as *mut PyObjRepr<PyType>;

        unsafe {
            (*type_type_ptr).sup.sup.ref_count.inc();
            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());
            ptr::write(
                &raw mut (*object_type_ptr).sup.sup.typ,
                PyAtomicRef::from(type_type),
            );
            (*type_type_ptr).sup.sup.ref_count.inc();
            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());
            ptr::write(
                &raw mut (*type_type_ptr).sup.sup.typ,
                PyAtomicRef::from(type_type),
            );

            let object_type = PyTypeRef::from_raw(object_type_ptr.cast());

            (*type_type_ptr).payload.mro = PyRwLock::new(vec![object_type.clone()]);
            (*type_type_ptr).payload.bases = PyRwLock::new(vec![object_type.clone()]);
            (*type_type_ptr).payload.base = Some(object_type.clone());

            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());

            (type_type, object_type)
        }
    };

    let weakref_type = PyType {
        base: Some(object_type.clone()),
        type_id: TypeId::of::<PyWeak>(),
        bases: PyRwLock::new(vec![object_type.clone()]),
        mro: PyRwLock::new(vec![object_type.clone()]),
        subclasses: PyRwLock::default(),
        attributes: PyRwLock::default(),
        slots: PyWeak::make_slots(),
        heaptype_ext: None,
    };
    let weakref_type = PyRef::new_ref(weakref_type, type_type.clone(), None);

    object_type.subclasses.write().push(
        type_type
            .as_object()
            .downgrade_with_weakref_typ_opt(None, weakref_type.clone())
            .unwrap(),
    );

    object_type.subclasses.write().push(
        weakref_type
            .as_object()
            .downgrade_with_weakref_typ_opt(None, weakref_type.clone())
            .unwrap(),
    );

    (type_type, object_type, weakref_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miri_test_type_initialization() {
        let _ = init_type_hierarchy();
    }

    #[test]
    fn miri_test_drop() {
        //cspell:ignore dfghjkl
        let ctx = crate::Context::genesis();
        let obj = ctx.new_bytes(b"dfghjkl".to_vec());
        drop(obj);
    }
}

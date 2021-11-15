use crate::common::atomic::{OncePtr, PyAtomic, Radium};
use crate::common::linked_list::{Link, LinkedList, Pointers};
use crate::common::lock::{PyMutex, PyMutexGuard, PyRwLock};
use crate::common::refcount::RefCount;
use crate::{
    builtins::{PyBaseExceptionRef, PyDictRef, PyTypeRef},
    IdProtocol, PyObjectPayload, PyResult, TypeProtocol, VirtualMachine,
};
use std::any::TypeId;
use std::borrow::Borrow;
use std::cell::UnsafeCell;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ptr::{self, NonNull};

// so, PyObjectRef is basically equivalent to `PyRc<PyInner<dyn PyObjectPayload>>`, except it's
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
// (like `dyn PyObjectPayload`) we don't *know* how much padding there is between the `payload`
// field and the previous field. So, Rust has to consult the vtable to know the exact offset of
// `payload` in `PyInner<dyn PyObjectPayload>`, which has a huge performance impact when *every
// single payload access* requires a vtable lookup. Thankfully, we're able to avoid that because of
// the way we use PyObjectRef, in that whenever we want to access the payload we (almost) always
// access it from a generic function. So, rather than doing
//
// - check vtable for payload offset
// - get offset in PyInner struct
// - call as_any() method of PyObjectPayload
// - call downcast_ref() method of Any
// we can just do
// - check vtable that typeid matches
// - pointer cast directly to *const PyInner<T>
//
// and at that point the compiler can know the offset of `payload` for us because **we've given it a
// concrete type to work with before we ever access the `payload` field**

/// A type to just represent "we've erased the type of this object, cast it before you use it"
struct Erased;

struct PyObjVTable {
    drop_dealloc: unsafe fn(*mut PyObject),
    debug: unsafe fn(&PyObject, &mut fmt::Formatter) -> fmt::Result,
}
unsafe fn drop_dealloc_obj<T: PyObjectPayload>(x: *mut PyObject) {
    Box::from_raw(x as *mut PyInner<T>);
}
unsafe fn debug_obj<T: PyObjectPayload>(x: &PyObject, f: &mut fmt::Formatter) -> fmt::Result {
    let x = &*(x as *const PyObject as *const PyInner<T>);
    fmt::Debug::fmt(x, f)
}

impl PyObjVTable {
    pub fn of<T: PyObjectPayload>() -> &'static Self {
        struct Helper<T: PyObjectPayload>(PhantomData<T>);
        trait VtableHelper {
            const VTABLE: PyObjVTable;
        }
        impl<T: PyObjectPayload> VtableHelper for Helper<T> {
            const VTABLE: PyObjVTable = PyObjVTable {
                drop_dealloc: drop_dealloc_obj::<T>,
                debug: debug_obj::<T>,
            };
        }
        &Helper::<T>::VTABLE
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
#[repr(C)]
struct PyInner<T> {
    ref_count: RefCount,
    // TODO: move typeid into vtable once TypeId::of is const
    typeid: TypeId,
    vtable: &'static PyObjVTable,

    typ: PyRwLock<PyTypeRef>, // __class__ member
    dict: Option<InstanceDict>,
    weak_list: WeakRefList,

    payload: T,
}

impl<T: fmt::Debug> fmt::Debug for PyInner<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObject {:?}]", &self.payload)
    }
}

struct WeakRefList {
    inner: OncePtr<PyMutex<WeakListInner>>,
}

impl fmt::Debug for WeakRefList {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("WeakRefList").finish_non_exhaustive()
    }
}

struct WeakListInner {
    list: LinkedList<WeakLink, PyObjectView<PyWeak>>,
    generic_weakref: Option<NonNull<PyObjectView<PyWeak>>>,
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
    ) -> PyObjectWeak {
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
                if generic_weakref.0.ref_count.get() != 0 {
                    return PyObjectWeak {
                        weak: generic_weakref.to_owned(),
                    };
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
        // SAFETY: we don't actually own the PyObjectWeaks inside `list`, and every time we take
        // one out of the list we immediately wrap it in ManuallyDrop or forget it
        inner.list.push_front(unsafe { ptr::read(&weak) });
        inner.ref_count += 1;
        if is_generic {
            inner.generic_weakref = Some(NonNull::from(&*weak));
        }
        PyObjectWeak { weak }
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
                                let _ = vm.invoke(&cb, (wr.clone(),));
                            });
                        }
                    }
                })
            }
            inner.ref_count -= 1;
            (inner.ref_count == 0).then(|| ptr)
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
        Box::from_raw(ptr.as_ptr());
    }

    fn get_weak_references(&self) -> Vec<PyObjectWeak> {
        let inner = match self.try_lock() {
            Some(inner) => inner,
            None => return vec![],
        };
        let mut v = Vec::with_capacity(inner.ref_count - 1);
        v.extend(inner.iter().map(|wr| PyObjectWeak {
            weak: wr.to_owned(),
        }));
        v
    }
}

impl WeakListInner {
    fn iter(&self) -> impl Iterator<Item = &PyObjectView<PyWeak>> {
        self.list.iter().filter(|wr| wr.0.ref_count.get() > 0)
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

    type Target = PyObjectView<PyWeak>;

    fn as_raw(handle: &PyRef<PyWeak>) -> NonNull<Self::Target> {
        NonNull::from(&**handle)
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self::Handle {
        PyRef::from_raw(ptr.as_ptr())
    }

    unsafe fn pointers(target: NonNull<Self::Target>) -> NonNull<Pointers<Self::Target>> {
        NonNull::new_unchecked(ptr::addr_of_mut!((*target.as_ptr()).0.payload.pointers))
    }
}

#[pyclass(name = "weakref", module = false)]
#[derive(Debug)]
pub struct PyWeak {
    pointers: Pointers<PyObjectView<PyWeak>>,
    parent: NonNull<PyMutex<WeakListInner>>,
    // this is treated as part of parent's mutex - you must hold that lock to access it
    callback: UnsafeCell<Option<PyObjectRef>>,
    pub(crate) hash: PyAtomic<crate::common::hash::PyHash>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
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
            Some(PyObjectRef::from_raw(obj_ptr.as_ptr()))
        }
    }

    pub(crate) fn is_dead(&self) -> bool {
        let guard = unsafe { self.parent.as_ref().lock() };
        guard.obj.is_none()
    }

    #[inline(always)]
    fn drop_inner(&self) {
        let dealloc = {
            let mut guard = unsafe { self.parent.as_ref().lock() };
            let offset = memoffset::offset_of!(PyInner<PyWeak>, payload);
            let pyinner = (self as *const Self as usize - offset) as *const PyInner<Self>;
            let node_ptr = unsafe { NonNull::new_unchecked(pyinner as *mut PyObjectView<Self>) };
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
    fn drop(&mut self) {
        // we do NOT have actual exclusive access!
        // no clue if doing this actually reduces chance of UB
        let me: &Self = self;
        me.drop_inner();
    }
}

#[derive(Debug)]
struct InstanceDict {
    d: PyRwLock<PyDictRef>,
}

impl From<PyDictRef> for InstanceDict {
    #[inline]
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

impl<T: PyObjectPayload> PyInner<T> {
    fn new(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> Box<Self> {
        Box::new(PyInner {
            ref_count: RefCount::new(),
            typeid: TypeId::of::<T>(),
            vtable: PyObjVTable::of::<T>(),
            typ: PyRwLock::new(typ),
            dict: dict.map(InstanceDict::new),
            weak_list: WeakRefList::new(),
            payload,
        })
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

#[derive(Clone)]
#[repr(transparent)]
pub struct PyObjectWeak {
    weak: PyRef<PyWeak>,
}

#[repr(transparent)]
pub struct PyObject(PyInner<Erased>);

impl Deref for PyObjectRef {
    type Target = PyObject;
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

pub trait PyObjectWrap
where
    Self: AsRef<PyObject>,
{
    #[inline(always)]
    fn as_object(&self) -> &PyObject {
        self.as_ref()
    }

    fn into_object(self) -> PyObjectRef;
}

impl PyObjectRef {
    pub fn into_raw(self) -> *const PyObject {
        let ptr = self.as_raw();
        std::mem::forget(self);
        ptr
    }

    /// # Safety
    /// The raw pointer must have been previously returned from a call to
    /// [`PyObjectRef::into_raw`]. The user is responsible for ensuring that the inner data is not
    /// dropped more than once due to mishandling the reference count by calling this function
    /// too many times.
    pub unsafe fn from_raw(ptr: *const PyObject) -> Self {
        Self {
            ptr: NonNull::new_unchecked(ptr as *mut PyObject),
        }
    }

    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    pub fn downcast<T: PyObjectPayload>(self) -> Result<PyRef<T>, Self> {
        if self.payload_is::<T>() {
            Ok(unsafe { PyRef::from_obj_unchecked(self) })
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<T: PyObjectPayload>(&self) -> Option<&PyObjectView<T>> {
        if self.payload_is::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { &*(self as *const PyObjectRef as *const PyRef<T>) })
        } else {
            None
        }
    }

    /// # Safety
    /// T must be the exact payload type
    pub unsafe fn downcast_unchecked<T: PyObjectPayload>(self) -> PyRef<T> {
        PyRef::from_obj_unchecked(self)
    }

    /// # Safety
    /// T must be the exact payload type
    pub unsafe fn downcast_unchecked_ref<T: PyObjectPayload>(&self) -> &crate::PyObjectView<T> {
        debug_assert!(self.payload_is::<T>());
        &*(self as *const PyObjectRef as *const PyRef<T>)
    }

    // ideally we'd be able to define these in pyobject.rs, but method visibility rules are weird

    /// Attempt to downcast this reference to the specific class that is associated `T`.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    pub fn downcast_exact<T: PyObjectPayload + crate::PyValue>(
        self,
        vm: &VirtualMachine,
    ) -> Result<PyRef<T>, Self> {
        if self.class().is(T::class(vm)) {
            // TODO: is this always true?
            assert!(
                self.payload_is::<T>(),
                "obj.__class__ is T::class() but payload is not T"
            );
            // SAFETY: just asserted that payload_is::<T>()
            Ok(unsafe { PyRef::from_obj_unchecked(self) })
        } else {
            Err(self)
        }
    }
}

impl PyObject {
    #[inline]
    fn weak_ref_list(&self) -> Option<&WeakRefList> {
        Some(&self.0.weak_list)
    }

    pub(crate) fn downgrade_with_weakref_typ_opt(
        &self,
        callback: Option<PyObjectRef>,
        // a reference to weakref_type **specifically**
        typ: PyTypeRef,
    ) -> Option<PyObjectWeak> {
        self.weak_ref_list()
            .map(|wrl| wrl.add(self, typ, true, callback, None))
    }

    pub(crate) fn downgrade_with_typ(
        &self,
        callback: Option<PyObjectRef>,
        typ: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectWeak> {
        let dict = if typ
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        let cls_is_weakref = typ.is(&vm.ctx.types.weakref_type);
        self.weak_ref_list()
            .map(|wrl| wrl.add(self, typ, cls_is_weakref, callback, dict))
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "cannot create weak reference to '{}' object",
                    self.class().name()
                ))
            })
    }

    pub fn downgrade(
        &self,
        callback: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectWeak> {
        self.downgrade_with_typ(callback, vm.ctx.types.weakref_type.clone(), vm)
    }

    pub fn get_weak_references(&self) -> Option<Vec<PyObjectWeak>> {
        self.weak_ref_list().map(|wrl| wrl.get_weak_references())
    }

    pub fn payload_is<T: PyObjectPayload>(&self) -> bool {
        self.0.typeid == TypeId::of::<T>()
    }

    pub fn payload<T: PyObjectPayload>(&self) -> Option<&T> {
        if self.payload_is::<T>() {
            // we cast to a PyInner<T> first because we don't know T's exact offset because of
            // varying alignment, but once we get a PyInner<T> the compiler can get it for us
            let inner = unsafe { &*(&self.0 as *const PyInner<Erased> as *const PyInner<T>) };
            Some(&inner.payload)
        } else {
            None
        }
    }

    pub(crate) fn class_lock(&self) -> &PyRwLock<PyTypeRef> {
        &self.0.typ
    }

    #[inline]
    pub fn payload_if_exact<T: PyObjectPayload + crate::PyValue>(
        &self,
        vm: &VirtualMachine,
    ) -> Option<&T> {
        if self.class().is(T::class(vm)) {
            self.payload()
        } else {
            None
        }
    }

    #[inline]
    fn instance_dict(&self) -> Option<&InstanceDict> {
        self.0.dict.as_ref()
    }

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

    #[inline]
    pub fn payload_if_subclass<T: crate::PyValue>(&self, vm: &VirtualMachine) -> Option<&T> {
        if self.class().issubclass(T::class(vm)) {
            self.payload()
        } else {
            None
        }
    }

    pub fn downcast_ref<T: PyObjectPayload>(&self) -> Option<&PyObjectView<T>> {
        if self.payload_is::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { &*(self as *const PyObject as *const PyObjectView<T>) })
        } else {
            None
        }
    }

    /// # Safety
    /// T must be the exact payload type
    pub unsafe fn downcast_unchecked_ref<T: PyObjectPayload>(&self) -> &crate::PyObjectView<T> {
        debug_assert!(self.payload_is::<T>());
        &*(self as *const PyObject as *const PyObjectView<T>)
    }

    #[inline]
    pub fn strong_count(&self) -> usize {
        self.0.ref_count.get()
    }

    #[inline]
    pub fn weak_count(&self) -> Option<usize> {
        self.weak_ref_list().map(|wrl| wrl.count())
    }

    #[inline]
    pub fn as_raw(&self) -> *const PyObject {
        self
    }

    #[inline]
    fn drop_slow_inner(&self) -> Result<(), ()> {
        // CPython-compatible drop implementation
        if let Some(slot_del) = self.class().mro_find_map(|cls| cls.slots.del.load()) {
            let ret = crate::vm::thread::with_vm(self, |vm| {
                self.0.ref_count.inc();
                if let Err(e) = slot_del(self, vm) {
                    print_del_error(e, self, vm);
                }
                self.0.ref_count.dec()
            });
            match ret {
                // the decref right above set ref_count back to 0
                Some(true) => {}
                // we've been resurrected by __del__
                Some(false) => return Err(()),
                None => {
                    warn!("couldn't run __del__ method for object")
                }
            }
        }
        if let Some(wrl) = self.weak_ref_list() {
            wrl.clear();
        }

        Ok(())
    }

    /// Can only be called when ref_count has dropped to zero. `ptr` must be valid
    #[inline(never)]
    #[cold]
    unsafe fn drop_slow(ptr: NonNull<PyObject>) {
        if let Err(()) = ptr.as_ref().drop_slow_inner() {
            // abort drop for whatever reason
            return;
        }
        let drop_dealloc = ptr.as_ref().0.vtable.drop_dealloc;
        // call drop only when there are no references in scope - stacked borrows stuff
        drop_dealloc(ptr.as_ptr())
    }
}

impl Borrow<PyObject> for PyObjectRef {
    fn borrow(&self) -> &PyObject {
        self
    }
}

impl AsRef<PyObject> for PyObjectRef {
    fn as_ref(&self) -> &PyObject {
        self
    }
}

impl AsRef<PyObject> for PyObject {
    fn as_ref(&self) -> &PyObject {
        self
    }
}

impl IdProtocol for PyObjectRef {
    fn get_id(&self) -> usize {
        self.ptr.as_ptr() as usize
    }
}

impl IdProtocol for PyObject {
    fn get_id(&self) -> usize {
        self as *const PyObject as usize
    }
}

impl<'a, T: PyObjectPayload> From<&'a PyObjectView<T>> for &'a PyObject {
    fn from(py_ref: &'a PyObjectView<T>) -> Self {
        py_ref.as_object()
    }
}

impl<T> From<T> for PyObjectRef
where
    T: PyObjectWrap,
{
    fn from(py_ref: T) -> Self {
        py_ref.into_object()
    }
}

impl PyObjectWeak {
    #[inline]
    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.weak.upgrade()
    }

    pub fn into_object(self) -> PyObjectRef {
        self.weak.into_object()
    }
}

impl Drop for PyObjectRef {
    fn drop(&mut self) {
        if self.0.ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr) }
        }
    }
}

#[cold]
fn print_del_error(e: PyBaseExceptionRef, zelf: &PyObject, vm: &VirtualMachine) {
    // exception in del will be ignored but printed
    print!("Exception ignored in: ",);
    let del_method = zelf.get_class_attr("__del__").unwrap();
    let repr = &del_method.repr(vm);
    match repr {
        Ok(v) => println!("{}", v.to_string()),
        Err(_) => println!("{}", del_method.class().name()),
    }
    let tb_module = vm.import("traceback", None, 0).unwrap();
    // TODO: set exc traceback
    let print_stack = tb_module.get_attr("print_stack", vm).unwrap();
    vm.invoke(&print_stack, ()).unwrap();

    if let Ok(repr) = e.as_object().repr(vm) {
        println!("{}", repr.as_str());
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the vtable contains functions that accept payload types that always match up
        // with the payload of the object
        unsafe { ((*self).0.vtable.debug)(self, f) }
    }
}

impl fmt::Debug for PyObjectWeak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(PyWeak)")
    }
}

#[repr(transparent)]
pub struct PyObjectView<T: PyObjectPayload>(PyInner<T>);

impl<T: PyObjectPayload> PyObjectView<T> {
    #[inline(always)]
    pub fn as_object(&self) -> &PyObject {
        unsafe { &*(&self.0 as *const PyInner<T> as *const PyObject) }
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

impl<T: PyObjectPayload> ToOwned for PyObjectView<T> {
    type Owned = PyRef<T>;

    #[inline(always)]
    fn to_owned(&self) -> Self::Owned {
        self.0.ref_count.inc();
        PyRef {
            ptr: NonNull::from(self),
        }
    }
}

impl<T: PyObjectPayload> Deref for PyObjectView<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0.payload
    }
}

impl<T> AsRef<PyObject> for PyObjectView<T>
where
    T: PyObjectPayload,
{
    fn as_ref(&self) -> &PyObject {
        self.as_object()
    }
}

impl<T: PyObjectPayload> fmt::Debug for PyObjectView<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
pub struct PyRef<T: PyObjectPayload> {
    ptr: NonNull<PyObjectView<T>>,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        unsafe impl<T: PyObjectPayload> Send for PyRef<T> {}
        unsafe impl<T: PyObjectPayload> Sync for PyRef<T> {}
    }
}

impl<T: PyObjectPayload> fmt::Debug for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: PyObjectPayload> Drop for PyRef<T> {
    #[inline]
    fn drop(&mut self) {
        if self.0.ref_count.dec() {
            unsafe { PyObject::drop_slow(self.ptr.cast::<PyObject>()) }
        }
    }
}

impl<T: PyObjectPayload> Clone for PyRef<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

impl<T: PyObjectPayload> PyRef<T> {
    unsafe fn from_raw(raw: *const PyObjectView<T>) -> Self {
        Self {
            ptr: NonNull::new_unchecked(raw as *mut _),
        }
    }

    /// Safety: payload type of `obj` must be `T`
    #[inline]
    unsafe fn from_obj_unchecked(obj: PyObjectRef) -> Self {
        debug_assert!(obj.payload_is::<T>());
        let obj = ManuallyDrop::new(obj);
        Self {
            ptr: obj.ptr.cast(),
        }
    }

    #[inline]
    pub fn new_ref(payload: T, typ: crate::builtins::PyTypeRef, dict: Option<PyDictRef>) -> Self {
        let inner = Box::into_raw(PyInner::new(payload, typ, dict));
        Self {
            ptr: unsafe { NonNull::new_unchecked(inner.cast::<PyObjectView<T>>()) },
        }
    }
}

impl<T> PyObjectWrap for PyRef<T>
where
    T: PyObjectPayload,
{
    #[inline]
    fn into_object(self) -> PyObjectRef {
        let me = ManuallyDrop::new(self);
        PyObjectRef { ptr: me.ptr.cast() }
    }
}

impl<T> AsRef<PyObject> for PyRef<T>
where
    T: PyObjectPayload,
{
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        (**self).as_object()
    }
}

impl<T> Borrow<PyObjectView<T>> for PyRef<T>
where
    T: PyObjectPayload,
{
    fn borrow(&self) -> &PyObjectView<T> {
        self
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyObjectPayload,
{
    type Target = PyObjectView<T>;

    #[inline(always)]
    fn deref(&self) -> &PyObjectView<T> {
        unsafe { self.ptr.as_ref() }
    }
}

#[repr(transparent)]
pub struct PyWeakRef<T: PyObjectPayload> {
    weak: PyObjectWeak,
    _marker: PhantomData<T>,
}

impl<T: PyObjectPayload> PyWeakRef<T> {
    pub fn upgrade(&self) -> Option<PyRef<T>> {
        self.weak
            .upgrade()
            // SAFETY: PyWeakRef<T> was always created from a PyRef<T>, so the object is T
            .map(|obj| unsafe { PyRef::from_obj_unchecked(obj) })
    }
}

/// Paritally initialize a struct, ensuring that all fields are
/// either given values or explicitly left uninitialized
macro_rules! partially_init {
    (
        $ty:path {$($init_field:ident: $init_value:expr),*$(,)?},
        Uninit { $($uninit_field:ident),*$(,)? }$(,)?
    ) => {{
        // check all the fields are there but *don't* actually run it
        if false {
            #[allow(invalid_value, dead_code, unreachable_code)]
            let _ = {$ty {
                $($init_field: $init_value,)*
                $($uninit_field: unreachable!(),)*
            }};
        }
        let mut m = ::std::mem::MaybeUninit::<$ty>::uninit();
        #[allow(unused_unsafe)]
        unsafe {
            $(::std::ptr::write(&mut (*m.as_mut_ptr()).$init_field, $init_value);)*
        }
        m
    }};
}

pub(crate) fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef, PyTypeRef) {
    use crate::builtins::{object, PyType};
    use crate::{PyAttributes, PyClassImpl};
    use std::mem::MaybeUninit;

    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        type UninitRef<T> = PyRwLock<NonNull<PyInner<T>>>;

        // We cast between these 2 types, so make sure (at compile time) that there's no change in
        // layout when we wrap PyInner<PyTypeObj> in MaybeUninit<>
        static_assertions::assert_eq_size!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);
        static_assertions::assert_eq_align!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);

        let type_payload = PyType {
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::default()),
            slots: PyType::make_slots(),
        };
        let object_payload = PyType {
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::default()),
            slots: object::PyBaseObject::make_slots(),
        };
        let type_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyInner::<PyType> {
                ref_count: RefCount::new(),
                typeid: TypeId::of::<PyType>(),
                vtable: PyObjVTable::of::<PyType>(),
                dict: None,
                weak_list: WeakRefList::new(),
                payload: type_payload,
            },
            Uninit { typ }
        )));
        let object_type_ptr = Box::into_raw(Box::new(partially_init!(
            PyInner::<PyType> {
                ref_count: RefCount::new(),
                typeid: TypeId::of::<PyType>(),
                vtable: PyObjVTable::of::<PyType>(),
                dict: None,
                weak_list: WeakRefList::new(),
                payload: object_payload,
            },
            Uninit { typ },
        )));

        let object_type_ptr =
            object_type_ptr as *mut MaybeUninit<PyInner<PyType>> as *mut PyInner<PyType>;
        let type_type_ptr =
            type_type_ptr as *mut MaybeUninit<PyInner<PyType>> as *mut PyInner<PyType>;

        unsafe {
            (*type_type_ptr).ref_count.inc();
            ptr::write(
                &mut (*object_type_ptr).typ as *mut PyRwLock<PyTypeRef> as *mut UninitRef<PyType>,
                PyRwLock::new(NonNull::new_unchecked(type_type_ptr)),
            );
            (*type_type_ptr).ref_count.inc();
            ptr::write(
                &mut (*type_type_ptr).typ as *mut PyRwLock<PyTypeRef> as *mut UninitRef<PyType>,
                PyRwLock::new(NonNull::new_unchecked(type_type_ptr)),
            );

            let object_type = PyTypeRef::from_raw(object_type_ptr.cast());

            (*type_type_ptr).payload.mro = vec![object_type.clone()];
            (*type_type_ptr).payload.bases = vec![object_type.clone()];
            (*type_type_ptr).payload.base = Some(object_type.clone());

            let type_type = PyTypeRef::from_raw(type_type_ptr.cast());

            (type_type, object_type)
        }
    };

    let weakref_type = PyType {
        base: Some(object_type.clone()),
        bases: vec![object_type.clone()],
        mro: vec![object_type.clone()],
        subclasses: PyRwLock::default(),
        attributes: PyRwLock::default(),
        slots: PyWeak::make_slots(),
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
        let ctx = crate::PyContext::new();
        let obj = ctx.new_bytes(b"dfghjkl".to_vec());
        drop(obj);
    }
}

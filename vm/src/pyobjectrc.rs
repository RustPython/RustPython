use crate::builtins::{PyDictRef, PyTypeRef};
use crate::common::lock::PyRwLock;
use crate::common::rc::{PyRc, PyWeak};
use crate::pyobject::{self, IdProtocol, PyObjectPayload, TypeProtocol};
use crate::VirtualMachine;
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::Deref;

// so, PyObjectRef is basically equivalent to `PyRc<PyObject<dyn PyObjectPayload>>`, except it's
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
// `payload` in `PyObject<dyn PyObjectPayload>`, which has a huge performance impact when *every
// single payload access* requires a vtable lookup. Thankfully, we're able to avoid that because of
// the way we use PyObjectRef, in that whenever we want to access the payload we (almost) always
// access it from a generic function. So, rather than doing
//
// - check vtable for payload offset
// - get offset in PyObject struct
// - call as_any() method of PyObjectPayload
// - call downcast_ref() method of Any
// we can just do
// - check vtable that typeid matches
// - pointer cast directly to *const PyObject<T>
//
// and at that point the compiler can know the offset of `payload` for us because **we've given it a
// concrete type to work with before we ever access the `payload` field**

/// A type to just represent "we've erased the type of this object, cast it before you use it"
struct Erased;

struct PyObjVTable {
    drop: unsafe fn(*mut PyInner<Erased>),
    debug: unsafe fn(*const PyInner<Erased>, &mut fmt::Formatter) -> fmt::Result,
}
unsafe fn drop_obj<T: PyObjectPayload>(x: *mut PyInner<Erased>) {
    std::ptr::drop_in_place(x as *mut PyInner<T>)
}
unsafe fn debug_obj<T: PyObjectPayload>(
    x: *const PyInner<Erased>,
    f: &mut fmt::Formatter,
) -> fmt::Result {
    let x = &*x.cast::<PyInner<T>>();
    fmt::Debug::fmt(x, f)
}
impl PyObjVTable {
    pub fn of<T: PyObjectPayload>() -> &'static Self {
        &PyObjVTable {
            drop: drop_obj::<T>,
            debug: debug_obj::<T>,
        }
    }
}

#[repr(C)]
struct PyInner<T> {
    // TODO: move typeid into vtable once TypeId::of is const
    typeid: TypeId,
    vtable: &'static PyObjVTable,

    typ: PyRwLock<PyTypeRef>,          // __class__ member
    dict: Option<PyRwLock<PyDictRef>>, // __dict__ member

    payload: T,
}

impl<T: fmt::Debug> fmt::Debug for PyInner<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObj {:?}]", &self.payload)
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
#[repr(transparent)]
pub struct PyObject<T> {
    inner: ManuallyDrop<PyInner<T>>,
}

impl<T: PyObjectPayload> PyObject<T> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> PyObjectRef {
        let inner = PyInner {
            typeid: TypeId::of::<T>(),
            vtable: PyObjVTable::of::<T>(),
            typ: PyRwLock::new(typ),
            dict: dict.map(PyRwLock::new),
            payload,
        };
        PyObjectRef::new(PyObject {
            inner: ManuallyDrop::new(inner),
        })
    }
}

impl<T> Drop for PyObject<T> {
    fn drop(&mut self) {
        let erased = &mut *self.inner as *mut _ as *mut PyInner<Erased>;
        // SAFETY: the vtable contains functions that accept payload types that always match up
        // with the payload of the object
        unsafe { (self.inner.vtable.drop)(erased) }
    }
}

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
#[derive(Clone)]
#[repr(transparent)]
pub struct PyObjectRef {
    rc: PyRc<PyObject<Erased>>,
}

#[derive(Clone)]
#[repr(transparent)]
pub struct PyObjectWeak {
    weak: PyWeak<PyObject<Erased>>,
}

/// A marker type that just references a raw python object. Don't use directly, pass as a pointer
/// back to [`PyObjectRef::from_raw`]
pub enum RawPyObject {}

impl PyObjectRef {
    pub fn into_raw(this: Self) -> *const RawPyObject {
        let ptr = PyRc::as_ptr(&this.rc);
        std::mem::forget(this);
        ptr.cast()
    }

    /// # Safety
    /// The raw pointer must have been previously returned from a call to
    /// [`PyObjectRef::into_raw`]. The user is responsible for ensuring that the inner data is not
    /// dropped more than once due to mishandling the reference count by calling this function
    /// too many times.
    pub unsafe fn from_raw(ptr: *const RawPyObject) -> Self {
        Self {
            rc: PyRc::from_raw(ptr.cast()),
        }
    }

    fn new<T: PyObjectPayload>(value: PyObject<T>) -> Self {
        let inner = PyRc::into_raw(PyRc::new(value));
        let rc = unsafe { PyRc::from_raw(inner as *const PyObject<Erased>) };
        Self { rc }
    }

    pub fn strong_count(this: &Self) -> usize {
        PyRc::strong_count(&this.rc)
    }

    pub fn weak_count(this: &Self) -> usize {
        PyRc::weak_count(&this.rc)
    }

    pub fn downgrade(this: &Self) -> PyObjectWeak {
        PyObjectWeak {
            weak: PyRc::downgrade(&this.rc),
        }
    }

    pub fn payload_is<T: PyObjectPayload>(&self) -> bool {
        self.rc.inner.typeid == TypeId::of::<T>()
    }

    pub fn payload<T: PyObjectPayload>(&self) -> Option<&T> {
        if self.payload_is::<T>() {
            // we cast to a PyInner<T> first because we don't know T's exact offset because of
            // varying alignment, but once we get a PyInner<T> the compiler can get it for us
            let inner =
                unsafe { &*(&*self.rc.inner as *const PyInner<Erased> as *const PyInner<T>) };
            Some(&inner.payload)
        } else {
            None
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

    pub fn downcast_ref<T: PyObjectPayload>(&self) -> Option<&PyRef<T>> {
        if self.payload_is::<T>() {
            // SAFETY: just checked that the payload is T, and PyRef is repr(transparent) over
            // PyObjectRef
            Some(unsafe { &*(self as *const PyObjectRef as *const PyRef<T>) })
        } else {
            None
        }
    }

    pub(crate) fn class_lock(&self) -> &PyRwLock<PyTypeRef> {
        &self.rc.inner.typ
    }

    // ideally we'd be able to define these in pyobject.rs, but method visibility rules are weird

    /// Attempt to downcast this reference to the specific class that is associated `T`.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    pub fn downcast_exact<T: PyObjectPayload + pyobject::PyValue>(
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

    #[inline]
    pub fn payload_if_exact<T: PyObjectPayload + pyobject::PyValue>(
        &self,
        vm: &VirtualMachine,
    ) -> Option<&T> {
        if self.class().is(T::class(vm)) {
            self.payload()
        } else {
            None
        }
    }

    pub fn dict(&self) -> Option<PyDictRef> {
        self.rc.inner.dict.as_ref().map(|mu| mu.read().clone())
    }
    /// Set the dict field. Returns `Err(dict)` if this object does not have a dict field
    /// in the first place.
    pub fn set_dict(&self, dict: PyDictRef) -> Result<(), PyDictRef> {
        match self.rc.inner.dict {
            Some(ref mu) => {
                *mu.write() = dict;
                Ok(())
            }
            None => Err(dict),
        }
    }

    #[inline]
    pub fn payload_if_subclass<T: pyobject::PyValue>(
        &self,
        vm: &crate::VirtualMachine,
    ) -> Option<&T> {
        if self.class().issubclass(T::class(vm)) {
            self.payload()
        } else {
            None
        }
    }
}

impl IdProtocol for PyObjectRef {
    fn get_id(&self) -> usize {
        self.rc.get_id()
    }
}

impl PyObjectWeak {
    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.weak.upgrade().map(|rc| PyObjectRef { rc })
    }
}

impl Drop for PyObjectRef {
    fn drop(&mut self) {
        use crate::pyobject::BorrowValue;

        // PyObjectRef will drop the value when its count goes to 0
        if PyRc::strong_count(&self.rc) != 1 {
            return;
        }

        // CPython-compatible drop implementation
        let zelf = self.clone();
        if let Some(del_slot) = self.class().mro_find_map(|cls| cls.slots.del.load()) {
            let ret = crate::vm::thread::with_vm(&zelf, |vm| {
                if let Err(e) = del_slot(&zelf, vm) {
                    // exception in del will be ignored but printed
                    print!("Exception ignored in: ",);
                    let del_method = zelf.get_class_attr("__del__").unwrap();
                    let repr = vm.to_repr(&del_method);
                    match repr {
                        Ok(v) => println!("{}", v.to_string()),
                        Err(_) => println!("{}", del_method.class().name),
                    }
                    let tb_module = vm.import("traceback", None, 0).unwrap();
                    // TODO: set exc traceback
                    let print_stack = vm.get_attribute(tb_module, "print_stack").unwrap();
                    vm.invoke(&print_stack, ()).unwrap();

                    if let Ok(repr) = vm.to_repr(e.as_object()) {
                        println!("{}", repr.borrow_value());
                    }
                }
            });
            if ret.is_none() {
                warn!("couldn't run __del__ method for object")
            }
        }

        // __del__ might have resurrected the object at this point, but that's fine,
        // inner.strong_count would be >1 now and it'll maybe get dropped the next time
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: the vtable contains functions that accept payload types that always match up
        // with the payload of the object
        unsafe { (self.rc.inner.vtable.debug)(&*self.rc.inner, f) }
    }
}

impl fmt::Debug for PyObjectWeak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(PyWeak)")
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
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<PyRc<T>>,
}

impl<T: PyObjectPayload> fmt::Debug for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.obj.fmt(f)
    }
}

impl<T: PyObjectPayload> Clone for PyRef<T> {
    fn clone(&self) -> Self {
        Self {
            obj: self.obj.clone(),
            _payload: PhantomData,
        }
    }
}

impl<T: PyObjectPayload> PyRef<T> {
    /// Safety: payload type of `obj` must be `T`
    unsafe fn from_obj_unchecked(obj: PyObjectRef) -> Self {
        PyRef {
            obj,
            _payload: PhantomData,
        }
    }

    #[inline(always)]
    pub fn as_object(&self) -> &PyObjectRef {
        &self.obj
    }

    #[inline(always)]
    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }

    pub fn downgrade(this: &Self) -> PyWeakRef<T> {
        PyWeakRef {
            weak: PyObjectRef::downgrade(&this.obj),
            _payload: PhantomData,
        }
    }

    // ideally we'd be able to define this in pyobject.rs, but method visibility rules are weird
    pub fn new_ref(
        payload: T,
        typ: crate::builtins::PyTypeRef,
        dict: Option<crate::builtins::PyDictRef>,
    ) -> Self {
        let obj = PyObject::new(payload, typ, dict);
        // SAFETY: we just created the object from a payload of type T
        unsafe { Self::from_obj_unchecked(obj) }
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyObjectPayload,
{
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: per the invariant on `self.obj`, the payload of the pyobject is always T, so it
        // can always be cast to a PyInner<T>
        let obj = unsafe { &*(&*self.obj.rc.inner as *const PyInner<Erased> as *const PyInner<T>) };
        &obj.payload
    }
}

#[repr(transparent)]
pub struct PyWeakRef<T: PyObjectPayload> {
    weak: PyObjectWeak,
    _payload: PhantomData<PyWeak<T>>,
}

impl<T: PyObjectPayload> PyWeakRef<T> {
    pub fn upgrade(&self) -> Option<PyRef<T>> {
        self.weak.upgrade().map(|obj| unsafe {
            // SAFETY: PyWeakRef<T> is only ever created from a PyRef<T>
            PyRef::from_obj_unchecked(obj)
        })
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

pub(crate) fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef) {
    use crate::builtins::{object, PyType, PyWeak};
    use crate::pyobject::{PyAttributes, PyClassDef, PyClassImpl};
    use std::mem::MaybeUninit;
    use std::ptr;

    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        type UninitRef<T> = PyRwLock<PyRc<MaybeUninit<PyInner<T>>>>;

        // We cast between these 2 types, so make sure (at compile time) that there's no change in
        // layout when we wrap PyInner<PyTypeObj> in MaybeUninit<>
        static_assertions::assert_eq_size!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);
        static_assertions::assert_eq_align!(MaybeUninit<PyInner<PyType>>, PyInner<PyType>);

        let type_payload = PyType {
            name: PyTypeRef::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::default()),
            slots: PyType::make_slots(),
        };
        let object_payload = PyType {
            name: object::PyBaseObject::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::default()),
            slots: object::PyBaseObject::make_slots(),
        };
        let type_type = PyRc::new(partially_init!(
            PyInner::<PyType> {
                typeid: TypeId::of::<PyType>(),
                vtable: PyObjVTable::of::<PyType>(),
                dict: None,
                payload: type_payload,
            },
            Uninit { typ }
        ));
        let object_type = PyRc::new(partially_init!(
            PyInner::<PyType> {
                typeid: TypeId::of::<PyType>(),
                vtable: PyObjVTable::of::<PyType>(),
                dict: None,
                payload: object_payload,
            },
            Uninit { typ },
        ));

        let object_type_ptr = PyRc::into_raw(object_type) as *mut MaybeUninit<PyInner<PyType>>
            as *mut PyInner<PyType>;
        let type_type_ptr = PyRc::into_raw(type_type.clone()) as *mut MaybeUninit<PyInner<PyType>>
            as *mut PyInner<PyType>;

        unsafe {
            ptr::write(
                &mut (*object_type_ptr).typ as *mut PyRwLock<PyTypeRef> as *mut UninitRef<PyType>,
                PyRwLock::new(type_type.clone()),
            );
            ptr::write(
                &mut (*type_type_ptr).typ as *mut PyRwLock<PyTypeRef> as *mut UninitRef<PyType>,
                PyRwLock::new(type_type),
            );

            let type_type =
                PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(type_type_ptr.cast()));
            let object_type =
                PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(object_type_ptr.cast()));

            (*type_type_ptr).payload.mro = vec![object_type.clone()];
            (*type_type_ptr).payload.bases = vec![object_type.clone()];
            (*type_type_ptr).payload.base = Some(object_type.clone());

            (type_type, object_type)
        }
    };

    object_type
        .subclasses
        .write()
        .push(PyWeak::downgrade(&type_type.as_object()));

    (type_type, object_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn miri_test_type_initialization() {
        let _ = init_type_hierarchy();
    }
}

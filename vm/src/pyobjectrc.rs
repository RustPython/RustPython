use crate::common::rc::{PyRc, PyWeak};
use crate::pyobject::{self, IdProtocol, PyObject, PyObjectPayload, TypeProtocol};
use crate::VirtualMachine;
use std::any::TypeId;
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::Deref;

struct Erased;

struct PyObjVTable {
    drop: unsafe fn(*mut PyObject<Erased>),
    debug: unsafe fn(*const PyObject<Erased>, &mut fmt::Formatter) -> fmt::Result,
}
unsafe fn drop_obj<T: PyObjectPayload>(x: *mut PyObject<Erased>) {
    std::ptr::drop_in_place(x as *mut PyObject<T>)
}
unsafe fn debug_obj<T: PyObjectPayload>(
    x: *const PyObject<Erased>,
    f: &mut fmt::Formatter,
) -> fmt::Result {
    let x = &*x.cast::<PyObject<T>>();
    fmt::Debug::fmt(x, f)
}

macro_rules! make_vtable {
    ($t:ty) => {
        &PyObjVTable {
            drop: drop_obj::<$t>,
            debug: debug_obj::<$t>,
        }
    };
}

#[repr(C)]
struct PyInner<T> {
    // TODO: move typeid into vtable once TypeId::of is const
    typeid: TypeId,
    vtable: &'static PyObjVTable,
    value: ManuallyDrop<PyObject<T>>,
}

impl<T: PyObjectPayload> PyInner<T> {
    fn new(value: PyObject<T>) -> Self {
        PyInner {
            typeid: TypeId::of::<T>(),
            vtable: make_vtable!(T),
            value: ManuallyDrop::new(value),
        }
    }
}

impl<T> Drop for PyInner<T> {
    fn drop(&mut self) {
        let erased = &mut *self.value as *mut _ as *mut PyObject<Erased>;
        unsafe { (self.vtable.drop)(erased) }
    }
}

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
#[derive(Clone)]
pub struct PyObjectRef {
    inner: PyRc<PyInner<Erased>>,
}

#[derive(Clone)]
pub struct PyObjectWeak {
    inner: PyWeak<PyInner<Erased>>,
}

pub enum RawPyObject {}

impl PyObjectRef {
    pub fn into_raw(this: Self) -> *const RawPyObject {
        let ptr = PyRc::as_ptr(&this.inner);
        std::mem::forget(this);
        ptr.cast()
    }

    /// # Safety
    /// See PyRc::from_raw
    pub unsafe fn from_raw(ptr: *const RawPyObject) -> Self {
        Self {
            inner: PyRc::from_raw(ptr.cast()),
        }
    }

    pub fn new<T: PyObjectPayload>(value: PyObject<T>) -> Self {
        let inner = PyRc::into_raw(PyRc::new(PyInner::<T>::new(value)));
        let inner = unsafe { PyRc::from_raw(inner as *const PyInner<Erased>) };
        Self { inner }
    }

    pub fn strong_count(this: &Self) -> usize {
        PyRc::strong_count(&this.inner)
    }

    pub fn weak_count(this: &Self) -> usize {
        PyRc::weak_count(&this.inner)
    }

    pub fn downgrade(this: &Self) -> PyObjectWeak {
        PyObjectWeak {
            inner: PyRc::downgrade(&this.inner),
        }
    }

    pub fn payload_is<T: PyObjectPayload>(&self) -> bool {
        self.inner.typeid == TypeId::of::<T>()
    }

    pub fn payload<T: PyObjectPayload>(&self) -> Option<&T> {
        if self.payload_is::<T>() {
            // we cast to a PyObject first because we don't know T's exact offset because of varying alignment,
            // but we *do* know that PyObject<T> is always
            let pyobj =
                unsafe { &*(&*self.inner.value as *const PyObject<Erased> as *const PyObject<T>) };
            Some(&pyobj.payload)
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
            // when payload exacts, PyObjectRef == PyRef { PyObject }
            Some(unsafe { &*(self as *const PyObjectRef as *const PyRef<T>) })
        } else {
            None
        }
    }

    pub(crate) fn class_lock(&self) -> &crate::common::lock::PyRwLock<crate::builtins::PyTypeRef> {
        &self.inner.value.typ
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

    pub fn dict(&self) -> Option<crate::builtins::PyDictRef> {
        self.inner.value.dict()
    }
    /// Set the dict field. Returns `Err(dict)` if this object does not have a dict field
    /// in the first place.
    pub fn set_dict(
        &self,
        dict: crate::builtins::PyDictRef,
    ) -> Result<(), crate::builtins::PyDictRef> {
        self.inner.value.set_dict(dict)
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
        self.inner.get_id()
    }
}

impl PyObjectWeak {
    pub fn upgrade(&self) -> Option<PyObjectRef> {
        self.inner.upgrade().map(|inner| PyObjectRef { inner })
    }
}

impl Drop for PyObjectRef {
    fn drop(&mut self) {
        use crate::pyobject::BorrowValue;

        // PyObjectRef will drop the value when its count goes to 0
        if PyRc::strong_count(&self.inner) != 1 {
            return;
        }

        // CPython-compatible drop implementation
        let zelf = self.clone();
        if let Some(del_slot) = self.class().mro_find_map(|cls| cls.slots.del.load()) {
            crate::vm::thread::with_vm(&zelf, |vm| {
                if let Err(e) = del_slot(&zelf, vm) {
                    // exception in del will be ignored but printed
                    print!("Exception ignored in: ",);
                    let del_method = zelf.get_class_attr("__del__").unwrap();
                    let repr = vm.to_repr(&del_method);
                    match repr {
                        Ok(v) => println!("{}", v.to_string()),
                        Err(_) => println!("{}", del_method.class().name),
                    }
                    let tb_module = vm.import("traceback", &[], 0).unwrap();
                    // TODO: set exc traceback
                    let print_stack = vm.get_attribute(tb_module, "print_stack").unwrap();
                    vm.invoke(&print_stack, ()).unwrap();

                    if let Ok(repr) = vm.to_repr(e.as_object()) {
                        println!("{}", repr.borrow_value());
                    }
                }
            });
        }

        // __del__ might have resurrected the object, but that's fine, strong_count would be >1 now
    }
}

impl fmt::Debug for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe { (self.inner.vtable.debug)(&*self.inner.value, f) }
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
#[derive(Debug)]
#[repr(transparent)]
pub struct PyRef<T: PyObjectPayload> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<PyRc<T>>,
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
    #[allow(clippy::new_ret_no_self)]
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
        let obj =
            unsafe { &*(&*self.obj.inner.value as *const PyObject<Erased> as *const PyObject<T>) };
        &obj.payload
    }
}

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

impl TypeProtocol for PyObjectRef {
    fn class(&self) -> pyobject::PyLease<'_, crate::builtins::PyType> {
        self.inner.value.class()
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

use crate::builtins::PyTypeRef;
pub(crate) fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef) {
    use crate::builtins::{object, PyType, PyWeak};
    use crate::common::lock::PyRwLock;
    use crate::pyobject::{PyAttributes, PyClassDef, PyClassImpl};
    use std::mem::MaybeUninit;
    use std::ptr;

    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        type PyTypeObj = PyObject<PyType>;
        type UninitRef<T> = PyRwLock<PyRc<MaybeUninit<PyInner<T>>>>;

        // We cast between these 2 types, so make sure (at compile time) that there's no change in
        // layout when we wrap PyInner<PyTypeObj> in MaybeUninit<>
        static_assertions::assert_eq_size!(MaybeUninit<PyInner<PyTypeObj>>, PyInner<PyTypeObj>);
        static_assertions::assert_eq_align!(MaybeUninit<PyInner<PyTypeObj>>, PyInner<PyTypeObj>);

        let type_payload = PyType {
            name: PyTypeRef::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::new()),
            slots: PyType::make_slots(),
        };
        let object_payload = PyType {
            name: object::PyBaseObject::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::new()),
            slots: object::PyBaseObject::make_slots(),
        };
        let type_type = PyRc::new(partially_init!(
            PyInner::<PyType> {
                typeid: TypeId::of::<PyType>(),
                vtable: make_vtable!(PyType),
            },
            Uninit { value }
        ));
        let object_type = PyRc::new(partially_init!(
            PyInner::<PyType> {
                typeid: TypeId::of::<PyType>(),
                vtable: make_vtable!(PyType),
                // dict: None,
                // payload: object_payload,
            },
            Uninit { value },
        ));

        let object_type_ptr = PyRc::into_raw(object_type) as *mut MaybeUninit<PyInner<PyType>>
            as *mut PyInner<PyType>;
        let type_type_ptr = PyRc::into_raw(type_type.clone()) as *mut MaybeUninit<PyInner<PyType>>
            as *mut PyInner<PyType>;

        unsafe {
            // TODO: make this part of the partially_init!() method
            // partially initialize the inner PyObject of PyInner
            std::ptr::write(
                &mut (*type_type_ptr).value as *mut ManuallyDrop<PyTypeObj>
                    as *mut MaybeUninit<PyTypeObj>,
                partially_init!(
                    PyTypeObj {
                        dict: None,
                        payload: type_payload,
                    },
                    Uninit { typ }
                ),
            );
            std::ptr::write(
                &mut (*object_type_ptr).value as *mut ManuallyDrop<PyTypeObj>
                    as *mut MaybeUninit<PyTypeObj>,
                partially_init!(
                    PyTypeObj {
                        dict: None,
                        payload: object_payload,
                    },
                    Uninit { typ }
                ),
            );

            ptr::write(
                &mut (*object_type_ptr).value.typ as *mut PyRwLock<PyTypeRef>
                    as *mut UninitRef<PyType>,
                PyRwLock::new(type_type.clone()),
            );
            ptr::write(
                &mut (*type_type_ptr).value.typ as *mut PyRwLock<PyTypeRef>
                    as *mut UninitRef<PyType>,
                PyRwLock::new(type_type),
            );

            let type_type =
                PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(type_type_ptr.cast()));
            let object_type =
                PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(object_type_ptr.cast()));

            (*type_type_ptr).value.payload.mro = vec![object_type.clone()];
            (*type_type_ptr).value.payload.bases = vec![object_type.clone()];
            (*type_type_ptr).value.payload.base = Some(object_type.clone());

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
    fn test_type_initialization() {
        let _ = init_type_hierarchy();
    }
}

use crate::common::rc::{PyRc, PyWeak};
use crate::pyobject::{IdProtocol, PyObject, PyObjectPayload, TypeProtocol};
use std::borrow;
use std::fmt;
use std::ops::Deref;

pub struct PyObjectRc<T: ?Sized + AsPyObjectPayload = dyn PyObjectPayload> {
    inner: PyRc<PyObject<T>>,
}

pub struct PyObjectWeak<T: ?Sized + PyObjectPayload = dyn PyObjectPayload> {
    inner: PyWeak<PyObject<T>>,
}

// invariant: must never be constructed directly, as a &PyObjectRcB<T> should always be
// the result of PyRc<PyObject<T>>.deref()
#[repr(transparent)]
pub struct PyObjectRcB<T: ?Sized + AsPyObjectPayload = dyn PyObjectPayload>(PyObject<T>);

pub trait AsPyObjectPayload: PyObjectPayload {
    fn rc_to_pyobj(rc: PyRc<PyObject<Self>>) -> PyRc<PyObject<dyn PyObjectPayload>>;
}

impl<T: PyObjectPayload> AsPyObjectPayload for T {
    fn rc_to_pyobj(rc: PyRc<PyObject<Self>>) -> PyRc<PyObject<dyn PyObjectPayload>> {
        rc
    }
}

impl AsPyObjectPayload for dyn PyObjectPayload {
    fn rc_to_pyobj(rc: PyRc<PyObject<Self>>) -> PyRc<PyObject<dyn PyObjectPayload>> {
        rc
    }
}

impl<T: ?Sized + AsPyObjectPayload> PyObjectRc<T> {
    pub fn into_raw(this: Self) -> *const PyObject<T> {
        let ptr = PyRc::as_ptr(&this.inner);
        std::mem::forget(this);
        ptr
    }

    unsafe fn into_rc(this: Self) -> PyRc<PyObject<T>> {
        let raw = Self::into_raw(this);
        PyRc::from_raw(raw)
    }

    pub fn into_ref(this: Self) -> PyObjectRc<dyn PyObjectPayload> {
        let rc = unsafe { Self::into_rc(this) };
        PyObjectRc::<dyn PyObjectPayload> {
            inner: T::rc_to_pyobj(rc),
        }
    }

    /// # Safety
    /// See PyRc::from_raw
    pub unsafe fn from_raw(ptr: *const PyObject<T>) -> Self {
        Self {
            inner: PyRc::from_raw(ptr),
        }
    }

    pub fn new(value: PyObject<T>) -> Self
    where
        T: Sized,
    {
        Self {
            inner: PyRc::new(value),
        }
    }

    pub fn strong_count(this: &Self) -> usize {
        PyRc::strong_count(&this.inner)
    }

    pub fn weak_count(this: &Self) -> usize {
        PyRc::weak_count(&this.inner)
    }

    pub fn downgrade(this: &Self) -> PyObjectWeak<T> {
        PyObjectWeak {
            inner: PyRc::downgrade(&this.inner),
        }
    }
}

impl<T: ?Sized + AsPyObjectPayload> IdProtocol for PyObjectRc<T> {
    fn get_id(&self) -> usize {
        self.inner.get_id()
    }
}

impl<T: ?Sized + AsPyObjectPayload> PyObjectWeak<T> {
    pub fn upgrade(&self) -> Option<PyObjectRc<T>> {
        self.inner.upgrade().map(|inner| PyObjectRc { inner })
    }
}

impl<T: ?Sized + AsPyObjectPayload> Drop for PyObjectRc<T> {
    fn drop(&mut self) {
        use crate::pyobject::BorrowValue;

        // PyObjectRc will drop the value when its count goes to 0
        if PyRc::strong_count(&self.inner) != 1 {
            return;
        }

        // CPython-compatible drop implementation
        let zelf = Self::into_ref(self.clone());
        if let Some(del_slot) = zelf.class().mro_find_map(|cls| cls.slots.del.load()) {
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
                    vm.invoke(&print_stack, vec![]).unwrap();

                    if let Ok(repr) = vm.to_repr(e.as_object()) {
                        println!("{}", repr.borrow_value());
                    }
                }
            });
        }

        let _ = unsafe { PyObjectRc::<dyn PyObjectPayload>::into_rc(zelf) };
        debug_assert!(PyRc::strong_count(&self.inner) == 1); // make sure to keep same state
    }
}

impl<T: ?Sized + AsPyObjectPayload> Deref for PyObjectRc<T> {
    type Target = PyObjectRcB<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.inner.deref() as *const PyObject<T> as *const PyObjectRcB<T>) }
    }
}

impl<T: ?Sized + AsPyObjectPayload> Deref for PyObjectRcB<T> {
    type Target = PyObject<T>;

    #[inline]
    fn deref(&self) -> &PyObject<T> {
        &self.0
    }
}

impl<T: ?Sized + AsPyObjectPayload> ToOwned for PyObjectRcB<T> {
    type Owned = PyObjectRc<T>;
    fn to_owned(&self) -> PyObjectRc<T> {
        let x = unsafe { PyObjectRc::from_raw(&self.0) };
        std::mem::forget(x.clone());
        x
    }
}

impl<T: ?Sized + AsPyObjectPayload> Clone for PyObjectRc<T> {
    fn clone(&self) -> Self {
        PyObjectRc {
            inner: self.inner.clone(),
        }
    }
}

impl<T: ?Sized + AsPyObjectPayload> fmt::Display for PyObjectRc<T>
where
    PyObject<T>: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: ?Sized + AsPyObjectPayload> fmt::Debug for PyObjectRc<T>
where
    PyObject<T>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: ?Sized + AsPyObjectPayload> fmt::Pointer for PyObjectRc<T>
where
    PyObject<T>: fmt::Pointer,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<T: ?Sized + AsPyObjectPayload> borrow::Borrow<PyObjectRcB<T>> for PyObjectRc<T> {
    fn borrow(&self) -> &PyObjectRcB<T> {
        self
    }
}

impl<T: ?Sized + AsPyObjectPayload> borrow::Borrow<T> for PyObjectRc<T>
where
    PyRc<PyObject<T>>: borrow::Borrow<T>,
{
    fn borrow(&self) -> &T {
        self.inner.borrow()
    }
}

impl<T: ?Sized + AsPyObjectPayload> AsRef<T> for PyObjectRc<T>
where
    PyRc<PyObject<T>>: AsRef<T>,
{
    fn as_ref(&self) -> &T {
        self.inner.as_ref()
    }
}

impl<T: ?Sized + PyObjectPayload> Clone for PyObjectWeak<T> {
    fn clone(&self) -> Self {
        PyObjectWeak {
            inner: self.inner.clone(),
        }
    }
}

impl<T: ?Sized + PyObjectPayload> fmt::Debug for PyObjectWeak<T>
where
    PyObject<T>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

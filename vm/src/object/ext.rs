use super::{
    core::{Py, PyObject, PyObjectRef, PyRef},
    payload::{PyObjectPayload, PyPayload},
};
use crate::common::lock::PyRwLockReadGuard;
use crate::{
    builtins::{object, pystr, PyBaseExceptionRef, PyType},
    convert::{ToPyObject, ToPyResult, TryFromObject},
    function::IntoFuncArgs,
    types::PyTypeFlags,
    VirtualMachine,
};
use std::{borrow::Borrow, fmt, ops::Deref};

/* Python objects and references.

Okay, so each python object itself is an class itself (PyObject). Each
python object can have several references to it (PyObjectRef). These
references are Rc (reference counting) rust smart pointers. So when
all references are destroyed, the object itself also can be cleaned up.
Basically reference counting, but then done by rust.

*/

/*
 * Good reference: https://github.com/ProgVal/pythonvm-rust/blob/master/src/objects/mod.rs
 */

/// Use this type for functions which return a python object or an exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyBaseExceptionRef>; // A valid value, or an exception

// TODO: remove these 2 impls
impl fmt::Display for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(f)
    }
}
impl fmt::Display for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "'{}' object", self.class().name())
    }
}

impl<T: fmt::Display> fmt::Display for PyRef<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<T: fmt::Display> fmt::Display for Py<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[derive(Debug)]
pub struct PyRefExact<T: PyObjectPayload> {
    inner: PyRef<T>,
}

impl<T: PyObjectPayload> PyRefExact<T> {
    /// # Safety
    /// obj must have exact type for the payload
    pub unsafe fn new_unchecked(obj: PyRef<T>) -> Self {
        Self { inner: obj }
    }

    pub fn into_pyref(self) -> PyRef<T> {
        self.inner
    }
}

impl<T: PyObjectPayload> Clone for PyRefExact<T> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self { inner }
    }
}

impl<T: PyPayload> TryFromObject for PyRefExact<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let target_cls = T::class(vm);
        let cls = obj.class();
        if cls.is(target_cls) {
            drop(cls);
            let obj = obj
                .downcast()
                .map_err(|obj| vm.new_downcast_runtime_error(target_cls, &obj))?;
            Ok(Self { inner: obj })
        } else if cls.fast_issubclass(target_cls) {
            Err(vm.new_type_error(format!(
                "Expected an exact instance of '{}', not a subclass '{}'",
                target_cls.name(),
                cls.name(),
            )))
        } else {
            Err(vm.new_type_error(format!(
                "Expected type '{}', not '{}'",
                target_cls.name(),
                cls.name(),
            )))
        }
    }
}

impl<T: PyPayload> Deref for PyRefExact<T> {
    type Target = PyRef<T>;
    #[inline(always)]
    fn deref(&self) -> &PyRef<T> {
        &self.inner
    }
}

impl<T: PyPayload> ToPyObject for PyRefExact<T> {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.inner.into()
    }
}

pub trait AsObject
where
    Self: Borrow<PyObject>,
{
    #[inline(always)]
    fn as_object(&self) -> &PyObject {
        self.borrow()
    }

    #[inline(always)]
    fn get_id(&self) -> usize {
        self.as_object().unique_id()
    }

    #[inline(always)]
    fn is<T>(&self, other: &T) -> bool
    where
        T: AsObject,
    {
        self.get_id() == other.get_id()
    }

    #[inline(always)]
    fn class(&self) -> PyLease<'_, PyType> {
        self.as_object().lease_class()
    }

    fn get_class_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        self.class().get_attr(attr_name)
    }

    /// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
    /// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
    #[inline]
    fn fast_isinstance(&self, cls: &Py<PyType>) -> bool {
        self.class().fast_issubclass(cls)
    }
}

impl<T> AsObject for T where T: Borrow<PyObject> {}

impl PyObject {
    #[inline(always)]
    fn unique_id(&self) -> usize {
        self as *const PyObject as usize
    }

    #[inline]
    fn lease_class(&self) -> PyLease<'_, PyType> {
        PyLease {
            inner: self.class_lock().read(),
        }
    }
}

// impl<T: ?Sized> Borrow<PyObject> for PyRc<T> {
//     #[inline(always)]
//     fn borrow(&self) -> &PyObject {
//         unsafe { &*(&**self as *const T as *const PyObject) }
//     }
// }

/// A borrow of a reference to a Python object. This avoids having clone the `PyRef<T>`/
/// `PyObjectRef`, which isn't that cheap as that increments the atomic reference counter.
pub struct PyLease<'a, T: PyObjectPayload> {
    inner: PyRwLockReadGuard<'a, PyRef<T>>,
}

impl<'a, T: PyObjectPayload + PyPayload> PyLease<'a, T> {
    #[inline(always)]
    pub fn into_owned(self) -> PyRef<T> {
        self.inner.clone()
    }
}

impl<'a, T: PyObjectPayload + PyPayload> Borrow<PyObject> for PyLease<'a, T> {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        self.inner.as_ref()
    }
}

impl<'a, T: PyObjectPayload + PyPayload> Deref for PyLease<'a, T> {
    type Target = PyRef<T>;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> fmt::Display for PyLease<'a, T>
where
    T: PyPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T: PyObjectPayload> ToPyObject for PyRef<T> {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}

impl ToPyObject for PyObjectRef {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self
    }
}

impl ToPyObject for &PyObject {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.to_owned()
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `ToPyObject`.
impl<T> ToPyObject for T
where
    T: PyPayload + Sized,
{
    #[inline(always)]
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyPayload::into_pyobject(self, vm)
    }
}

impl<T> ToPyResult for T
where
    T: ToPyObject,
{
    #[inline(always)]
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        Ok(self.to_pyobject(vm))
    }
}

impl<T> ToPyResult for PyResult<T>
where
    T: ToPyObject,
{
    #[inline(always)]
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        self.map(|res| T::to_pyobject(res, vm))
    }
}

pub trait PyObjectWrap
where
    Self: AsObject,
{
    fn into_object(self) -> PyObjectRef;
}

impl<T> From<T> for PyObjectRef
where
    T: PyObjectWrap,
{
    #[inline(always)]
    fn from(py_ref: T) -> Self {
        py_ref.into_object()
    }
}

#[derive(Debug)]
pub enum PyMethod {
    Function {
        target: PyObjectRef,
        func: PyObjectRef,
    },
    Attribute(PyObjectRef),
}

impl PyMethod {
    pub fn get(obj: PyObjectRef, name: pystr::PyStrRef, vm: &VirtualMachine) -> PyResult<Self> {
        let cls = obj.class();
        let getattro = cls.mro_find_map(|cls| cls.slots.getattro.load()).unwrap();
        if getattro as usize != object::PyBaseObject::getattro as usize {
            drop(cls);
            return obj.get_attr(name, vm).map(Self::Attribute);
        }

        let mut is_method = false;

        let cls_attr = match cls.get_attr(name.as_str()) {
            Some(descr) => {
                let descr_cls = descr.class();
                let descr_get = if descr_cls.slots.flags.has_feature(PyTypeFlags::METHOD_DESCR) {
                    is_method = true;
                    None
                } else {
                    let descr_get = descr_cls.mro_find_map(|cls| cls.slots.descr_get.load());
                    if let Some(descr_get) = descr_get {
                        if descr_cls
                            .mro_find_map(|cls| cls.slots.descr_set.load())
                            .is_some()
                        {
                            drop(descr_cls);
                            let cls = cls.into_owned().into();
                            return descr_get(descr, Some(obj), Some(cls), vm).map(Self::Attribute);
                        }
                    }
                    descr_get
                };
                drop(descr_cls);
                Some((descr, descr_get))
            }
            None => None,
        };

        if let Some(dict) = obj.dict() {
            if let Some(attr) = dict.get_item_opt(name.clone(), vm)? {
                return Ok(Self::Attribute(attr));
            }
        }

        if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                None if is_method => {
                    drop(cls);
                    Ok(Self::Function {
                        target: obj,
                        func: attr,
                    })
                }
                Some(descr_get) => {
                    let cls = cls.into_owned().into();
                    descr_get(attr, Some(obj), Some(cls), vm).map(Self::Attribute)
                }
                None => Ok(Self::Attribute(attr)),
            }
        } else if let Some(getter) = cls.get_attr("__getattr__") {
            drop(cls);
            vm.invoke(&getter, (obj, name)).map(Self::Attribute)
        } else {
            let exc = vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                cls.name(),
                name
            ));
            vm.set_attribute_error_context(&exc, obj.clone(), name);
            Err(exc)
        }
    }

    pub(crate) fn get_special(
        obj: PyObjectRef,
        name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Result<Self, PyObjectRef>> {
        let obj_cls = obj.class();
        let func = match obj_cls.get_attr(name) {
            Some(f) => f,
            None => {
                drop(obj_cls);
                return Ok(Err(obj));
            }
        };
        let meth = if func
            .class()
            .slots
            .flags
            .has_feature(PyTypeFlags::METHOD_DESCR)
        {
            drop(obj_cls);
            Self::Function { target: obj, func }
        } else {
            let obj_cls = obj_cls.into_owned().into();
            let attr = vm
                .call_get_descriptor_specific(func, Some(obj), Some(obj_cls))
                .unwrap_or_else(Ok)?;
            Self::Attribute(attr)
        };
        Ok(Ok(meth))
    }

    pub fn invoke(self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => (func, args.into_method_args(target, vm)),
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        vm.invoke(&func, args)
    }

    pub fn invoke_ref(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => {
                (func, args.into_method_args(target.clone(), vm))
            }
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        vm.invoke(func, args)
    }
}

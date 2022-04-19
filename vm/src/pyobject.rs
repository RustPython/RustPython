use crate::common::lock::PyRwLockReadGuard;
use crate::{
    builtins::{
        builtinfunc::{PyBuiltinFunction, PyBuiltinMethod, PyNativeFuncDef},
        bytes,
        getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetSet},
        object, pystr,
        pytype::PyAttributes,
        PyBaseException, PyBaseExceptionRef, PyDict, PyDictRef, PyEllipsis, PyFloat, PyFrozenSet,
        PyInt, PyIntRef, PyList, PyListRef, PyNone, PyNotImplemented, PyStr, PyTuple, PyTupleRef,
        PyType, PyTypeRef,
    },
    convert::TryFromObject,
    convert::{ToPyObject, ToPyResult},
    dictdatatype::Dict,
    exceptions,
    function::{IntoFuncArgs, IntoPyNativeFunc},
    pyclass::{PyClassImpl, StaticType},
    types::{PyTypeFlags, PyTypeSlots, TypeZoo},
    VirtualMachine,
    _pyobjectrc::{Py, PyObject, PyObjectRef, PyRef},
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::{any::Any, borrow::Borrow, fmt, ops::Deref};

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

#[derive(Debug, Clone)]
pub struct PyContext {
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub none: PyRef<PyNone>,
    pub empty_tuple: PyTupleRef,
    pub empty_frozenset: PyRef<PyFrozenSet>,
    pub ellipsis: PyRef<PyEllipsis>,
    pub not_implemented: PyRef<PyNotImplemented>,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyIntRef>,
    // there should only be exact objects of str in here, no non-strs and no subclasses
    pub(crate) string_cache: Dict<()>,
    pub(super) slot_new_wrapper: PyObjectRef,
}

// Basic objects:
impl PyContext {
    pub const INT_CACHE_POOL_MIN: i32 = -5;
    pub const INT_CACHE_POOL_MAX: i32 = 256;

    fn init() -> Self {
        flame_guard!("init PyContext");
        let types = TypeZoo::init();
        let exceptions = exceptions::ExceptionZoo::init();

        #[inline]
        fn create_object<T: PyObjectPayload + PyPayload>(payload: T, cls: &PyTypeRef) -> PyRef<T> {
            PyRef::new_ref(payload, cls.clone(), None)
        }

        let none = create_object(PyNone, PyNone::static_type());
        let ellipsis = create_object(PyEllipsis, PyEllipsis::static_type());
        let not_implemented = create_object(PyNotImplemented, PyNotImplemented::static_type());

        let int_cache_pool = (Self::INT_CACHE_POOL_MIN..=Self::INT_CACHE_POOL_MAX)
            .map(|v| PyRef::new_ref(PyInt::from(BigInt::from(v)), types.int_type.clone(), None))
            .collect();

        let true_value = create_object(PyInt::from(1), &types.bool_type);
        let false_value = create_object(PyInt::from(0), &types.bool_type);

        let empty_tuple = create_object(
            PyTuple::new_unchecked(Vec::new().into_boxed_slice()),
            &types.tuple_type,
        );
        let empty_frozenset =
            PyRef::new_ref(PyFrozenSet::default(), types.frozenset_type.clone(), None);

        let string_cache = Dict::default();

        let new_str = PyRef::new_ref(pystr::PyStr::from("__new__"), types.str_type.clone(), None);
        let slot_new_wrapper = create_object(
            PyNativeFuncDef::new(PyType::__new__.into_func(), new_str).into_function(),
            &types.builtin_function_or_method_type,
        )
        .into();

        let context = PyContext {
            true_value,
            false_value,
            none,
            empty_tuple,
            empty_frozenset,
            ellipsis,
            not_implemented,

            types,
            exceptions,
            int_cache_pool,
            string_cache,
            slot_new_wrapper,
        };
        TypeZoo::extend(&context);
        exceptions::ExceptionZoo::extend(&context);
        context
    }

    #[inline(always)]
    pub fn none(&self) -> PyObjectRef {
        self.none.clone().into()
    }

    #[inline(always)]
    pub fn ellipsis(&self) -> PyObjectRef {
        self.ellipsis.clone().into()
    }

    #[inline(always)]
    pub fn not_implemented(&self) -> PyObjectRef {
        self.not_implemented.clone().into()
    }

    // shortcuts for common type

    #[inline]
    pub fn new_int<T: Into<BigInt> + ToPrimitive>(&self, i: T) -> PyIntRef {
        if let Some(i) = i.to_i32() {
            if i >= Self::INT_CACHE_POOL_MIN && i <= Self::INT_CACHE_POOL_MAX {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyRef::new_ref(PyInt::from(i), self.types.int_type.clone(), None)
    }

    #[inline]
    pub fn new_bigint(&self, i: &BigInt) -> PyIntRef {
        if let Some(i) = i.to_i32() {
            if i >= Self::INT_CACHE_POOL_MIN && i <= Self::INT_CACHE_POOL_MAX {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyRef::new_ref(PyInt::from(i.clone()), self.types.int_type.clone(), None)
    }

    #[inline]
    pub fn new_float(&self, value: f64) -> PyRef<PyFloat> {
        PyRef::new_ref(PyFloat::from(value), self.types.float_type.clone(), None)
    }

    #[inline]
    pub fn new_str(&self, s: impl Into<pystr::PyStr>) -> PyRef<PyStr> {
        pystr::PyStr::new_ref(s, self)
    }

    #[inline]
    pub fn new_bytes(&self, data: Vec<u8>) -> PyRef<bytes::PyBytes> {
        bytes::PyBytes::new_ref(data, self)
    }

    #[inline(always)]
    pub fn new_bool(&self, b: bool) -> PyIntRef {
        let value = if b {
            &self.true_value
        } else {
            &self.false_value
        };
        value.clone()
    }

    #[inline(always)]
    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyTupleRef {
        PyTuple::new_ref(elements, self)
    }

    #[inline(always)]
    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyListRef {
        PyList::new_ref(elements, self)
    }

    #[inline(always)]
    pub fn new_dict(&self) -> PyDictRef {
        PyDict::new_ref(self)
    }

    pub fn new_class(
        &self,
        module: Option<&str>,
        name: &str,
        base: &PyTypeRef,
        slots: PyTypeSlots,
    ) -> PyTypeRef {
        let mut attrs = PyAttributes::default();
        if let Some(module) = module {
            attrs.insert("__module__".to_string(), self.new_str(module).into());
        };
        PyType::new_ref(
            name,
            vec![base.clone()],
            attrs,
            slots,
            self.types.type_type.clone(),
        )
        .unwrap()
    }

    pub fn new_exception_type(
        &self,
        module: &str,
        name: &str,
        bases: Option<Vec<PyTypeRef>>,
    ) -> PyTypeRef {
        let bases = if let Some(bases) = bases {
            bases
        } else {
            vec![self.exceptions.exception_type.clone()]
        };
        let mut attrs = PyAttributes::default();
        attrs.insert("__module__".to_owned(), self.new_str(module).into());

        PyType::new_ref(
            name,
            bases,
            attrs,
            PyBaseException::make_slots(),
            self.types.type_type.clone(),
        )
        .unwrap()
    }

    #[inline]
    pub fn make_funcdef<F, FKind>(&self, name: impl Into<PyStr>, f: F) -> PyNativeFuncDef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyNativeFuncDef::new(f.into_func(), PyStr::new_ref(name, self))
    }

    // #[deprecated]
    pub fn new_function<F, FKind>(&self, name: impl Into<PyStr>, f: F) -> PyRef<PyBuiltinFunction>
    where
        F: IntoPyNativeFunc<FKind>,
    {
        self.make_funcdef(name, f).build_function(self)
    }

    pub fn new_method<F, FKind>(
        &self,
        name: impl Into<PyStr>,
        class: PyTypeRef,
        f: F,
    ) -> PyRef<PyBuiltinMethod>
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyBuiltinMethod::new_ref(name, class, f, self)
    }

    pub fn new_readonly_getset<F, T>(
        &self,
        name: impl Into<String>,
        class: PyTypeRef,
        f: F,
    ) -> PyRef<PyGetSet>
    where
        F: IntoPyGetterFunc<T>,
    {
        PyRef::new_ref(
            PyGetSet::new(name.into(), class).with_get(f),
            self.types.getset_type.clone(),
            None,
        )
    }

    pub fn new_getset<G, S, T, U>(
        &self,
        name: impl Into<String>,
        class: PyTypeRef,
        g: G,
        s: S,
    ) -> PyRef<PyGetSet>
    where
        G: IntoPyGetterFunc<T>,
        S: IntoPySetterFunc<U>,
    {
        PyRef::new_ref(
            PyGetSet::new(name.into(), class).with_get(g).with_set(s),
            self.types.getset_type.clone(),
            None,
        )
    }

    pub fn new_base_object(&self, class: PyTypeRef, dict: Option<PyDictRef>) -> PyObjectRef {
        debug_assert_eq!(
            class.slots.flags.contains(PyTypeFlags::HAS_DICT),
            dict.is_some()
        );
        PyRef::new_ref(object::PyBaseObject, class, dict).into()
    }
}

impl Default for PyContext {
    fn default() -> Self {
        rustpython_common::static_cell! {
            static CONTEXT: PyContext;
        }
        CONTEXT.get_or_init(Self::init).clone()
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

cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        pub trait PyThreadingConstraint: Send + Sync {}
        impl<T: Send + Sync> PyThreadingConstraint for T {}
    } else {
        pub trait PyThreadingConstraint {}
        impl<T> PyThreadingConstraint for T {}
    }
}

pub trait PyPayload: fmt::Debug + PyThreadingConstraint + Sized + 'static {
    fn class(vm: &VirtualMachine) -> &PyTypeRef;

    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_ref(vm).into()
    }

    #[inline(always)]
    fn special_retrieve(_vm: &VirtualMachine, _obj: &PyObject) -> Option<PyResult<PyRef<Self>>> {
        None
    }

    #[inline]
    fn _into_ref(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyRef<Self> {
        let dict = if cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }

    #[inline]
    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        let cls = Self::class(vm);
        self._into_ref(cls.clone(), vm)
    }

    #[cold]
    fn _into_ref_with_type_error(
        vm: &VirtualMachine,
        cls: &PyTypeRef,
        exact_class: &PyTypeRef,
    ) -> PyBaseExceptionRef {
        vm.new_type_error(format!(
            "'{}' is not a subtype of '{}'",
            &cls.name(),
            exact_class.name()
        ))
    }

    #[inline]
    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>> {
        let exact_class = Self::class(vm);
        if cls.fast_issubclass(exact_class) {
            Ok(self._into_ref(cls, vm))
        } else {
            Err(Self::_into_ref_with_type_error(vm, &cls, exact_class))
        }
    }

    #[inline]
    fn into_pyresult_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult {
        self.into_ref_with_type(vm, cls).to_pyresult(vm)
    }
}

pub trait PyObjectPayload: Any + fmt::Debug + PyThreadingConstraint + 'static {}

impl<T: PyPayload + 'static> PyObjectPayload for T {}

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

use crate::common::{
    lock::{PyRwLock, PyRwLockReadGuard},
    rc::PyRc,
    static_cell,
};
pub use crate::pyobjectrc::{
    PyObject, PyObjectRef, PyObjectView, PyObjectWeak, PyObjectWrap, PyRef, PyWeakRef,
};
use crate::{
    builtins::{
        builtinfunc::{PyBuiltinFunction, PyBuiltinMethod, PyNativeFuncDef},
        bytes,
        getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetSet},
        object, pystr, PyBaseException, PyBaseExceptionRef, PyBoundMethod, PyDict, PyDictRef,
        PyEllipsis, PyFloat, PyFrozenSet, PyGenericAlias, PyInt, PyIntRef, PyList, PyListRef,
        PyNone, PyNotImplemented, PyStr, PyTuple, PyTupleRef, PyType, PyTypeRef,
    },
    dictdatatype::Dict,
    exceptions,
    function::{IntoFuncArgs, IntoPyNativeFunc, IntoPyObject, IntoPyResult},
    protocol::PyMapping,
    types::TypeZoo,
    types::{PyTypeFlags, PyTypeSlots},
    VirtualMachine,
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
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

/// For attributes we do not use a dict, but a hashmap. This is probably
/// faster, unordered, and only supports strings as keys.
/// TODO: class attributes should maintain insertion order (use IndexMap here)
pub type PyAttributes = HashMap<String, PyObjectRef, ahash::RandomState>;

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
    slot_new_wrapper: PyObjectRef,
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
        fn create_object<T: PyObjectPayload + PyValue>(payload: T, cls: &PyTypeRef) -> PyRef<T> {
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
    pub fn new() -> Self {
        rustpython_common::static_cell! {
            static CONTEXT: PyContext;
        }
        CONTEXT.get_or_init(Self::init).clone()
    }

    pub fn none(&self) -> PyObjectRef {
        self.none.clone().into()
    }

    pub fn ellipsis(&self) -> PyObjectRef {
        self.ellipsis.clone().into()
    }

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

    pub fn new_float(&self, value: f64) -> PyRef<PyFloat> {
        PyRef::new_ref(PyFloat::from(value), self.types.float_type.clone(), None)
    }

    pub fn new_str(&self, s: impl Into<pystr::PyStr>) -> PyRef<PyStr> {
        pystr::PyStr::new_ref(s, self)
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyRef<bytes::PyBytes> {
        bytes::PyBytes::new_ref(data, self)
    }

    #[inline]
    pub fn new_bool(&self, b: bool) -> PyIntRef {
        let value = if b {
            &self.true_value
        } else {
            &self.false_value
        };
        value.clone()
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyTupleRef {
        PyTuple::new_ref(elements, self)
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyListRef {
        PyList::new_ref(elements, self)
    }

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
        attrs.insert("__module__".to_string(), self.new_str(module).into());

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
        PyContext::new()
    }
}

pub(crate) fn try_value_from_borrowed_object<T, F, R>(
    vm: &VirtualMachine,
    obj: &PyObject,
    f: F,
) -> PyResult<R>
where
    T: PyValue,
    F: Fn(&T) -> PyResult<R>,
{
    let class = T::class(vm);
    let special;
    let py_ref = if obj.isinstance(class) {
        obj.downcast_ref()
            .ok_or_else(|| pyref_payload_error(vm, class, obj))?
    } else {
        special = T::special_retrieve(vm, obj)
            .unwrap_or_else(|| Err(pyref_type_error(vm, class, obj)))?;
        &special
    };
    f(py_ref)
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyValue,
{
    #[inline]
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = T::class(vm);
        if obj.isinstance(class) {
            obj.downcast()
                .map_err(|obj| pyref_payload_error(vm, class, obj))
        } else {
            T::special_retrieve(vm, &obj).unwrap_or_else(|| Err(pyref_type_error(vm, class, obj)))
        }
    }
}
// the impl Borrow allows to pass PyObjectRef or &PyObject
fn pyref_payload_error(
    vm: &VirtualMachine,
    class: &PyTypeRef,
    obj: impl std::borrow::Borrow<PyObject>,
) -> PyBaseExceptionRef {
    vm.new_runtime_error(format!(
        "Unexpected payload '{}' for type '{}'",
        &*class.name(),
        &*obj.borrow().class().name(),
    ))
}

pub(crate) fn pyref_type_error(
    vm: &VirtualMachine,
    class: &PyTypeRef,
    obj: impl std::borrow::Borrow<PyObject>,
) -> PyBaseExceptionRef {
    let expected_type = &*class.name();
    let actual_class = obj.borrow().class();
    let actual_type = &*actual_class.name();
    vm.new_type_error(format!(
        "Expected type '{}', not '{}'",
        expected_type, actual_type,
    ))
}

impl<T: fmt::Display> fmt::Display for PyRef<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}
impl<T: fmt::Display> fmt::Display for PyObjectView<T>
where
    T: PyObjectPayload + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

pub struct PyRefExact<T: PyObjectPayload> {
    obj: PyRef<T>,
}
impl<T: PyValue> TryFromObject for PyRefExact<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let target_cls = T::class(vm);
        let cls = obj.class();
        if cls.is(target_cls) {
            drop(cls);
            let obj = obj
                .downcast()
                .map_err(|obj| pyref_payload_error(vm, target_cls, obj))?;
            Ok(Self { obj })
        } else if cls.issubclass(target_cls) {
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
impl<T: PyValue> Deref for PyRefExact<T> {
    type Target = PyRef<T>;
    fn deref(&self) -> &PyRef<T> {
        &self.obj
    }
}
impl<T: PyValue> IntoPyObject for PyRefExact<T> {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.obj.into()
    }
}

pub trait IdProtocol {
    fn get_id(&self) -> usize;
    fn is<T>(&self, other: &T) -> bool
    where
        T: IdProtocol,
    {
        self.get_id() == other.get_id()
    }
}

impl<T: ?Sized> IdProtocol for PyRc<T> {
    fn get_id(&self) -> usize {
        &**self as *const T as *const () as usize
    }
}

impl<T: PyObjectPayload> IdProtocol for PyRef<T> {
    fn get_id(&self) -> usize {
        self.as_object().get_id()
    }
}

impl<T: PyObjectPayload> IdProtocol for PyObjectView<T> {
    fn get_id(&self) -> usize {
        self.as_object().get_id()
    }
}

impl<'a, T: PyObjectPayload> IdProtocol for PyLease<'a, T> {
    fn get_id(&self) -> usize {
        self.inner.get_id()
    }
}

impl<T: IdProtocol> IdProtocol for &'_ T {
    fn get_id(&self) -> usize {
        (&**self).get_id()
    }
}

/// A borrow of a reference to a Python object. This avoids having clone the `PyRef<T>`/
/// `PyObjectRef`, which isn't that cheap as that increments the atomic reference counter.
pub struct PyLease<'a, T: PyObjectPayload> {
    inner: PyRwLockReadGuard<'a, PyRef<T>>,
}

impl<'a, T: PyObjectPayload + PyValue> PyLease<'a, T> {
    // Associated function on purpose, because of deref
    #[allow(clippy::wrong_self_convention)]
    pub fn into_pyref(zelf: Self) -> PyRef<T> {
        zelf.inner.clone()
    }
}

impl<'a, T: PyObjectPayload + PyValue> Deref for PyLease<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> fmt::Display for PyLease<'a, T>
where
    T: PyValue + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

pub trait TypeProtocol {
    fn class(&self) -> PyLease<'_, PyType>;

    fn clone_class(&self) -> PyTypeRef {
        PyLease::into_pyref(self.class())
    }

    fn get_class_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        self.class().get_attr(attr_name)
    }

    fn has_class_attr(&self, attr_name: &str) -> bool {
        self.class().has_attr(attr_name)
    }

    /// Determines if `obj` actually an instance of `cls`, this doesn't call __instancecheck__, so only
    /// use this if `cls` is known to have not overridden the base __instancecheck__ magic method.
    #[inline]
    fn isinstance(&self, cls: &PyObjectView<PyType>) -> bool {
        self.class().issubclass(cls)
    }
}

impl TypeProtocol for PyObjectRef {
    fn class(&self) -> PyLease<'_, PyType> {
        PyLease {
            inner: self.class_lock().read(),
        }
    }
}

impl TypeProtocol for PyObject {
    fn class(&self) -> PyLease<'_, PyType> {
        PyLease {
            inner: self.class_lock().read(),
        }
    }
}

impl<T: PyObjectPayload> TypeProtocol for PyObjectView<T> {
    fn class(&self) -> PyLease<'_, PyType> {
        self.as_object().class()
    }
}

impl<T: PyObjectPayload> TypeProtocol for PyRef<T> {
    fn class(&self) -> PyLease<'_, PyType> {
        self.as_object().class()
    }
}

impl<T: TypeProtocol> TypeProtocol for &'_ T {
    fn class(&self) -> PyLease<'_, PyType> {
        (&**self).class()
    }
}

/// The python item protocol. Mostly applies to dictionaries.
/// Allows getting, setting and deletion of keys-value pairs.
pub trait ItemProtocol<T>
where
    T: IntoPyObject + ?Sized,
{
    fn get_item(&self, key: T, vm: &VirtualMachine) -> PyResult;
    fn set_item(&self, key: T, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>;
    fn del_item(&self, key: T, vm: &VirtualMachine) -> PyResult<()>;
}

impl<T> ItemProtocol<T> for PyObject
where
    T: IntoPyObject,
{
    fn get_item(&self, key: T, vm: &VirtualMachine) -> PyResult {
        if let Ok(mapping) = PyMapping::try_from_object(vm, self.to_owned()) {
            if let Some(getitem) = mapping.methods(vm).subscript {
                return getitem(self.to_owned(), key.into_pyobject(vm), vm);
            }
        }

        match vm.get_special_method(self.to_owned(), "__getitem__")? {
            Ok(special_method) => return special_method.invoke((key,), vm),
            Err(obj) => {
                if obj.class().issubclass(&vm.ctx.types.type_type) {
                    if obj.is(&vm.ctx.types.type_type) {
                        return PyGenericAlias::new(obj.clone_class(), key.into_pyobject(vm), vm)
                            .into_pyresult(vm);
                    }

                    if let Some(class_getitem) = vm.get_attribute_opt(obj, "__class_getitem__")? {
                        return vm.invoke(&class_getitem, (key,));
                    }
                }
            }
        }
        Err(vm.new_type_error(format!(
            "'{}' object is not subscriptable",
            self.class().name()
        )))
    }

    fn set_item(&self, key: T, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(mapping) = PyMapping::try_from_object(vm, self.to_owned()) {
            if let Some(setitem) = mapping.methods(vm).ass_subscript {
                return setitem(self.to_owned(), key.into_pyobject(vm), Some(value), vm);
            }
        }

        vm.get_special_method(self.to_owned(), "__setitem__")?
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' does not support item assignment",
                    obj.class().name()
                ))
            })?
            .invoke((key, value), vm)?;
        Ok(())
    }

    fn del_item(&self, key: T, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(mapping) = PyMapping::try_from_object(vm, self.to_owned()) {
            if let Some(setitem) = mapping.methods(vm).ass_subscript {
                return setitem(self.to_owned(), key.into_pyobject(vm), None, vm);
            }
        }

        vm.get_special_method(self.to_owned(), "__delitem__")?
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' does not support item deletion",
                    obj.class().name()
                ))
            })?
            .invoke((key,), vm)?;
        Ok(())
    }
}

impl TryFromObject for PyObjectRef {
    #[inline]
    fn try_from_object(_vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(obj)
    }
}

impl<T: TryFromObject> TryFromObject for Option<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.is_none(&obj) {
            Ok(None)
        } else {
            T::try_from_object(vm, obj).map(Some)
        }
    }
}

/// Implemented by any type that can be created from a Python object.
///
/// Any type that implements `TryFromObject` is automatically `FromArgs`, and
/// so can be accepted as a argument to a built-in function.
pub trait TryFromObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
}

/// Rust-side only version of TryFromObject to reduce unnecessary Rc::clone
impl<T: TryFromBorrowedObject> TryFromObject for T {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        TryFromBorrowedObject::try_from_borrowed_object(vm, &obj)
    }
}

pub trait TryFromBorrowedObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self>;
}

impl PyObjectRef {
    pub fn try_into_value<T>(self, vm: &VirtualMachine) -> PyResult<T>
    where
        T: TryFromObject,
    {
        T::try_from_object(vm, self)
    }

    pub fn try_borrow_to_object<T>(&self, vm: &VirtualMachine) -> PyResult<T>
    where
        T: TryFromBorrowedObject,
    {
        T::try_from_borrowed_object(vm, self)
    }
}

/// Marks a type that has the exact same layout as PyObjectRef, e.g. a type that is
/// `repr(transparent)` over PyObjectRef.
///
/// # Safety
/// Can only be implemented for types that are `repr(transparent)` over a PyObjectRef `obj`,
/// and logically valid so long as `check(vm, obj)` returns `Ok(())`
pub unsafe trait TransmuteFromObject: Sized {
    fn check(vm: &VirtualMachine, obj: &PyObject) -> PyResult<()>;
}

unsafe impl<T: PyValue> TransmuteFromObject for PyRef<T> {
    fn check(vm: &VirtualMachine, obj: &PyObject) -> PyResult<()> {
        let class = T::class(vm);
        if obj.isinstance(class) {
            if obj.payload_is::<T>() {
                Ok(())
            } else {
                Err(pyref_payload_error(vm, class, obj))
            }
        } else {
            Err(pyref_type_error(vm, class, obj))
        }
    }
}

pub trait IntoPyRef<T: PyObjectPayload> {
    fn into_pyref(self, vm: &VirtualMachine) -> PyRef<T>;
}

impl<T, P> IntoPyRef<P> for T
where
    P: PyValue + IntoPyObject + From<T>,
{
    fn into_pyref(self, vm: &VirtualMachine) -> PyRef<P> {
        P::from(self).into_ref(vm)
    }
}

impl<T: PyObjectPayload> IntoPyObject for PyRef<T> {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into()
    }
}

impl IntoPyObject for PyObjectRef {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self
    }
}

impl IntoPyObject for &PyObject {
    #[inline]
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.to_owned()
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyValue + Sized,
{
    #[inline]
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyValue::into_object(self, vm)
    }
}

impl<T> IntoPyResult for T
where
    T: IntoPyObject,
{
    fn into_pyresult(self, vm: &VirtualMachine) -> PyResult {
        Ok(self.into_pyobject(vm))
    }
}

impl<T> IntoPyResult for PyResult<T>
where
    T: IntoPyObject,
{
    fn into_pyresult(self, vm: &VirtualMachine) -> PyResult {
        self.map(|res| T::into_pyobject(res, vm))
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

pub trait PyValue: fmt::Debug + PyThreadingConstraint + Sized + 'static {
    fn class(vm: &VirtualMachine) -> &PyTypeRef;

    #[inline]
    fn into_object(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_ref(vm).into()
    }

    #[inline(always)]
    fn special_retrieve(_vm: &VirtualMachine, _obj: &PyObject) -> Option<PyResult<PyRef<Self>>> {
        None
    }

    fn _into_ref(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyRef<Self> {
        let dict = if cls.slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }

    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        let cls = Self::class(vm);
        self._into_ref(cls.clone(), vm)
    }

    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>> {
        let exact_class = Self::class(vm);
        if cls.issubclass(exact_class) {
            Ok(self._into_ref(cls, vm))
        } else {
            Err(vm.new_type_error(format!(
                "'{}' is not a subtype of '{}'",
                &cls.name(),
                exact_class.name()
            )))
        }
    }

    fn into_pyresult_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult {
        self.into_ref_with_type(vm, cls).into_pyresult(vm)
    }
}

pub trait PyObjectPayload: Any + fmt::Debug + PyThreadingConstraint + 'static {}

impl<T: PyValue + 'static> PyObjectPayload for T {}

pub trait PyClassDef {
    const NAME: &'static str;
    const MODULE_NAME: Option<&'static str>;
    const TP_NAME: &'static str;
    const DOC: Option<&'static str> = None;
}

pub trait StaticType {
    // Ideally, saving PyType is better than PyTypeRef
    fn static_cell() -> &'static static_cell::StaticCell<PyTypeRef>;
    fn static_metaclass() -> &'static PyTypeRef {
        crate::builtins::pytype::PyType::static_type()
    }
    fn static_baseclass() -> &'static PyTypeRef {
        crate::builtins::object::PyBaseObject::static_type()
    }
    fn static_type() -> &'static PyTypeRef {
        Self::static_cell()
            .get()
            .expect("static type has not been initialized")
    }
    fn init_manually(typ: PyTypeRef) -> &'static PyTypeRef {
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization from init_manually"));
        cell.get().unwrap()
    }
    fn init_bare_type() -> &'static PyTypeRef
    where
        Self: PyClassImpl,
    {
        let typ = Self::create_bare_type();
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization of {}", Self::NAME));
        cell.get().unwrap()
    }
    fn create_bare_type() -> PyTypeRef
    where
        Self: PyClassImpl,
    {
        PyType::new_ref(
            Self::NAME,
            vec![Self::static_baseclass().clone()],
            Default::default(),
            Self::make_slots(),
            Self::static_metaclass().clone(),
        )
        .unwrap()
    }
}

impl<T> PyClassDef for PyRef<T>
where
    T: PyObjectPayload + PyClassDef,
{
    const NAME: &'static str = T::NAME;
    const MODULE_NAME: Option<&'static str> = T::MODULE_NAME;
    const TP_NAME: &'static str = T::TP_NAME;
    const DOC: Option<&'static str> = T::DOC;
}

pub trait PyClassImpl: PyClassDef {
    const TP_FLAGS: PyTypeFlags = PyTypeFlags::DEFAULT;

    fn impl_extend_class(ctx: &PyContext, class: &PyTypeRef);

    fn extend_class(ctx: &PyContext, class: &PyTypeRef) {
        #[cfg(debug_assertions)]
        {
            assert!(class.slots.flags.is_created_with_flags());
        }
        if Self::TP_FLAGS.has_feature(PyTypeFlags::HAS_DICT) {
            class.set_str_attr(
                "__dict__",
                ctx.new_getset(
                    "__dict__",
                    class.clone(),
                    object::object_get_dict,
                    object::object_set_dict,
                ),
            );
        }
        Self::impl_extend_class(ctx, class);
        if let Some(doc) = Self::DOC {
            class.set_str_attr("__doc__", ctx.new_str(doc));
        }
        if let Some(module_name) = Self::MODULE_NAME {
            class.set_str_attr("__module__", ctx.new_str(module_name));
        }
        if class.slots.new.load().is_some() {
            let bound: PyObjectRef =
                PyBoundMethod::new_ref(class.clone().into(), ctx.slot_new_wrapper.clone(), ctx)
                    .into();
            class.set_str_attr("__new__", bound);
        }
    }

    fn make_class(ctx: &PyContext) -> PyTypeRef
    where
        Self: StaticType,
    {
        Self::static_cell()
            .get_or_init(|| {
                let typ = Self::create_bare_type();
                Self::extend_class(ctx, &typ);
                typ
            })
            .clone()
    }

    fn extend_slots(slots: &mut PyTypeSlots);

    fn make_slots() -> PyTypeSlots {
        let mut slots = PyTypeSlots {
            flags: Self::TP_FLAGS,
            name: PyRwLock::new(Some(Self::TP_NAME.to_owned())),
            doc: Self::DOC,
            ..Default::default()
        };
        Self::extend_slots(&mut slots);
        slots
    }
}

#[pyimpl]
pub trait PyStructSequence: StaticType + PyClassImpl + Sized + 'static {
    const FIELD_NAMES: &'static [&'static str];

    fn into_tuple(self, vm: &VirtualMachine) -> PyTuple;

    fn into_struct_sequence(self, vm: &VirtualMachine) -> PyTupleRef {
        self.into_tuple(vm)
            .into_ref_with_type(vm, Self::static_type().clone())
            .unwrap()
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyResult<String> {
        let format_field = |(value, name): (&PyObjectRef, _)| {
            let s = value.repr(vm)?;
            Ok(format!("{}={}", name, s))
        };
        let (body, suffix) =
            if let Some(_guard) = rustpython_vm::vm::ReprGuard::enter(vm, zelf.as_object()) {
                if Self::FIELD_NAMES.len() == 1 {
                    let value = zelf.as_slice().first().unwrap();
                    let formatted = format_field((value, Self::FIELD_NAMES[0]))?;
                    (formatted, ",")
                } else {
                    let fields: PyResult<Vec<_>> = zelf
                        .as_slice()
                        .iter()
                        .zip(Self::FIELD_NAMES.iter().copied())
                        .map(format_field)
                        .collect();
                    (fields?.join(", "), "")
                }
            } else {
                (String::new(), "...")
            };
        Ok(format!("{}({}{})", Self::TP_NAME, body, suffix))
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((
            zelf.clone_class(),
            (vm.ctx.new_tuple(zelf.as_slice().to_vec()),),
        ))
    }

    #[extend_class]
    fn extend_pyclass(ctx: &PyContext, class: &PyTypeRef) {
        for (i, &name) in Self::FIELD_NAMES.iter().enumerate() {
            // cast i to a u8 so there's less to store in the getter closure.
            // Hopefully there's not struct sequences with >=256 elements :P
            let i = i as u8;
            class.set_str_attr(
                name,
                ctx.new_readonly_getset(name, class.clone(), move |zelf: &PyTuple| {
                    zelf.fast_getitem(i.into())
                }),
            );
        }
    }
}

#[derive(result_like::OptionLike)]
pub enum PyArithmeticValue<T> {
    Implemented(T),
    NotImplemented,
}

impl PyArithmeticValue<PyObjectRef> {
    pub fn from_object(vm: &VirtualMachine, obj: PyObjectRef) -> Self {
        if obj.is(&vm.ctx.not_implemented) {
            Self::NotImplemented
        } else {
            Self::Implemented(obj)
        }
    }
}

impl<T: TryFromObject> TryFromObject for PyArithmeticValue<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyArithmeticValue::from_object(vm, obj)
            .map(|x| T::try_from_object(vm, x))
            .transpose()
    }
}

impl<T> IntoPyObject for PyArithmeticValue<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            PyArithmeticValue::Implemented(v) => v.into_pyobject(vm),
            PyArithmeticValue::NotImplemented => vm.ctx.not_implemented(),
        }
    }
}

pub type PyComparisonValue = PyArithmeticValue<bool>;

#[derive(Clone)]
pub struct PySequence<T = PyObjectRef>(Vec<T>);

impl<T> PySequence<T> {
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}
impl<T: TryFromObject> TryFromObject for PySequence<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        vm.extract_elements(&obj).map(Self)
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
                            let cls = PyLease::into_pyref(cls).into();
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
            if let Some(attr) = dict.get_item_option(name.clone(), vm)? {
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
                    let cls = PyLease::into_pyref(cls).into();
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
            let obj_cls = PyLease::into_pyref(obj_cls).into();
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

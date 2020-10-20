use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use crate::builtins::builtinfunc::PyNativeFuncDef;
use crate::builtins::bytearray;
use crate::builtins::bytes;
use crate::builtins::code;
use crate::builtins::code::PyCodeRef;
use crate::builtins::complex::PyComplex;
use crate::builtins::dict::{PyDict, PyDictRef};
use crate::builtins::float::PyFloat;
use crate::builtins::function::{PyBoundMethod, PyFunction};
use crate::builtins::getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetSet};
use crate::builtins::int::{PyInt, PyIntRef};
use crate::builtins::iter::PySequenceIterator;
use crate::builtins::list::PyList;
use crate::builtins::namespace::PyNamespace;
use crate::builtins::object;
use crate::builtins::pystr;
use crate::builtins::pytype::{self, PyType, PyTypeRef};
use crate::builtins::set;
use crate::builtins::singletons::{PyNone, PyNoneRef, PyNotImplemented, PyNotImplementedRef};
use crate::builtins::slice::PyEllipsis;
use crate::builtins::staticmethod::PyStaticMethod;
use crate::builtins::tuple::{PyTuple, PyTupleRef};
use crate::bytecode;
pub use crate::common::borrow::BorrowValue;
use crate::common::lock::{PyRwLock, PyRwLockReadGuard};
use crate::common::rc::PyRc;
use crate::common::static_cell;
use crate::exceptions::{self, PyBaseExceptionRef};
use crate::function::{IntoFuncArgs, IntoPyNativeFunc};
use crate::iterator;
pub use crate::pyobjectrc::{PyObjectRc, PyObjectWeak};
use crate::scope::Scope;
use crate::slots::{PyTpFlags, PyTypeSlots};
use crate::types::{create_type_with_slots, TypeZoo};
use crate::vm::VirtualMachine;

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

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
pub type PyObjectRef = PyObjectRc<dyn PyObjectPayload>;

/// Use this type for functions which return a python object or an exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyBaseExceptionRef>; // A valid value, or an exception

/// For attributes we do not use a dict, but a hashmap. This is probably
/// faster, unordered, and only supports strings as keys.
/// TODO: class attributes should maintain insertion order (use IndexMap here)
pub type PyAttributes = HashMap<String, PyObjectRef>;

impl fmt::Display for PyObject<dyn PyObjectPayload> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(PyType { ref name, .. }) = self.payload::<PyType>() {
            let type_name = self.class().name.clone();
            // We don't have access to a vm, so just assume that if its parent's name
            // is type, it's a type
            if type_name == "type" {
                return write!(f, "type object '{}'", name);
            } else {
                return write!(f, "'{}' object", type_name);
            }
        }

        write!(f, "'{}' object", self.class().name)
    }
}

#[derive(Debug, Clone)]
pub struct PyContext {
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub none: PyNoneRef,
    pub empty_tuple: PyTupleRef,
    pub ellipsis: PyRef<PyEllipsis>,
    pub not_implemented: PyNotImplementedRef,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyIntRef>,
    tp_new_wrapper: PyObjectRef,
}

// Basic objects:
impl PyContext {
    pub const INT_CACHE_POOL_MIN: i32 = -5;
    pub const INT_CACHE_POOL_MAX: i32 = 256;

    pub fn new() -> Self {
        use rustpython_common::static_cells;
        static_cells! {
            static CONTEXT: PyContext;
        }
        static_cell::get_or_init(&CONTEXT, || {
            flame_guard!("init PyContext");
            let types = TypeZoo::init();
            let exceptions = exceptions::ExceptionZoo::init();

            fn create_object<T: PyObjectPayload + PyValue>(
                payload: T,
                cls: &PyTypeRef,
            ) -> PyRef<T> {
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
                PyTuple::_new(Vec::new().into_boxed_slice()),
                &types.tuple_type,
            );

            let tp_new_wrapper = create_object(
                PyNativeFuncDef::from(pytype::tp_new_wrapper.into_func()).into_function(),
                &types.builtin_function_or_method_type,
            )
            .into_object();

            let context = PyContext {
                true_value,
                false_value,
                not_implemented,
                none,
                empty_tuple,
                ellipsis,

                types,
                exceptions,
                int_cache_pool,
                tp_new_wrapper,
            };
            TypeZoo::extend(&context);
            exceptions::ExceptionZoo::extend(&context);
            context
        })
        .clone()
    }

    pub fn none(&self) -> PyObjectRef {
        self.none.clone().into_object()
    }

    pub fn ellipsis(&self) -> PyObjectRef {
        self.ellipsis.clone().into_object()
    }

    pub fn not_implemented(&self) -> PyObjectRef {
        self.not_implemented.clone().into_object()
    }

    #[inline]
    pub fn new_int<T: Into<BigInt> + ToPrimitive>(&self, i: T) -> PyObjectRef {
        if let Some(i) = i.to_i32() {
            if i >= Self::INT_CACHE_POOL_MIN && i <= Self::INT_CACHE_POOL_MAX {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].as_object().clone();
            }
        }
        PyObject::new(PyInt::from(i), self.types.int_type.clone(), None)
    }

    #[inline]
    pub fn new_bigint(&self, i: &BigInt) -> PyObjectRef {
        if let Some(i) = i.to_i32() {
            if i >= Self::INT_CACHE_POOL_MIN && i <= Self::INT_CACHE_POOL_MAX {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].as_object().clone();
            }
        }
        PyObject::new(PyInt::from(i.clone()), self.types.int_type.clone(), None)
    }

    pub fn new_float(&self, value: f64) -> PyObjectRef {
        PyObject::new(PyFloat::from(value), self.types.float_type.clone(), None)
    }

    pub fn new_complex(&self, value: Complex64) -> PyObjectRef {
        PyObject::new(
            PyComplex::from(value),
            self.types.complex_type.clone(),
            None,
        )
    }

    pub fn new_str<S>(&self, s: S) -> PyObjectRef
    where
        S: Into<pystr::PyStr>,
    {
        PyObject::new(s.into(), self.types.str_type.clone(), None)
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            bytes::PyBytes::from(data),
            self.types.bytes_type.clone(),
            None,
        )
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            bytearray::PyByteArray::from(data),
            self.types.bytearray_type.clone(),
            None,
        )
    }

    #[inline]
    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        let value = if b {
            &self.true_value
        } else {
            &self.false_value
        };
        value.clone().into_object()
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyTupleRef::with_elements(elements, self).into_object()
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyList::from(elements), self.types.list_type.clone(), None)
    }

    pub fn new_set(&self) -> set::PySetRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the set code itself.
        PyRef::new_ref(set::PySet::default(), self.types.set_type.clone(), None)
    }

    pub fn new_dict(&self) -> PyDictRef {
        PyRef::new_ref(PyDict::default(), self.types.dict_type.clone(), None)
    }

    pub fn new_class(&self, name: &str, base: &PyTypeRef, slots: PyTypeSlots) -> PyTypeRef {
        create_type_with_slots(name, &self.types.type_type, base, slots)
    }

    pub fn new_namespace(&self) -> PyObjectRef {
        PyObject::new(
            PyNamespace,
            self.types.namespace_type.clone(),
            Some(self.new_dict()),
        )
    }

    pub fn new_function<F, FKind>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyNativeFuncDef::from(f.into_func()).build_function(self)
    }

    pub(crate) fn new_stringref(&self, s: String) -> pystr::PyStrRef {
        PyRef::new_ref(pystr::PyStr::from(s), self.types.str_type.clone(), None)
    }

    pub fn new_function_named<F, FKind>(&self, f: F, name: String) -> PyNativeFuncDef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        let mut f = PyNativeFuncDef::from(f.into_func());
        f.name = Some(self.new_stringref(name));
        f
    }

    pub fn new_method<F, FKind>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyNativeFuncDef::from(f.into_func()).build_method(self)
    }

    pub fn new_classmethod<F, FKind>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyNativeFuncDef::from(f.into_func()).build_classmethod(self)
    }
    pub fn new_staticmethod<F, FKind>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<FKind>,
    {
        PyObject::new(
            PyStaticMethod::from(self.new_method(f)),
            self.types.staticmethod_type.clone(),
            None,
        )
    }

    pub fn new_readonly_getset<F, T>(&self, name: impl Into<String>, f: F) -> PyObjectRef
    where
        F: IntoPyGetterFunc<T>,
    {
        PyObject::new(
            PyGetSet::with_get(name.into(), f),
            self.types.getset_type.clone(),
            None,
        )
    }

    pub fn new_getset<G, S, T, U>(&self, name: impl Into<String>, g: G, s: S) -> PyObjectRef
    where
        G: IntoPyGetterFunc<T>,
        S: IntoPySetterFunc<U>,
    {
        PyObject::new(
            PyGetSet::with_get_set(name.into(), g, s),
            self.types.getset_type.clone(),
            None,
        )
    }

    pub fn new_code_object(&self, code: bytecode::CodeObject) -> PyCodeRef {
        PyRef::new_ref(code::PyCode::new(code), self.types.code_type.clone(), None)
    }

    pub fn new_pyfunction(
        &self,
        code_obj: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> PyObjectRef {
        PyObject::new(
            PyFunction::new(code_obj, scope, defaults, kw_only_defaults),
            self.types.function_type.clone(),
            Some(self.new_dict()),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyBoundMethod::new(object, function),
            self.types.bound_method_type.clone(),
            None,
        )
    }

    pub fn new_base_object(&self, class: PyTypeRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject {
            typ: PyRwLock::new(class.into_typed_pyobj()),
            dict: dict.map(|d| PyRwLock::new(d.into_typed_pyobj())),
            payload: object::PyBaseObject,
        }
        .into_ref()
    }

    pub fn unwrap_constant(&self, value: &bytecode::Constant) -> PyObjectRef {
        match *value {
            bytecode::Constant::Integer { ref value } => self.new_bigint(value),
            bytecode::Constant::Float { ref value } => self.new_float(*value),
            bytecode::Constant::Complex { ref value } => self.new_complex(*value),
            bytecode::Constant::String { ref value } => self.new_str(value.clone()),
            bytecode::Constant::Bytes { ref value } => self.new_bytes(value.clone()),
            bytecode::Constant::Boolean { value } => self.new_bool(value),
            bytecode::Constant::Code { ref code } => {
                self.new_code_object(*code.clone()).into_object()
            }
            bytecode::Constant::Tuple { ref elements } => {
                let elements = elements
                    .iter()
                    .map(|value| self.unwrap_constant(value))
                    .collect();
                self.new_tuple(elements)
            }
            bytecode::Constant::None => self.none(),
            bytecode::Constant::Ellipsis => self.ellipsis(),
        }
    }

    pub fn add_tp_new_wrapper(&self, ty: &PyTypeRef) {
        if !ty.attributes.read().contains_key("__new__") {
            let new_wrapper =
                self.new_bound_method(self.tp_new_wrapper.clone(), ty.clone().into_object());
            ty.set_str_attr("__new__", new_wrapper);
        }
    }

    pub fn is_tp_new_wrapper(&self, obj: &PyObjectRef) -> bool {
        obj.payload::<PyBoundMethod>()
            .map_or(false, |bound| bound.function.is(&self.tp_new_wrapper))
    }
}

impl Default for PyContext {
    fn default() -> Self {
        PyContext::new()
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
pub struct PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    pub(crate) typ: PyRwLock<PyObjectRc<PyType>>, // __class__ member
    pub(crate) dict: Option<PyRwLock<PyObjectRc<PyDict>>>, // __dict__ member
    pub payload: T,
}

impl PyObjectRef {
    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    pub fn downcast<T: PyObjectPayload + PyValue>(self) -> Result<PyRef<T>, Self> {
        if self.payload_is::<T>() {
            Ok(unsafe { PyRef::from_obj_unchecked(self) })
        } else {
            Err(self)
        }
    }

    /// Attempt to downcast this reference to the specific class that is associated `T`.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    pub fn downcast_exact<T: PyObjectPayload + PyValue>(
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

    /// Downcast this PyObjectRef to an `PyRc<PyObject<T>>`. The [`downcast`](#method.downcast) method
    /// is generally preferred, as the `PyRef<T>` it returns implements `Deref<Target=T>`, and
    /// therefore can be used similarly to an `&T`.
    pub fn downcast_generic<T: PyObjectPayload>(self) -> Result<PyObjectRc<T>, Self> {
        if self.payload_is::<T>() {
            let ptr = PyObjectRc::into_raw(self) as *const PyObject<T>;
            let ret = unsafe { PyObjectRc::from_raw(ptr) };
            Ok(ret)
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<T: PyObjectPayload + PyValue>(&self) -> Option<&PyRef<T>> {
        if self.payload_is::<T>() {
            // when payload exacts, PyObjectRef == PyRef { PyObject }
            Some(unsafe { &*(self as *const PyObjectRef as *const PyRef<T>) })
        } else {
            None
        }
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
pub struct PyRef<T> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<T>,
}

impl<T> Clone for PyRef<T> {
    fn clone(&self) -> Self {
        Self {
            obj: self.obj.clone(),
            _payload: PhantomData,
        }
    }
}

impl<T: PyValue> PyRef<T> {
    #[allow(clippy::new_ret_no_self)]
    pub fn new_ref(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> Self {
        let obj = PyObject::new(payload, typ, dict);
        // SAFETY: we just created the object from a payload of type T
        unsafe { Self::from_obj_unchecked(obj) }
    }

    fn from_obj(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        if obj.payload_is::<T>() {
            // SAFETY: we just checked the payload
            Ok(unsafe { Self::from_obj_unchecked(obj) })
        } else {
            Err(vm.new_runtime_error(format!(
                "Unexpected payload '{}' for type '{}'",
                T::class(vm).name,
                obj.class().name,
            )))
        }
    }

    /// Safety: payload type of `obj` must be `T`
    pub(crate) unsafe fn from_obj_unchecked(obj: PyObjectRef) -> Self {
        PyRef {
            obj,
            _payload: PhantomData,
        }
    }

    pub fn as_object(&self) -> &PyObjectRef {
        &self.obj
    }

    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }

    pub fn into_typed_pyobj(self) -> PyObjectRc<T> {
        self.into_object().downcast_generic().unwrap()
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyValue,
{
    type Target = T;

    fn deref(&self) -> &T {
        self.obj.payload().unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                panic!("PyRef.obj did not have a payload of T")
            }
            // SAFETY: self.obj has invariant that payload is `T`
            unsafe { std::hint::unreachable_unchecked() }
        })
    }
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyValue,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let class = T::class(vm);
        if obj.isinstance(class) {
            PyRef::from_obj(obj, vm)
        } else {
            let expected_type = &class.name;
            let actual_type = &obj.class().name;
            Err(vm.new_type_error(format!(
                "Expected type '{}', not '{}'",
                expected_type, actual_type,
            )))
        }
    }
}

impl<'a, T: PyValue> From<&'a PyRef<T>> for &'a PyObjectRef {
    fn from(obj: &'a PyRef<T>) -> Self {
        obj.as_object()
    }
}

impl<T: PyValue> From<PyRef<T>> for PyObjectRef {
    fn from(obj: PyRef<T>) -> Self {
        obj.into_object()
    }
}

impl<T: fmt::Display> fmt::Display for PyRef<T>
where
    T: PyValue + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value: &T = self.obj.payload().expect("unexpected payload for type");
        fmt::Display::fmt(value, f)
    }
}

#[derive(Clone, Debug)]
pub struct PyCallable {
    obj: PyObjectRef,
}

impl PyCallable {
    #[inline]
    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        vm.invoke(&self.obj, args)
    }

    #[inline]
    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }
}

impl TryFromObject for PyCallable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.is_callable(&obj) {
            Ok(PyCallable { obj })
        } else {
            Err(vm.new_type_error(format!("'{}' object is not callable", obj.class().name)))
        }
    }
}

pub type Never = std::convert::Infallible;

impl PyValue for Never {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        unreachable!()
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

impl<T: ?Sized + PyObjectPayload> IdProtocol for PyObject<T> {
    fn get_id(&self) -> usize {
        self as *const _ as *const PyObject<Never> as usize
    }
}

impl<T: ?Sized + IdProtocol> IdProtocol for PyRc<T> {
    fn get_id(&self) -> usize {
        (**self).get_id()
    }
}

impl<T: PyObjectPayload> IdProtocol for PyRef<T> {
    fn get_id(&self) -> usize {
        self.obj.get_id()
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
    inner: PyRwLockReadGuard<'a, PyObjectRc<T>>,
}

impl<'a, T: PyObjectPayload + PyValue> PyLease<'a, T> {
    // Associated function on purpose, because of deref
    #[allow(clippy::wrong_self_convention)]
    pub fn into_pyref(zelf: Self) -> PyRef<T> {
        zelf.inner.clone().into_pyref()
    }
}

impl<'a, T: PyObjectPayload + PyValue> Deref for PyLease<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.payload
    }
}

impl<'a, T> fmt::Display for PyLease<'a, T>
where
    T: PyValue + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.inner.payload, f)
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
    fn isinstance(&self, cls: &PyTypeRef) -> bool {
        self.class().issubclass(cls)
    }
}

impl TypeProtocol for PyObjectRef {
    fn class(&self) -> PyLease<'_, PyType> {
        (**self).class()
    }
}

impl<T> TypeProtocol for PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    fn class(&self) -> PyLease<'_, PyType> {
        PyLease {
            inner: self.typ.read(),
        }
    }
}

impl<T> TypeProtocol for PyRef<T> {
    fn class(&self) -> PyLease<'_, PyType> {
        self.obj.class()
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
    fn set_item(&self, key: T, value: PyObjectRef, vm: &VirtualMachine) -> PyResult;
    fn del_item(&self, key: T, vm: &VirtualMachine) -> PyResult;
}

impl<T> ItemProtocol<T> for PyObjectRef
where
    T: IntoPyObject,
{
    fn get_item(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__getitem__", (key,))
    }

    fn set_item(&self, key: T, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__setitem__", (key, value))
    }

    fn del_item(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__delitem__", (key,))
    }
}

impl fmt::Debug for PyObject<dyn PyObjectPayload> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObj {:?}]", &self.payload)
    }
}

/// An iterable Python object.
///
/// `PyIterable` implements `FromArgs` so that a built-in function can accept
/// an object that is required to conform to the Python iterator protocol.
///
/// PyIterable can optionally perform type checking and conversions on iterated
/// objects using a generic type parameter that implements `TryFromObject`.
pub struct PyIterable<T = PyObjectRef> {
    method: PyObjectRef,
    _item: std::marker::PhantomData<T>,
}

impl<T> PyIterable<T> {
    pub fn from_method(method: PyObjectRef) -> Self {
        PyIterable {
            method,
            _item: std::marker::PhantomData,
        }
    }

    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterator<'a, T>> {
        let method = &self.method;
        let iter_obj = vm.invoke(method, ())?;

        let length_hint = iterator::length_hint(vm, iter_obj.clone())?;

        Ok(PyIterator {
            vm,
            obj: iter_obj,
            length_hint,
            _item: std::marker::PhantomData,
        })
    }
}

impl<T> TryFromObject for PyIterable<T>
where
    T: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if let Some(method_or_err) = vm.get_method(obj.clone(), "__iter__") {
            let method = method_or_err?;
            Ok(PyIterable {
                method,
                _item: std::marker::PhantomData,
            })
        } else {
            vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
                format!("'{}' object is not iterable", obj.class().name)
            })?;
            Self::try_from_object(
                vm,
                PySequenceIterator::new_forward(obj)
                    .into_ref(vm)
                    .into_object(),
            )
        }
    }
}

pub struct PyIterator<'a, T> {
    vm: &'a VirtualMachine,
    obj: PyObjectRef,
    length_hint: Option<usize>,
    _item: std::marker::PhantomData<T>,
}

impl<'a, T> Iterator for PyIterator<'a, T>
where
    T: TryFromObject,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.vm.call_method(&self.obj, "__next__", ()) {
            Ok(value) => Some(T::try_from_object(self.vm, value)),
            Err(err) => {
                if err.isinstance(&self.vm.ctx.exceptions.stop_iteration) {
                    None
                } else {
                    Some(Err(err))
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.length_hint.unwrap_or(0), self.length_hint)
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

/// Allows coercion of a types into PyRefs, so that we can write functions that can take
/// refs, pyobject refs or basic types.
pub trait TryIntoRef<T> {
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<T>>;
}

impl<T> TryIntoRef<T> for PyRef<T> {
    fn try_into_ref(self, _vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        Ok(self)
    }
}

impl<T> TryIntoRef<T> for PyObjectRef
where
    T: PyValue,
{
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        TryFromObject::try_from_object(vm, self)
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

pub trait IntoPyRef<T> {
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

/// Implemented by any type that can be returned from a built-in Python function.
///
/// `IntoPyObject` has a blanket implementation for any built-in object payload,
/// and should be implemented by many primitive Rust types, allowing a built-in
/// function to simply return a `bool` or a `usize` for example.
pub trait IntoPyObject {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef;
}

impl<T> IntoPyObject for PyRef<T> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.obj
    }
}

impl IntoPyObject for PyCallable {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.into_object()
    }
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyValue + Sized,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyValue::into_object(self, vm)
    }
}

pub trait IntoPyResult {
    fn into_pyresult(self, vm: &VirtualMachine) -> PyResult;
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

impl<T> PyObject<T>
where
    T: Sized + PyObjectPayload,
{
    #[allow(clippy::new_ret_no_self)]
    pub fn new(payload: T, typ: PyTypeRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject {
            typ: PyRwLock::new(typ.into_typed_pyobj()),
            dict: dict.map(|d| PyRwLock::new(d.into_typed_pyobj())),
            payload,
        }
        .into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        let raw = PyObjectRc::into_raw(PyObjectRc::new(self));
        unsafe { PyObjectRef::from_raw(raw) }
    }
}

impl<T> PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    pub fn dict(&self) -> Option<PyDictRef> {
        self.dict.as_ref().map(|mu| mu.read().clone().into_pyref())
    }
    /// Set the dict field. Returns `Err(dict)` if this object does not have a dict field
    /// in the first place.
    pub fn set_dict(&self, dict: PyDictRef) -> Result<(), PyDictRef> {
        match self.dict {
            Some(ref mu) => {
                *mu.write() = dict.into_typed_pyobj();
                Ok(())
            }
            None => Err(dict),
        }
    }
}

impl PyObject<dyn PyObjectPayload> {
    #[inline]
    pub fn payload<T: PyObjectPayload>(&self) -> Option<&T> {
        self.payload.as_any().downcast_ref()
    }

    #[inline]
    pub fn payload_is<T: PyObjectPayload>(&self) -> bool {
        self.payload.as_any().is::<T>()
    }

    #[inline]
    pub fn payload_if_subclass<T: PyObjectPayload + PyValue>(
        &self,
        vm: &VirtualMachine,
    ) -> Option<&T> {
        if self.class().issubclass(T::class(vm)) {
            self.payload()
        } else {
            None
        }
    }
}

impl<T> PyObjectRc<T>
where
    T: Sized + PyObjectPayload,
{
    pub fn into_pyref(self) -> PyRef<T>
    where
        T: PyValue,
    {
        // SAFETY: we know just casted from PyRc<PyObject<T>> to PyObjectRef, so we know the
        // payload is `T`
        let raw = PyObjectRc::into_raw(self);
        unsafe {
            let rc = PyObjectRc::<dyn PyObjectPayload>::from_raw(raw);
            PyRef::from_obj_unchecked(rc)
        }
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

    fn into_object(self, vm: &VirtualMachine) -> PyObjectRef {
        self.into_ref(vm).into_object()
    }

    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        self.into_ref_with_type_unchecked(Self::class(vm).clone(), vm)
    }

    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyTypeRef) -> PyResult<PyRef<Self>> {
        let class = Self::class(vm);
        if cls.issubclass(class) {
            Ok(self.into_ref_with_type_unchecked(cls, vm))
        } else {
            let subtype = vm.to_str(&cls.obj)?;
            let basetype = vm.to_str(&class.obj)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }

    fn into_ref_with_type_unchecked(self, cls: PyTypeRef, vm: &VirtualMachine) -> PyRef<Self> {
        let dict = if cls.slots.flags.has_feature(PyTpFlags::HAS_DICT) {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        PyRef::new_ref(self, cls, dict)
    }
}

pub trait PyObjectPayload: Any + fmt::Debug + PyThreadingConstraint + 'static {
    fn as_any(&self) -> &dyn Any;
}

impl<T: PyValue + 'static> PyObjectPayload for T {
    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A: PyValue, B: PyValue> Either<PyRef<A>, PyRef<B>> {
    pub fn as_object(&self) -> &PyObjectRef {
        match self {
            Either::A(a) => a.as_object(),
            Either::B(b) => b.as_object(),
        }
    }

    pub fn into_object(self) -> PyObjectRef {
        match self {
            Either::A(a) => a.into_object(),
            Either::B(b) => b.into_object(),
        }
    }
}

impl<A: IntoPyObject, B: IntoPyObject> IntoPyObject for Either<A, B> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::A(a) => a.into_pyobject(vm),
            Self::B(b) => b.into_pyobject(vm),
        }
    }
}

/// This allows a builtin method to accept arguments that may be one of two
/// types, raising a `TypeError` if it is neither.
///
/// # Example
///
/// ```
/// use rustpython_vm::VirtualMachine;
/// use rustpython_vm::builtins::{PyStrRef, PyIntRef};
/// use rustpython_vm::pyobject::Either;
///
/// fn do_something(arg: Either<PyIntRef, PyStrRef>, vm: &VirtualMachine) {
///     match arg {
///         Either::A(int)=> {
///             // do something with int
///         }
///         Either::B(string) => {
///             // do something with string
///         }
///     }
/// }
/// ```
impl<A, B> TryFromObject for Either<A, B>
where
    A: TryFromObject,
    B: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        A::try_from_object(vm, obj.clone())
            .map(Either::A)
            .or_else(|_| B::try_from_object(vm, obj.clone()).map(Either::B))
            .map_err(|_| vm.new_type_error(format!("unexpected type {}", obj.class())))
    }
}

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
        static_cell::get(Self::static_cell()).unwrap_or_else(|| unsafe {
            // SAFETY: object must be initialized by init_* method.
            // So this is actually not safe as itself.
            // But easy to find out when it happened in debug build
            std::hint::unreachable_unchecked()
        })
    }

    fn init_manually(typ: PyTypeRef) -> &'static PyTypeRef {
        static_cell::init_expect(Self::static_cell(), typ, "init_manually")
    }
    fn init_bare_type() -> &'static PyTypeRef
    where
        Self: PyClassImpl,
    {
        let typ = Self::create_bare_type();
        static_cell::init_expect(Self::static_cell(), typ, Self::NAME)
    }
    fn create_bare_type() -> PyTypeRef
    where
        Self: PyClassImpl,
    {
        create_type_with_slots(
            Self::NAME,
            Self::static_metaclass(),
            Self::static_baseclass(),
            Self::make_slots(),
        )
    }
}

impl<T> PyClassDef for PyRef<T>
where
    T: PyClassDef,
{
    const NAME: &'static str = T::NAME;
    const MODULE_NAME: Option<&'static str> = T::MODULE_NAME;
    const TP_NAME: &'static str = T::TP_NAME;
    const DOC: Option<&'static str> = T::DOC;
}

pub trait PyClassImpl: PyClassDef {
    const TP_FLAGS: PyTpFlags = PyTpFlags::DEFAULT;

    fn impl_extend_class(ctx: &PyContext, class: &PyTypeRef);

    fn extend_class(ctx: &PyContext, class: &PyTypeRef) {
        #[cfg(debug_assertions)]
        {
            assert!(class.slots.flags.is_created_with_flags());
        }
        if Self::TP_FLAGS.has_feature(PyTpFlags::HAS_DICT) {
            class.set_str_attr(
                "__dict__",
                ctx.new_getset("__dict__", object::object_get_dict, object::object_set_dict),
            );
        }
        Self::impl_extend_class(ctx, class);
        ctx.add_tp_new_wrapper(&class);
        if let Some(doc) = Self::DOC {
            class.set_str_attr("__doc__", ctx.new_str(doc));
        }
        if let Some(module_name) = Self::MODULE_NAME {
            class.set_str_attr("__module__", ctx.new_str(module_name));
        }
    }

    fn make_class(ctx: &PyContext) -> PyTypeRef
    where
        Self: StaticType,
    {
        static_cell::get_or_init(Self::static_cell(), || {
            let typ = Self::create_bare_type();
            Self::extend_class(ctx, &typ);
            typ
        })
        .clone()
    }

    fn extend_slots(slots: &mut PyTypeSlots);

    fn make_slots() -> PyTypeSlots {
        let mut slots = PyTypeSlots::default();
        slots.flags = Self::TP_FLAGS;
        slots.name = PyRwLock::new(Some(Self::TP_NAME.to_owned()));
        Self::extend_slots(&mut slots);
        slots
    }
}

#[pyimpl]
pub trait PyStructSequence: StaticType + PyClassImpl + Sized + 'static {
    const FIELD_NAMES: &'static [&'static str];

    fn into_tuple(self, vm: &VirtualMachine) -> PyTuple;

    fn into_struct_sequence(self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.into_tuple(vm)
            .into_ref_with_type(vm, Self::static_type().clone())
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyResult<String> {
        let format_field = |(value, name)| {
            let s = vm.to_repr(value)?;
            Ok(format!("{}: {}", name, s))
        };
        let (body, suffix) =
            if let Some(_guard) = rustpython_vm::vm::ReprGuard::enter(vm, zelf.as_object()) {
                if Self::FIELD_NAMES.len() == 1 {
                    let value = zelf.borrow_value().first().unwrap();
                    let formatted = format_field((value, Self::FIELD_NAMES[0]))?;
                    (formatted, ",")
                } else {
                    let fields: PyResult<Vec<_>> = zelf
                        .borrow_value()
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

    #[extend_class]
    fn extend_pyclass(ctx: &PyContext, class: &PyTypeRef) {
        for (i, &name) in Self::FIELD_NAMES.iter().enumerate() {
            // cast i to a u8 so there's less to store in the getter closure.
            // Hopefully there's not struct sequences with >=256 elements :P
            let i = i as u8;
            class.set_str_attr(
                name,
                ctx.new_readonly_getset(name, move |zelf: &PyTuple| zelf.fast_getitem(i.into())),
            );
        }
    }
}

// TODO: find a better place to put this impl
impl TryFromObject for std::time::Duration {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        use std::time::Duration;
        u64::try_from_object(vm, obj.clone())
            .map(Duration::from_secs)
            .or_else(|_| f64::try_from_object(vm, obj.clone()).map(Duration::from_secs_f64))
            .map_err(|_| {
                vm.new_type_error(format!(
                    "expected an int or float for duration, got {}",
                    obj.class()
                ))
            })
    }
}

result_like::option_like!(pub PyArithmaticValue, Implemented, NotImplemented);

impl PyArithmaticValue<PyObjectRef> {
    pub fn from_object(vm: &VirtualMachine, obj: PyObjectRef) -> Self {
        if obj.is(&vm.ctx.not_implemented) {
            Self::NotImplemented
        } else {
            Self::Implemented(obj)
        }
    }
}

impl<T: TryFromObject> TryFromObject for PyArithmaticValue<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyArithmaticValue::from_object(vm, obj)
            .map(|x| T::try_from_object(vm, x))
            .transpose()
    }
}

impl<T> IntoPyObject for PyArithmaticValue<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            PyArithmaticValue::Implemented(v) => v.into_pyobject(vm),
            PyArithmaticValue::NotImplemented => vm.ctx.not_implemented(),
        }
    }
}

pub type PyComparisonValue = PyArithmaticValue<bool>;

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

pub fn hash_iter<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    vm.state.hash_secret.hash_iter(iter, |obj| vm._hash(obj))
}

pub fn hash_iter_unordered<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    rustpython_common::hash::hash_iter_unordered(iter, |obj| vm._hash(obj))
}

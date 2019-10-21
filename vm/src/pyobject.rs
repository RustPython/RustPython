use std::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::Rc;

use indexmap::IndexMap;
use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::{One, ToPrimitive, Zero};

use crate::bytecode;
use crate::dictdatatype::DictKey;
use crate::exceptions;
use crate::function::{IntoPyNativeFunc, PyFuncArgs};
use crate::obj::objbuiltinfunc::PyBuiltinFunction;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objclassmethod::PyClassMethod;
use crate::obj::objcode;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objcomplex::PyComplex;
use crate::obj::objdict::{PyDict, PyDictRef};
use crate::obj::objfloat::PyFloat;
use crate::obj::objfunction::{PyFunction, PyMethod};
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objiter;
use crate::obj::objlist::PyList;
use crate::obj::objnamespace::PyNamespace;
use crate::obj::objnone::{PyNone, PyNoneRef};
use crate::obj::objobject;
use crate::obj::objproperty::PropertyBuilder;
use crate::obj::objset::PySet;
use crate::obj::objstr;
use crate::obj::objtuple::{PyTuple, PyTupleRef};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::scope::Scope;
use crate::types::{create_type, initialize_types, TypeZoo};
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
pub type PyObjectRef = Rc<PyObject<dyn PyObjectPayload>>;

/// Use this type for function which return a python object or and exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyObjectRef>; // A valid value, or an exception

/// For attributes we do not use a dict, but a hashmap. This is probably
/// faster, unordered, and only supports strings as keys.
/// TODO: class attributes should maintain insertion order (use IndexMap here)
pub type PyAttributes = HashMap<String, PyObjectRef>;

impl fmt::Display for PyObject<dyn PyObjectPayload> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(PyClass { ref name, .. }) = self.payload::<PyClass>() {
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

const INT_CACHE_POOL_MIN: i32 = -5;
const INT_CACHE_POOL_MAX: i32 = 256;

#[derive(Debug)]
pub struct PyContext {
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub none: PyNoneRef,
    pub empty_tuple: PyTupleRef,
    pub ellipsis_type: PyClassRef,
    pub ellipsis: PyEllipsisRef,
    pub not_implemented: PyNotImplementedRef,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyObjectRef>,
}

pub type PyNotImplementedRef = PyRef<PyNotImplemented>;

#[derive(Debug)]
pub struct PyNotImplemented;

impl PyValue for PyNotImplemented {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.not_implemented().class()
    }
}

pub type PyEllipsisRef = PyRef<PyEllipsis>;

#[derive(Debug)]
pub struct PyEllipsis;

impl PyValue for PyEllipsis {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.ellipsis_type.clone()
    }
}

// Basic objects:
impl PyContext {
    pub fn new() -> Self {
        flame_guard!("init PyContext");
        let types = TypeZoo::new();
        let exceptions = exceptions::ExceptionZoo::new(&types.type_type, &types.object_type);

        fn create_object<T: PyObjectPayload + PyValue>(payload: T, cls: &PyClassRef) -> PyRef<T> {
            PyRef::new_ref_unchecked(PyObject::new(payload, cls.clone(), None))
        }

        let none_type = create_type("NoneType", &types.type_type, &types.object_type);
        let none = create_object(PyNone, &none_type);

        let ellipsis_type = create_type("EllipsisType", &types.type_type, &types.object_type);
        let ellipsis = create_object(PyEllipsis, &ellipsis_type);

        let not_implemented_type =
            create_type("NotImplementedType", &types.type_type, &types.object_type);
        let not_implemented = create_object(PyNotImplemented, &not_implemented_type);

        let int_cache_pool = (INT_CACHE_POOL_MIN..=INT_CACHE_POOL_MAX)
            .map(|v| create_object(PyInt::new(BigInt::from(v)), &types.int_type).into_object())
            .collect();

        let true_value = create_object(PyInt::new(BigInt::one()), &types.bool_type);
        let false_value = create_object(PyInt::new(BigInt::zero()), &types.bool_type);

        let empty_tuple = create_object(PyTuple::from(vec![]), &types.tuple_type);

        let context = PyContext {
            true_value,
            false_value,
            not_implemented,
            none,
            empty_tuple,
            ellipsis,
            ellipsis_type,

            types,
            exceptions,
            int_cache_pool,
        };
        initialize_types(&context);

        exceptions::init(&context);
        context
    }

    pub fn bytearray_type(&self) -> PyClassRef {
        self.types.bytearray_type.clone()
    }

    pub fn bytearrayiterator_type(&self) -> PyClassRef {
        self.types.bytearrayiterator_type.clone()
    }

    pub fn bytes_type(&self) -> PyClassRef {
        self.types.bytes_type.clone()
    }

    pub fn bytesiterator_type(&self) -> PyClassRef {
        self.types.bytesiterator_type.clone()
    }

    pub fn code_type(&self) -> PyClassRef {
        self.types.code_type.clone()
    }

    pub fn complex_type(&self) -> PyClassRef {
        self.types.complex_type.clone()
    }

    pub fn dict_type(&self) -> PyClassRef {
        self.types.dict_type.clone()
    }

    pub fn float_type(&self) -> PyClassRef {
        self.types.float_type.clone()
    }

    pub fn frame_type(&self) -> PyClassRef {
        self.types.frame_type.clone()
    }

    pub fn int_type(&self) -> PyClassRef {
        self.types.int_type.clone()
    }

    pub fn list_type(&self) -> PyClassRef {
        self.types.list_type.clone()
    }

    pub fn listiterator_type(&self) -> PyClassRef {
        self.types.listiterator_type.clone()
    }

    pub fn listreverseiterator_type(&self) -> PyClassRef {
        self.types.listreverseiterator_type.clone()
    }

    pub fn striterator_type(&self) -> PyClassRef {
        self.types.striterator_type.clone()
    }

    pub fn strreverseiterator_type(&self) -> PyClassRef {
        self.types.strreverseiterator_type.clone()
    }

    pub fn module_type(&self) -> PyClassRef {
        self.types.module_type.clone()
    }

    pub fn namespace_type(&self) -> PyClassRef {
        self.types.namespace_type.clone()
    }

    pub fn set_type(&self) -> PyClassRef {
        self.types.set_type.clone()
    }

    pub fn range_type(&self) -> PyClassRef {
        self.types.range_type.clone()
    }

    pub fn rangeiterator_type(&self) -> PyClassRef {
        self.types.rangeiterator_type.clone()
    }

    pub fn slice_type(&self) -> PyClassRef {
        self.types.slice_type.clone()
    }

    pub fn frozenset_type(&self) -> PyClassRef {
        self.types.frozenset_type.clone()
    }

    pub fn bool_type(&self) -> PyClassRef {
        self.types.bool_type.clone()
    }

    pub fn memoryview_type(&self) -> PyClassRef {
        self.types.memoryview_type.clone()
    }

    pub fn tuple_type(&self) -> PyClassRef {
        self.types.tuple_type.clone()
    }

    pub fn tupleiterator_type(&self) -> PyClassRef {
        self.types.tupleiterator_type.clone()
    }

    pub fn iter_type(&self) -> PyClassRef {
        self.types.iter_type.clone()
    }

    pub fn enumerate_type(&self) -> PyClassRef {
        self.types.enumerate_type.clone()
    }

    pub fn filter_type(&self) -> PyClassRef {
        self.types.filter_type.clone()
    }

    pub fn map_type(&self) -> PyClassRef {
        self.types.map_type.clone()
    }

    pub fn zip_type(&self) -> PyClassRef {
        self.types.zip_type.clone()
    }

    pub fn str_type(&self) -> PyClassRef {
        self.types.str_type.clone()
    }

    pub fn super_type(&self) -> PyClassRef {
        self.types.super_type.clone()
    }

    pub fn function_type(&self) -> PyClassRef {
        self.types.function_type.clone()
    }

    pub fn builtin_function_or_method_type(&self) -> PyClassRef {
        self.types.builtin_function_or_method_type.clone()
    }

    pub fn property_type(&self) -> PyClassRef {
        self.types.property_type.clone()
    }

    pub fn readonly_property_type(&self) -> PyClassRef {
        self.types.readonly_property_type.clone()
    }

    pub fn classmethod_type(&self) -> PyClassRef {
        self.types.classmethod_type.clone()
    }

    pub fn staticmethod_type(&self) -> PyClassRef {
        self.types.staticmethod_type.clone()
    }

    pub fn generator_type(&self) -> PyClassRef {
        self.types.generator_type.clone()
    }

    pub fn bound_method_type(&self) -> PyClassRef {
        self.types.bound_method_type.clone()
    }

    pub fn weakref_type(&self) -> PyClassRef {
        self.types.weakref_type.clone()
    }

    pub fn weakproxy_type(&self) -> PyClassRef {
        self.types.weakproxy_type.clone()
    }

    pub fn type_type(&self) -> PyClassRef {
        self.types.type_type.clone()
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

    pub fn object(&self) -> PyClassRef {
        self.types.object_type.clone()
    }

    #[inline]
    pub fn new_int<T: Into<BigInt> + ToPrimitive>(&self, i: T) -> PyObjectRef {
        if let Some(i) = i.to_i32() {
            if i >= INT_CACHE_POOL_MIN && i <= INT_CACHE_POOL_MAX {
                let inner_idx = (i - INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyObject::new(PyInt::new(i), self.int_type(), None)
    }

    #[inline]
    pub fn new_bigint(&self, i: &BigInt) -> PyObjectRef {
        if let Some(i) = i.to_i32() {
            if i >= INT_CACHE_POOL_MIN && i <= INT_CACHE_POOL_MAX {
                let inner_idx = (i - INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyObject::new(PyInt::new(i.clone()), self.int_type(), None)
    }

    pub fn new_float(&self, value: f64) -> PyObjectRef {
        PyObject::new(PyFloat::from(value), self.float_type(), None)
    }

    pub fn new_complex(&self, value: Complex64) -> PyObjectRef {
        PyObject::new(PyComplex::from(value), self.complex_type(), None)
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(objstr::PyString::from(s), self.str_type(), None)
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(objbytes::PyBytes::new(data), self.bytes_type(), None)
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            objbytearray::PyByteArray::new(data),
            self.bytearray_type(),
            None,
        )
    }

    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        let value = if b {
            &self.true_value
        } else {
            &self.false_value
        };
        value.clone().into_object()
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        if elements.is_empty() {
            self.empty_tuple.clone().into_object()
        } else {
            PyObject::new(PyTuple::from(elements), self.tuple_type(), None)
        }
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyList::from(elements), self.list_type(), None)
    }

    pub fn new_set(&self) -> PyObjectRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the objset code itself.
        PyObject::new(PySet::default(), self.set_type(), None)
    }

    pub fn new_dict(&self) -> PyDictRef {
        PyObject::new(PyDict::default(), self.dict_type(), None)
            .downcast()
            .unwrap()
    }

    pub fn new_class(&self, name: &str, base: PyClassRef) -> PyClassRef {
        objtype::new(self.type_type(), name, vec![base], PyAttributes::new()).unwrap()
    }

    pub fn new_namespace(&self) -> PyObjectRef {
        PyObject::new(PyNamespace, self.namespace_type(), Some(self.new_dict()))
    }

    pub fn new_rustfunc<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyBuiltinFunction::new(f.into_func()),
            self.builtin_function_or_method_type(),
            None,
        )
    }

    pub fn new_classmethod<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyClassMethod {
                callable: self.new_rustfunc(f),
            },
            self.classmethod_type(),
            None,
        )
    }

    pub fn new_property<F, I, V>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<I, V>,
    {
        PropertyBuilder::new(self).add_getter(f).create()
    }

    pub fn new_code_object(&self, code: bytecode::CodeObject) -> PyCodeRef {
        PyObject::new(objcode::PyCode::new(code), self.code_type(), None)
            .downcast()
            .unwrap()
    }

    pub fn new_function(
        &self,
        code_obj: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> PyObjectRef {
        PyObject::new(
            PyFunction::new(code_obj, scope, defaults, kw_only_defaults),
            self.function_type(),
            Some(self.new_dict()),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyMethod::new(object, function),
            self.bound_method_type(),
            None,
        )
    }

    pub fn new_instance(&self, class: PyClassRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject {
            typ: class,
            dict,
            payload: objobject::PyInstance,
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
            bytecode::Constant::Boolean { ref value } => self.new_bool(value.clone()),
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
    pub typ: PyClassRef,
    pub dict: Option<PyDictRef>, // __dict__ member
    pub payload: T,
}

impl PyObject<dyn PyObjectPayload> {
    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    ///
    /// Note: The returned `Result` is _not_ a `PyResult`, even though the
    ///       types are compatible.
    pub fn downcast<T: PyObjectPayload>(self: Rc<Self>) -> Result<PyRef<T>, PyObjectRef> {
        if self.payload_is::<T>() {
            Ok({
                PyRef {
                    obj: self,
                    _payload: PhantomData,
                }
            })
        } else {
            Err(self)
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
    fn new_ref(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        if obj.payload_is::<T>() {
            Ok(Self::new_ref_unchecked(obj))
        } else {
            Err(vm.new_exception(
                vm.ctx.exceptions.runtime_error.clone(),
                format!("Unexpected payload for type {:?}", obj.class().name),
            ))
        }
    }

    fn new_ref_unchecked(obj: PyObjectRef) -> Self {
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

    pub fn typ(&self) -> PyClassRef {
        self.obj.class()
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyValue,
{
    type Target = T;

    fn deref(&self) -> &T {
        self.obj.payload().expect("unexpected payload for type")
    }
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyValue,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if objtype::isinstance(&obj, &T::class(vm)) {
            PyRef::new_ref(obj, vm)
        } else {
            let class = T::class(vm);
            let expected_type = vm.to_pystr(&class)?;
            let actual_type = vm.to_pystr(&obj.class())?;
            Err(vm.new_type_error(format!(
                "Expected type {}, not {}",
                expected_type, actual_type,
            )))
        }
    }
}

impl<T> IntoPyObject for PyRef<T> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self.obj)
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
    pub fn invoke(&self, args: impl Into<PyFuncArgs>, vm: &VirtualMachine) -> PyResult {
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

impl IntoPyObject for PyCallable {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self.into_object())
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

#[derive(Debug)]
enum Never {}

impl PyValue for Never {
    fn class(_vm: &VirtualMachine) -> PyClassRef {
        unreachable!()
    }
}

impl<T: ?Sized + PyObjectPayload> IdProtocol for PyObject<T> {
    fn get_id(&self) -> usize {
        self as *const _ as *const PyObject<Never> as usize
    }
}

impl<T: ?Sized + IdProtocol> IdProtocol for Rc<T> {
    fn get_id(&self) -> usize {
        (**self).get_id()
    }
}

impl<T: PyObjectPayload> IdProtocol for PyRef<T> {
    fn get_id(&self) -> usize {
        self.obj.get_id()
    }
}

pub trait TypeProtocol {
    fn class(&self) -> PyClassRef;
}

impl TypeProtocol for PyObjectRef {
    fn class(&self) -> PyClassRef {
        (**self).class()
    }
}

impl<T> TypeProtocol for PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    fn class(&self) -> PyClassRef {
        self.typ.clone()
    }
}

impl<T> TypeProtocol for PyRef<T> {
    fn class(&self) -> PyClassRef {
        self.obj.typ.clone()
    }
}

/// The python item protocol. Mostly applies to dictionaries.
/// Allows getting, setting and deletion of keys-value pairs.
pub trait ItemProtocol {
    fn get_item<T: IntoPyObject + DictKey + Copy>(&self, key: T, vm: &VirtualMachine) -> PyResult;
    fn set_item<T: IntoPyObject + DictKey + Copy>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult;
    fn del_item<T: IntoPyObject + DictKey + Copy>(&self, key: T, vm: &VirtualMachine) -> PyResult;
}

impl ItemProtocol for PyObjectRef {
    fn get_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__getitem__", key.into_pyobject(vm)?)
    }

    fn set_item<T: IntoPyObject>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        vm.call_method(self, "__setitem__", vec![key.into_pyobject(vm)?, value])
    }

    fn del_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__delitem__", key.into_pyobject(vm)?)
    }
}

pub trait BufferProtocol {
    fn readonly(&self) -> bool;
}

impl BufferProtocol for PyObjectRef {
    fn readonly(&self) -> bool {
        match self.class().name.as_str() {
            "bytes" => false,
            "bytearray" | "memoryview" => true,
            _ => panic!("Bytes-Like type expected not {:?}", self),
        }
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
    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterator<'a, T>> {
        let method = &self.method;
        let iter_obj = vm.invoke(
            method,
            PyFuncArgs {
                args: vec![],
                kwargs: IndexMap::new(),
            },
        )?;

        Ok(PyIterator {
            vm,
            obj: iter_obj,
            _item: std::marker::PhantomData,
        })
    }
}

pub struct PyIterator<'a, T> {
    vm: &'a VirtualMachine,
    obj: PyObjectRef,
    _item: std::marker::PhantomData<T>,
}

impl<'a, T> Iterator for PyIterator<'a, T>
where
    T: TryFromObject,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.vm.call_method(&self.obj, "__next__", vec![]) {
            Ok(value) => Some(T::try_from_object(self.vm, value)),
            Err(err) => {
                if objtype::isinstance(&err, &self.vm.ctx.exceptions.stop_iteration) {
                    None
                } else {
                    Some(Err(err))
                }
            }
        }
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
                objiter::PySequenceIterator {
                    position: Cell::new(0),
                    obj: obj.clone(),
                    reversed: false,
                }
                .into_ref(vm)
                .into_object(),
            )
        }
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
        if vm.get_none().is(&obj) {
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

/// Implemented by any type that can be returned from a built-in Python function.
///
/// `IntoPyObject` has a blanket implementation for any built-in object payload,
/// and should be implemented by many primitive Rust types, allowing a built-in
/// function to simply return a `bool` or a `usize` for example.
pub trait IntoPyObject {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult;
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self)
    }
}

impl IntoPyObject for &PyObjectRef {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self.clone())
    }
}

impl<T> IntoPyObject for PyResult<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        self.and_then(|res| T::into_pyobject(res, vm))
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyValue + Sized,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(self, T::class(vm), None))
    }
}

impl<T> PyObject<T>
where
    T: Sized + PyObjectPayload,
{
    #[allow(clippy::new_ret_no_self)]
    pub fn new(payload: T, typ: PyClassRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject { typ, dict, payload }.into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(self)
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
}

pub trait PyValue: fmt::Debug + Sized + 'static {
    const HAVE_DICT: bool = false;

    fn class(vm: &VirtualMachine) -> PyClassRef;

    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        PyRef::new_ref_unchecked(PyObject::new(self, Self::class(vm), None))
    }

    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyClassRef) -> PyResult<PyRef<Self>> {
        let class = Self::class(vm);
        if objtype::issubclass(&cls, &class) {
            let dict = if !Self::HAVE_DICT && cls.is(&class) {
                None
            } else {
                Some(vm.ctx.new_dict())
            };
            PyRef::new_ref(PyObject::new(self, cls, dict), vm)
        } else {
            let subtype = vm.to_pystr(&cls.obj)?;
            let basetype = vm.to_pystr(&class.obj)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }
}

pub trait PyObjectPayload: Any + fmt::Debug + 'static {
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
    pub fn into_object(self) -> PyObjectRef {
        match self {
            Either::A(a) => a.into_object(),
            Either::B(b) => b.into_object(),
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
/// use rustpython_vm::obj::{objstr::PyStringRef, objint::PyIntRef};
/// use rustpython_vm::pyobject::Either;
///
/// fn do_something(arg: Either<PyIntRef, PyStringRef>, vm: &VirtualMachine) {
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
    const DOC: Option<&'static str> = None;
}

impl<T> PyClassDef for PyRef<T>
where
    T: PyClassDef,
{
    const NAME: &'static str = T::NAME;
    const DOC: Option<&'static str> = T::DOC;
}

pub trait PyClassImpl: PyClassDef {
    fn impl_extend_class(ctx: &PyContext, class: &PyClassRef);

    fn extend_class(ctx: &PyContext, class: &PyClassRef) {
        Self::impl_extend_class(ctx, class);
        if let Some(doc) = Self::DOC {
            class.set_str_attr("__doc__", ctx.new_str(doc.into()));
        }
    }

    fn make_class(ctx: &PyContext) -> PyClassRef {
        Self::make_class_with_base(ctx, ctx.object())
    }

    fn make_class_with_base(ctx: &PyContext, base: PyClassRef) -> PyClassRef {
        let py_class = ctx.new_class(Self::NAME, base);
        Self::extend_class(ctx, &py_class);
        py_class
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_type() {
        // TODO: Write this test
        PyContext::new();
    }
}

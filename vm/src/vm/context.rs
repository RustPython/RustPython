use crate::{
    builtins::{
        builtinfunc::{PyBuiltinFunction, PyBuiltinMethod, PyNativeFuncDef},
        bytes,
        getset::{IntoPyGetterFunc, IntoPySetterFunc, PyGetSet},
        object, pystr,
        pytype::PyAttributes,
        PyBaseException, PyDict, PyDictRef, PyEllipsis, PyFloat, PyFrozenSet, PyInt, PyIntRef,
        PyList, PyListRef, PyNone, PyNotImplemented, PyStr, PyTuple, PyTupleRef, PyType, PyTypeRef,
    },
    exceptions,
    function::IntoPyNativeFunc,
    intern::{Internable, StringPool},
    pyclass::{PyClassImpl, StaticType},
    pyobject::{PyObjectPayload, PyObjectRef, PyPayload, PyRef, PyRefExact},
    types::{PyTypeFlags, PyTypeSlots, TypeZoo},
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;

#[derive(Debug, Clone)]
pub struct PyContext {
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub none: PyRef<PyNone>,
    pub empty_tuple: PyTupleRef,
    pub empty_frozenset: PyRef<PyFrozenSet>,
    pub ellipsis: PyRef<PyEllipsis>,
    pub not_implemented: PyRef<PyNotImplemented>,

    pub(crate) true_str: PyRef<PyStr>,
    pub(crate) false_str: PyRef<PyStr>,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyIntRef>,
    // there should only be exact objects of str in here, no non-strs and no subclasses
    pub(crate) string_pool: StringPool,
    pub(crate) slot_new_wrapper: PyObjectRef,
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

        let string_pool = StringPool::default();

        let new_str = unsafe { string_pool.intern("__new__", types.str_type.clone()) };
        let slot_new_wrapper = create_object(
            PyNativeFuncDef::new(PyType::__new__.into_func(), new_str.into_pyref()).into_function(),
            &types.builtin_function_or_method_type,
        )
        .into();

        let true_str = unsafe { string_pool.intern("True", types.str_type.clone()) }.into_pyref();
        let false_str = unsafe { string_pool.intern("False", types.str_type.clone()) }.into_pyref();

        let context = PyContext {
            true_value,
            false_value,
            none,
            empty_tuple,
            empty_frozenset,
            ellipsis,
            not_implemented,

            true_str,
            false_str,

            types,
            exceptions,
            int_cache_pool,
            string_pool,
            slot_new_wrapper,
        };
        TypeZoo::extend(&context);
        exceptions::ExceptionZoo::extend(&context);
        context
    }

    pub fn intern_string<S: Internable>(&self, s: S) -> PyRefExact<PyStr> {
        unsafe { self.string_pool.intern(s, self.types.str_type.clone()) }
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

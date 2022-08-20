use crate::{
    builtins::{
        builtinfunc::{PyBuiltinFunction, PyBuiltinMethod, PyNativeFuncDef},
        bytes,
        code::{self, PyCode},
        descriptor::{DescrObject, MemberDef, MemberDescrObject, MemberKind},
        getset::PyGetSet,
        object, pystr,
        type_::PyAttributes,
        PyBaseException, PyBytes, PyComplex, PyDict, PyDictRef, PyEllipsis, PyFloat, PyFrozenSet,
        PyInt, PyIntRef, PyList, PyListRef, PyNone, PyNotImplemented, PyStr, PyStrInterned,
        PyTuple, PyTupleRef, PyType, PyTypeRef,
    },
    class::{PyClassImpl, StaticType},
    common::rc::PyRc,
    exceptions,
    function::{IntoPyGetterFunc, IntoPyNativeFunc, IntoPySetterFunc},
    intern::{Internable, MaybeInterned, StringPool},
    object::{Py, PyObjectPayload, PyObjectRef, PyPayload, PyRef},
    types::{PyTypeFlags, PyTypeSlots, TypeZoo},
    PyResult, VirtualMachine,
};
use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;

#[derive(Debug)]
pub struct Context {
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub none: PyRef<PyNone>,
    pub empty_tuple: PyTupleRef,
    pub empty_frozenset: PyRef<PyFrozenSet>,
    pub empty_str: PyRef<PyStr>,
    pub empty_bytes: PyRef<PyBytes>,
    pub ellipsis: PyRef<PyEllipsis>,
    pub not_implemented: PyRef<PyNotImplemented>,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyIntRef>,
    // there should only be exact objects of str in here, no non-strs and no subclasses
    pub(crate) string_pool: StringPool,
    pub(crate) slot_new_wrapper: PyObjectRef,
    pub names: ConstName,
}

macro_rules! declare_const_name {
    ($($name:ident,)*) => {
        #[derive(Debug, Clone, Copy)]
        #[allow(non_snake_case)]
        pub struct ConstName {
            $(pub $name: &'static PyStrInterned,)*
        }

        impl ConstName {
            unsafe fn new(pool: &StringPool, typ: &PyTypeRef) -> Self {
                Self {
                    $($name: pool.intern(stringify!($name), typ.clone()),)*
                }
            }
        }
    }
}

declare_const_name! {
    True,
    False,

    // magic methods
    __abs__,
    __abstractmethods__,
    __add__,
    __aenter__,
    __aexit__,
    __aiter__,
    __all__,
    __and__,
    __anext__,
    __annotations__,
    __args__,
    __await__,
    __bases__,
    __bool__,
    __build_class__,
    __builtins__,
    __bytes__,
    __call__,
    __ceil__,
    __cformat__,
    __class__,
    __classcell__,
    __class_getitem__,
    __complex__,
    __contains__,
    __copy__,
    __deepcopy__,
    __del__,
    __delattr__,
    __delete__,
    __delitem__,
    __dict__,
    __dir__,
    __div__,
    __divmod__,
    __doc__,
    __enter__,
    __eq__,
    __exit__,
    __file__,
    __float__,
    __floor__,
    __floordiv__,
    __format__,
    __fspath__,
    __ge__,
    __get__,
    __getattr__,
    __getattribute__,
    __getitem__,
    __gt__,
    __hash__,
    __iadd__,
    __iand__,
    __idiv__,
    __ifloordiv__,
    __ilshift__,
    __imatmul__,
    __imod__,
    __import__,
    __imul__,
    __index__,
    __init__,
    __init_subclass__,
    __instancecheck__,
    __int__,
    __invert__,
    __ior__,
    __ipow__,
    __irshift__,
    __isub__,
    __iter__,
    __itruediv__,
    __ixor__,
    __le__,
    __len__,
    __length_hint__,
    __lshift__,
    __lt__,
    __main__,
    __matmul__,
    __missing__,
    __mod__,
    __module__,
    __mro_entries__,
    __mul__,
    __name__,
    __ne__,
    __neg__,
    __new__,
    __next__,
    __or__,
    __orig_bases__,
    __orig_class__,
    __origin__,
    __parameters__,
    __pos__,
    __pow__,
    __prepare__,
    __qualname__,
    __radd__,
    __rand__,
    __rdiv__,
    __rdivmod__,
    __reduce__,
    __reduce_ex__,
    __repr__,
    __reversed__,
    __rfloordiv__,
    __rlshift__,
    __rmatmul__,
    __rmod__,
    __rmul__,
    __ror__,
    __round__,
    __rpow__,
    __rrshift__,
    __rshift__,
    __rsub__,
    __rtruediv__,
    __rxor__,
    __set__,
    __set_name__,
    __setattr__,
    __setitem__,
    __str__,
    __sub__,
    __subclasscheck__,
    __truediv__,
    __trunc__,
    __xor__,

    // common names
    _attributes,
    _fields,
    decode,
    encode,
    keys,
    items,
    values,
    update,
    copy,
    flush,
    close,
}

// Basic objects:
impl Context {
    pub const INT_CACHE_POOL_MIN: i32 = -5;
    pub const INT_CACHE_POOL_MAX: i32 = 256;

    pub fn genesis() -> &'static PyRc<Self> {
        rustpython_common::static_cell! {
            static CONTEXT: PyRc<Context>;
        }
        CONTEXT.get_or_init(|| PyRc::new(Self::init_genesis()))
    }

    fn init_genesis() -> Self {
        flame_guard!("init Context");
        let types = TypeZoo::init();
        let exceptions = exceptions::ExceptionZoo::init();

        #[inline]
        fn create_object<T: PyObjectPayload + PyPayload>(
            payload: T,
            cls: &'static Py<PyType>,
        ) -> PyRef<T> {
            PyRef::new_ref(payload, cls.to_owned(), None)
        }

        let none = create_object(PyNone, PyNone::static_type());
        let ellipsis = create_object(PyEllipsis, PyEllipsis::static_type());
        let not_implemented = create_object(PyNotImplemented, PyNotImplemented::static_type());

        let int_cache_pool = (Self::INT_CACHE_POOL_MIN..=Self::INT_CACHE_POOL_MAX)
            .map(|v| {
                PyRef::new_ref(
                    PyInt::from(BigInt::from(v)),
                    types.int_type.to_owned(),
                    None,
                )
            })
            .collect();

        let true_value = create_object(PyInt::from(1), types.bool_type);
        let false_value = create_object(PyInt::from(0), types.bool_type);

        let empty_tuple = create_object(
            PyTuple::new_unchecked(Vec::new().into_boxed_slice()),
            types.tuple_type,
        );
        let empty_frozenset = PyRef::new_ref(
            PyFrozenSet::default(),
            types.frozenset_type.to_owned(),
            None,
        );

        let string_pool = StringPool::default();
        let names = unsafe { ConstName::new(&string_pool, &types.str_type.to_owned()) };

        let slot_new_wrapper = create_object(
            PyNativeFuncDef::new(PyType::__new__.into_func(), names.__new__.to_owned())
                .into_function(),
            types.builtin_function_or_method_type,
        )
        .into();

        let empty_str = unsafe { string_pool.intern("", types.str_type.to_owned()) }.to_owned();
        let empty_bytes = create_object(PyBytes::from(Vec::new()), types.bytes_type);
        let context = Context {
            true_value,
            false_value,
            none,
            empty_tuple,
            empty_frozenset,
            empty_str,
            empty_bytes,

            ellipsis,
            not_implemented,

            types,
            exceptions,
            int_cache_pool,
            string_pool,
            slot_new_wrapper,
            names,
        };
        TypeZoo::extend(&context);
        exceptions::ExceptionZoo::extend(&context);
        context
    }

    pub fn intern_str<S: Internable>(&self, s: S) -> &'static PyStrInterned {
        unsafe { self.string_pool.intern(s, self.types.str_type.to_owned()) }
    }

    pub fn interned_str<S: MaybeInterned + ?Sized>(&self, s: &S) -> Option<&'static PyStrInterned> {
        self.string_pool.interned(s)
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
            if (Self::INT_CACHE_POOL_MIN..=Self::INT_CACHE_POOL_MAX).contains(&i) {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyRef::new_ref(PyInt::from(i), self.types.int_type.to_owned(), None)
    }

    #[inline]
    pub fn new_bigint(&self, i: &BigInt) -> PyIntRef {
        if let Some(i) = i.to_i32() {
            if (Self::INT_CACHE_POOL_MIN..=Self::INT_CACHE_POOL_MAX).contains(&i) {
                let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
                return self.int_cache_pool[inner_idx].clone();
            }
        }
        PyRef::new_ref(PyInt::from(i.clone()), self.types.int_type.to_owned(), None)
    }

    #[inline]
    pub fn new_float(&self, value: f64) -> PyRef<PyFloat> {
        PyRef::new_ref(PyFloat::from(value), self.types.float_type.to_owned(), None)
    }

    #[inline]
    pub fn new_complex(&self, value: Complex64) -> PyRef<PyComplex> {
        PyRef::new_ref(
            PyComplex::from(value),
            self.types.complex_type.to_owned(),
            None,
        )
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
        base: PyTypeRef,
        slots: PyTypeSlots,
    ) -> PyTypeRef {
        let mut attrs = PyAttributes::default();
        if let Some(module) = module {
            attrs.insert(identifier!(self, __module__), self.new_str(module).into());
        };
        PyType::new_ref(
            name,
            vec![base],
            attrs,
            slots,
            self.types.type_type.to_owned(),
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
            vec![self.exceptions.exception_type.to_owned()]
        };
        let mut attrs = PyAttributes::default();
        attrs.insert(identifier!(self, __module__), self.new_str(module).into());

        PyType::new_ref(
            name,
            bases,
            attrs,
            PyBaseException::make_slots(),
            self.types.type_type.to_owned(),
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

    #[inline]
    pub fn new_member(
        &self,
        name: &str,
        getter: fn(PyObjectRef, &VirtualMachine) -> PyResult,
        class: &'static Py<PyType>,
    ) -> PyRef<MemberDescrObject> {
        let member_def = MemberDef {
            name: name.to_owned(),
            kind: MemberKind::ObjectEx,
            getter,
            doc: None,
        };
        let member_descriptor = MemberDescrObject {
            common: DescrObject {
                typ: class.to_owned(),
                name: name.to_owned(),
                qualname: PyRwLock::new(None),
            },
            member: member_def,
        };

        PyRef::new_ref(
            member_descriptor,
            self.types.member_descriptor_type.to_owned(),
            None,
        )
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
        class: &'static Py<PyType>,
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
        class: &'static Py<PyType>,
        f: F,
    ) -> PyRef<PyGetSet>
    where
        F: IntoPyGetterFunc<T>,
    {
        PyRef::new_ref(
            PyGetSet::new(name.into(), class).with_get(f),
            self.types.getset_type.to_owned(),
            None,
        )
    }

    pub fn new_getset<G, S, T, U>(
        &self,
        name: impl Into<String>,
        class: &'static Py<PyType>,
        g: G,
        s: S,
    ) -> PyRef<PyGetSet>
    where
        G: IntoPyGetterFunc<T>,
        S: IntoPySetterFunc<U>,
    {
        PyRef::new_ref(
            PyGetSet::new(name.into(), class).with_get(g).with_set(s),
            self.types.getset_type.to_owned(),
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

    pub fn new_code(&self, code: impl code::IntoCodeObject) -> PyRef<PyCode> {
        let code = code.into_codeobj(self);
        PyRef::new_ref(PyCode { code }, self.types.code_type.to_owned(), None)
    }
}

impl AsRef<Context> for Context {
    fn as_ref(&self) -> &Self {
        self
    }
}

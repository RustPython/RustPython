use crate::{
    PyResult, VirtualMachine,
    builtins::{
        PyByteArray, PyBytes, PyComplex, PyDict, PyDictRef, PyEllipsis, PyFloat, PyFrozenSet,
        PyInt, PyIntRef, PyList, PyListRef, PyNone, PyNotImplemented, PyStr, PyStrInterned,
        PyTuple, PyTupleRef, PyType, PyTypeRef,
        bool_::PyBool,
        code::{self, PyCode},
        descriptor::{
            MemberGetter, MemberKind, MemberSetter, MemberSetterFunc, PyDescriptorOwned,
            PyMemberDef, PyMemberDescriptor,
        },
        getset::PyGetSet,
        object, pystr,
        type_::PyAttributes,
    },
    class::StaticType,
    common::rc::PyRc,
    exceptions,
    function::{
        HeapMethodDef, IntoPyGetterFunc, IntoPyNativeFn, IntoPySetterFunc, PyMethodDef,
        PyMethodFlags,
    },
    intern::{InternableString, MaybeInternedString, StringPool},
    object::{Py, PyObjectPayload, PyObjectRef, PyPayload, PyRef},
    types::{PyTypeFlags, PyTypeSlots, TypeZoo},
};
use malachite_bigint::BigInt;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;

#[derive(Debug)]
pub struct Context {
    pub true_value: PyRef<PyBool>,
    pub false_value: PyRef<PyBool>,
    pub none: PyRef<PyNone>,
    pub empty_tuple: PyTupleRef,
    pub empty_frozenset: PyRef<PyFrozenSet>,
    pub empty_str: &'static PyStrInterned,
    pub empty_bytes: PyRef<PyBytes>,
    pub ellipsis: PyRef<PyEllipsis>,
    pub not_implemented: PyRef<PyNotImplemented>,

    pub typing_no_default: PyRef<crate::stdlib::typing::NoDefault>,

    pub types: TypeZoo,
    pub exceptions: exceptions::ExceptionZoo,
    pub int_cache_pool: Vec<PyIntRef>,
    // there should only be exact objects of str in here, no non-str objects and no subclasses
    pub(crate) string_pool: StringPool,
    pub(crate) slot_new_wrapper: PyMethodDef,
    pub names: ConstName,
}

macro_rules! declare_const_name {
    ($($name:ident$(: $s:literal)?,)*) => {
        #[derive(Debug, Clone, Copy)]
        #[allow(non_snake_case)]
        pub struct ConstName {
            $(pub $name: &'static PyStrInterned,)*
        }

        impl ConstName {
            unsafe fn new(pool: &StringPool, typ: &Py<PyType>) -> Self {
                Self {
                    $($name: unsafe { pool.intern(declare_const_name!(@string $name $($s)?), typ.to_owned()) },)*
                }
            }
        }
    };
    (@string $name:ident) => { stringify!($name) };
    (@string $name:ident $string:literal) => { $string };
}

declare_const_name! {
    True,
    False,
    None,
    NotImplemented,
    Ellipsis,

    // magic methods
    __abs__,
    __abstractmethods__,
    __add__,
    __aenter__,
    __aexit__,
    __aiter__,
    __alloc__,
    __all__,
    __and__,
    __anext__,
    __annotate__,
    __annotate_func__,
    __annotations__,
    __annotations_cache__,
    __args__,
    __await__,
    __bases__,
    __bool__,
    __build_class__,
    __builtins__,
    __bytes__,
    __cached__,
    __call__,
    __ceil__,
    __cformat__,
    __class__,
    __class_getitem__,
    __classcell__,
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
    __getformat__,
    __getitem__,
    __getnewargs__,
    __getnewargs_ex__,
    __getstate__,
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
    __jit__,  // RustPython dialect
    __le__,
    __len__,
    __length_hint__,
    __lshift__,
    __lt__,
    __main__,
    __match_args__,
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
    __objclass__,
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
    __setattr__,
    __setitem__,
    __setstate__,
    __set_name__,
    __slots__,
    __slotnames__,
    __str__,
    __sub__,
    __subclasscheck__,
    __subclasshook__,
    __subclasses__,
    __sizeof__,
    __truediv__,
    __trunc__,
    __type_params__,
    __typing_subst__,
    __typing_is_unpacked_typevartuple__,
    __typing_prepare_subst__,
    __typing_unpacked_tuple_args__,
    __xor__,

    // common names
    _attributes,
    _fields,
    _showwarnmsg,
    backslashreplace,
    close,
    copy,
    decode,
    encode,
    flush,
    ignore,
    items,
    keys,
    modules,
    n_fields,
    n_sequence_fields,
    n_unnamed_fields,
    namereplace,
    replace,
    strict,
    surrogateescape,
    surrogatepass,
    update,
    utf_8: "utf-8",
    values,
    version,
    WarningMessage,
    xmlcharrefreplace,
}

// Basic objects:
impl Context {
    pub const INT_CACHE_POOL_RANGE: core::ops::RangeInclusive<i32> = (-5)..=256;
    const INT_CACHE_POOL_MIN: i32 = *Self::INT_CACHE_POOL_RANGE.start();

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
        fn create_object<T: PyObjectPayload>(payload: T, cls: &'static Py<PyType>) -> PyRef<T> {
            PyRef::new_ref(payload, cls.to_owned(), None)
        }

        let none = create_object(PyNone, PyNone::static_type());
        let ellipsis = create_object(PyEllipsis, PyEllipsis::static_type());
        let not_implemented = create_object(PyNotImplemented, PyNotImplemented::static_type());

        let typing_no_default = create_object(
            crate::stdlib::typing::NoDefault,
            crate::stdlib::typing::NoDefault::static_type(),
        );

        let int_cache_pool = Self::INT_CACHE_POOL_RANGE
            .map(|v| {
                PyRef::new_ref(
                    PyInt::from(BigInt::from(v)),
                    types.int_type.to_owned(),
                    None,
                )
            })
            .collect();

        let true_value = create_object(PyBool(PyInt::from(1)), types.bool_type);
        let false_value = create_object(PyBool(PyInt::from(0)), types.bool_type);

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
        let names = unsafe { ConstName::new(&string_pool, types.str_type) };

        let slot_new_wrapper = PyMethodDef::new_const(
            names.__new__.as_str(),
            PyType::__new__,
            PyMethodFlags::METHOD,
            None,
        );

        let empty_str = unsafe { string_pool.intern("", types.str_type.to_owned()) };
        let empty_bytes = create_object(PyBytes::from(Vec::new()), types.bytes_type);
        Self {
            true_value,
            false_value,
            none,
            empty_tuple,
            empty_frozenset,
            empty_str,
            empty_bytes,
            ellipsis,

            not_implemented,
            typing_no_default,

            types,
            exceptions,
            int_cache_pool,
            string_pool,
            slot_new_wrapper,
            names,
        }
    }

    pub fn intern_str<S: InternableString>(&self, s: S) -> &'static PyStrInterned {
        unsafe { self.string_pool.intern(s, self.types.str_type.to_owned()) }
    }

    pub fn interned_str<S: MaybeInternedString + ?Sized>(
        &self,
        s: &S,
    ) -> Option<&'static PyStrInterned> {
        self.string_pool.interned(s)
    }

    #[inline(always)]
    pub fn none(&self) -> PyObjectRef {
        self.none.clone().into()
    }

    #[inline(always)]
    pub fn not_implemented(&self) -> PyObjectRef {
        self.not_implemented.clone().into()
    }

    #[inline]
    pub fn empty_tuple_typed<T>(&self) -> &Py<PyTuple<T>> {
        let py: &Py<PyTuple> = &self.empty_tuple;
        unsafe { core::mem::transmute(py) }
    }

    // universal pyref constructor
    pub fn new_pyref<T, P>(&self, value: T) -> PyRef<P>
    where
        T: Into<P>,
        P: PyPayload + core::fmt::Debug,
    {
        value.into().into_ref(self)
    }

    // shortcuts for common type

    #[inline]
    pub fn new_int<T: Into<BigInt> + ToPrimitive>(&self, i: T) -> PyIntRef {
        if let Some(i) = i.to_i32()
            && Self::INT_CACHE_POOL_RANGE.contains(&i)
        {
            let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
            return self.int_cache_pool[inner_idx].clone();
        }
        PyInt::from(i).into_ref(self)
    }

    #[inline]
    pub fn new_bigint(&self, i: &BigInt) -> PyIntRef {
        if let Some(i) = i.to_i32()
            && Self::INT_CACHE_POOL_RANGE.contains(&i)
        {
            let inner_idx = (i - Self::INT_CACHE_POOL_MIN) as usize;
            return self.int_cache_pool[inner_idx].clone();
        }
        PyInt::from(i.clone()).into_ref(self)
    }

    #[inline]
    pub fn new_float(&self, value: f64) -> PyRef<PyFloat> {
        PyFloat::from(value).into_ref(self)
    }

    #[inline]
    pub fn new_complex(&self, value: Complex64) -> PyRef<PyComplex> {
        PyComplex::from(value).into_ref(self)
    }

    #[inline]
    pub fn new_str(&self, s: impl Into<pystr::PyStr>) -> PyRef<PyStr> {
        s.into().into_ref(self)
    }

    pub fn interned_or_new_str<S, M>(&self, s: S) -> PyRef<PyStr>
    where
        S: Into<PyStr> + AsRef<M>,
        M: MaybeInternedString,
    {
        match self.interned_str(s.as_ref()) {
            Some(s) => s.to_owned(),
            None => self.new_str(s),
        }
    }

    #[inline]
    pub fn new_bytes(&self, data: Vec<u8>) -> PyRef<PyBytes> {
        PyBytes::from(data).into_ref(self)
    }

    #[inline]
    pub fn new_bytearray(&self, data: Vec<u8>) -> PyRef<PyByteArray> {
        PyByteArray::from(data).into_ref(self)
    }

    #[inline(always)]
    pub fn new_bool(&self, b: bool) -> PyRef<PyBool> {
        let value = if b {
            &self.true_value
        } else {
            &self.false_value
        };
        value.to_owned()
    }

    #[inline(always)]
    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyTupleRef {
        PyTuple::new_ref(elements, self)
    }

    #[inline(always)]
    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyListRef {
        PyList::from(elements).into_ref(self)
    }

    #[inline(always)]
    pub fn new_dict(&self) -> PyDictRef {
        PyDict::default().into_ref(self)
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
        PyType::new_heap(
            name,
            vec![base],
            attrs,
            slots,
            self.types.type_type.to_owned(),
            self,
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

        let interned_name = self.intern_str(name);
        let slots = PyTypeSlots {
            name: interned_name.as_str(),
            basicsize: 0,
            flags: PyTypeFlags::heap_type_flags() | PyTypeFlags::HAS_DICT,
            ..PyTypeSlots::default()
        };
        PyType::new_heap(
            name,
            bases,
            attrs,
            slots,
            self.types.type_type.to_owned(),
            self,
        )
        .unwrap()
    }

    pub fn new_method_def<F, FKind>(
        &self,
        name: &'static str,
        f: F,
        flags: PyMethodFlags,
        doc: Option<&'static str>,
    ) -> PyRef<HeapMethodDef>
    where
        F: IntoPyNativeFn<FKind>,
    {
        let def = PyMethodDef {
            name,
            func: Box::leak(Box::new(f.into_func())),
            flags,
            doc,
        };
        let payload = HeapMethodDef::new(def);
        PyRef::new_ref(payload, self.types.method_def.to_owned(), None)
    }

    #[inline]
    pub fn new_member(
        &self,
        name: &str,
        member_kind: MemberKind,
        getter: fn(&VirtualMachine, PyObjectRef) -> PyResult,
        setter: MemberSetterFunc,
        class: &'static Py<PyType>,
    ) -> PyRef<PyMemberDescriptor> {
        let member_def = PyMemberDef {
            name: name.to_owned(),
            kind: member_kind,
            getter: MemberGetter::Getter(getter),
            setter: MemberSetter::Setter(setter),
            doc: None,
        };
        let member_descriptor = PyMemberDescriptor {
            common: PyDescriptorOwned {
                typ: class.to_owned(),
                name: self.intern_str(name),
                qualname: PyRwLock::new(None),
            },
            member: member_def,
        };
        member_descriptor.into_ref(self)
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
        let name = name.into();
        let getset = PyGetSet::new(name, class).with_get(f);
        PyRef::new_ref(getset, self.types.getset_type.to_owned(), None)
    }

    pub fn new_static_getset<G, S, T, U>(
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
        let name = name.into();
        let getset = PyGetSet::new(name, class).with_get(g).with_set(s);
        PyRef::new_ref(getset, self.types.getset_type.to_owned(), None)
    }

    /// Creates a new `PyGetSet` with a heap type.
    ///
    /// # Safety
    /// In practice, this constructor is safe because a getset is always owned by its `class` type.
    /// However, it can be broken if used unconventionally.
    pub unsafe fn new_getset<G, S, T, U>(
        &self,
        name: impl Into<String>,
        class: &Py<PyType>,
        g: G,
        s: S,
    ) -> PyRef<PyGetSet>
    where
        G: IntoPyGetterFunc<T>,
        S: IntoPySetterFunc<U>,
    {
        let class = unsafe { &*(class as *const _) };
        self.new_static_getset(name, class, g, s)
    }

    pub fn new_base_object(&self, class: PyTypeRef, dict: Option<PyDictRef>) -> PyObjectRef {
        debug_assert_eq!(
            class.slots.flags.contains(PyTypeFlags::HAS_DICT),
            dict.is_some()
        );
        PyRef::new_ref(object::PyBaseObject, class, dict).into()
    }

    pub fn new_code(&self, code: impl code::IntoCodeObject) -> PyRef<PyCode> {
        let code = code.into_code_object(self);
        PyRef::new_ref(PyCode { code }, self.types.code_type.to_owned(), None)
    }
}

impl AsRef<Self> for Context {
    fn as_ref(&self) -> &Self {
        self
    }
}

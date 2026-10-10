use super::{PyStr, PyStrInterned, PyTuple, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyTypeRef, builtin_func::PyNativeFunction, type_},
    class::PyClassImpl,
    common::hash::PyHash,
    convert::{ToPyObject, ToPyResult},
    function::{
        Callee, FuncArgs, ItemDoc, PyMethodDef, PyMethodFlags, PySetterValue, PySsize, plain_doc,
    },
    protocol::{PyNumberBinaryFunc, PyNumberTernaryFunc, PyNumberUnaryFunc},
    types::{
        Callable, Comparable, DelFunc, DescrGetFunc, DescrSetFunc, GenericMethod, GetDescriptor,
        GetattroFunc, HashFunc, Hashable, InitFunc, IterFunc, IterNextFunc, MapAssSubscriptFunc,
        MapLenFunc, MapSubscriptFunc, PyComparisonOp, Representable, RichCompareFunc,
        SeqAssItemFunc, SeqConcatFunc, SeqContainsFunc, SeqItemFunc, SeqLenFunc, SeqRepeatFunc,
        SetattroFunc, StringifyFunc,
    },
};
use core::mem::{align_of, size_of};
use rustpython_common::lock::PyRwLock;

#[derive(Debug)]
pub struct PyDescriptor {
    pub typ: &'static Py<PyType>,
    pub name: &'static PyStrInterned,
    pub qualname: PyRwLock<Option<String>>,
}

#[derive(Debug)]
pub struct PyDescriptorOwned {
    pub typ: PyRef<PyType>,
    pub name: &'static PyStrInterned,
    pub qualname: PyRwLock<Option<String>>,
}

#[pyclass(name = "method_descriptor", module = false)]
pub struct PyMethodDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptor,
    pub method: &'static PyMethodDef,
    // vectorcall: vector_call_func,
    /// Prevent HeapMethodDef from being freed while this descriptor references it
    pub(crate) _method_def_owner: Option<PyObjectRef>,
}

impl PyMethodDescriptor {
    pub fn new(method: &'static PyMethodDef, typ: &'static Py<PyType>, ctx: &Context) -> Self {
        Self {
            common: PyDescriptor {
                typ,
                name: ctx.intern_str(method.name),
                qualname: PyRwLock::new(None),
            },
            method,
            _method_def_owner: None,
        }
    }
}

impl PyPayload for PyMethodDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.method_descriptor_type
    }
}

impl core::fmt::Debug for PyMethodDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "method descriptor for '{}'", self.common.name)
    }
}

impl GetDescriptor for PyMethodDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm).unwrap();
        let bound = match obj {
            Some(obj) => {
                method_descr_typecheck(descr, obj, vm)?;
                if descr.method.flags.contains(PyMethodFlags::METHOD) {
                    if cls
                        .as_ref()
                        .is_none_or(|c| c.fast_isinstance(vm.ctx.types.type_type))
                    {
                        obj.to_owned()
                    } else {
                        return Err(vm.new_type_error(format!(
                            "descriptor '{}' needs a type, not '{}', as arg 2",
                            descr.common.name.as_str(),
                            obj.class().name()
                        )));
                    }
                } else if descr.method.flags.contains(PyMethodFlags::CLASS) {
                    obj.class().to_owned().into()
                } else {
                    obj.to_owned()
                }
            }
            None if descr.method.flags.contains(PyMethodFlags::CLASS) => cls.unwrap().to_owned(),
            None => return Ok(zelf.to_owned()),
        };
        Ok(descr.bind(bound, &vm.ctx).into())
    }
}

impl Callable for PyMethodDescriptor {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        method_descr_check_self(zelf, args.args.first().map(|obj| &**obj), vm)?;
        if !args.kwargs.is_empty() && !zelf.method.flags.contains(PyMethodFlags::KEYWORDS) {
            return Err(method_descr_no_keywords_error(zelf, vm));
        }
        (zelf.method.func)(
            vm,
            args,
            Callee::named(zelf.method.name).with_instance_arg(true),
        )
    }
}

impl PyMethodDescriptor {
    pub fn bind(&self, obj: PyObjectRef, ctx: &Context) -> PyRef<PyNativeFunction> {
        self.method.build_bound_function(ctx, obj)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(METHOD_DESCRIPTOR, DISALLOW_INSTANTIATION)
)]
impl Py<PyMethodDescriptor> {
    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.common.typ.name(), self.common.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        type_::rendered_item_doc(self.method.name, self.method.item_doc())
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_text_signature_from_internal_doc(self.method.name, doc)
    }

    #[pymethod]
    fn __reduce__(
        &self,
        vm: &VirtualMachine,
    ) -> (Option<PyObjectRef>, (Option<PyObjectRef>, &'static str)) {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm).ok();
        let classname = vm.builtins.get_attr(&self.common.typ.__name__(vm), vm).ok();
        (builtins_getattr, (classname, self.method.name))
    }
}

impl Representable for PyMethodDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method '{}' of '{}' objects>",
            zelf.method.name,
            zelf.common.typ.name()
        ))
    }
}

// METH_CLASS descriptors. Same layout as method_descriptor; a distinct type.
#[pyclass(name = "classmethod_descriptor", module = false)]
pub struct PyClassMethodDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptor,
    pub method: &'static PyMethodDef,
    pub(crate) _method_def_owner: Option<PyObjectRef>,
}

impl PyClassMethodDescriptor {
    pub fn new(method: &'static PyMethodDef, typ: &'static Py<PyType>, ctx: &Context) -> Self {
        Self {
            common: PyDescriptor {
                typ,
                name: ctx.intern_str(method.name),
                qualname: PyRwLock::new(None),
            },
            method,
            _method_def_owner: None,
        }
    }

    pub fn bind(&self, obj: PyObjectRef, ctx: &Context) -> PyRef<PyNativeFunction> {
        self.method.build_bound_function(ctx, obj)
    }
}

impl PyPayload for PyClassMethodDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.classmethod_descriptor_type
    }
}

impl core::fmt::Debug for PyClassMethodDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "classmethod descriptor for '{}'", self.common.name)
    }
}

impl GetDescriptor for PyClassMethodDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm).unwrap();
        let type_obj = match cls {
            Some(typ) => typ.to_owned(),
            None => match obj {
                Some(o) => o.class().to_owned().into(),
                None => {
                    return Err(vm.new_type_error(format!(
                        "descriptor '{}' for type '{}' needs either an object or a type",
                        descr.common.name,
                        descr.common.typ.name()
                    )));
                }
            },
        };
        if !type_obj.fast_isinstance(vm.ctx.types.type_type) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' for type '{}' needs a type, not '{}' as arg 2",
                descr.common.name,
                descr.common.typ.name(),
                type_obj.class().name()
            )));
        }
        let typ = type_obj.downcast::<PyType>().map_err(|obj| {
            vm.new_type_error(format!(
                "descriptor '{}' for type '{}' needs a type, not '{}' as arg 2",
                descr.common.name,
                descr.common.typ.name(),
                obj.class().name()
            ))
        })?;
        if !typ.fast_issubclass(descr.common.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a subtype of '{}' but received '{}'",
                descr.common.name,
                descr.common.typ.name(),
                typ.name()
            )));
        }
        Ok(descr.bind(typ.into(), &vm.ctx).into())
    }
}

impl Callable for PyClassMethodDescriptor {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let Some(owner) = args.args.first().cloned() else {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' of '{}' object needs an argument",
                zelf.method.name,
                zelf.common.typ.name()
            )));
        };
        let bound = Self::descr_get(zelf.as_object(), None, Some(&owner), vm)?;
        args.args.remove(0);
        bound.call(args, vm)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(DISALLOW_INSTANTIATION)
)]
impl Py<PyClassMethodDescriptor> {
    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.common.typ.name(), self.common.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        type_::rendered_item_doc(self.method.name, self.method.item_doc())
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_text_signature_from_internal_doc(self.method.name, doc)
    }
}

impl Representable for PyClassMethodDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method '{}' of '{}' objects>",
            zelf.method.name,
            zelf.common.typ.name()
        ))
    }
}

/// Member type. Discriminants match `PyMemberDef.type`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum MemberKind {
    /// `short`. Writable cells are `AtomicI16`.
    Short = 0,
    Int = 1,
    /// `long` (`c_long`). Writable cells are the atomic of that width.
    Long = 2,
    Double = 4,
    /// `char *`. The slot is a pointer to a NUL-terminated string. Null loads
    /// as `None`. Assignment is rejected.
    String = 5,
    Object = 6,
    /// `unsigned char`. Writable cells are `AtomicU8`.
    UByte = 9,
    /// `unsigned short`. Writable cells are `AtomicU16`.
    UShort = 10,
    Uint = 11,
    /// `unsigned long` (`c_ulong`). Writable cells are the atomic of that width.
    ULong = 12,
    Bool = 14,
    ObjectEx = 16,
    /// `long long`. Writable cells are `AtomicI64`.
    LongLong = 17,
    /// `unsigned long long`. Writable cells are `AtomicU64`.
    ULongLong = 18,
    /// `Py_ssize_t`. Writable cells are `AtomicIsize`.
    PySsizeT = 19,
}

impl MemberKind {
    const fn from_i64() -> Self {
        if size_of::<core::ffi::c_long>() == 8 {
            Self::Long
        } else {
            Self::LongLong
        }
    }

    const fn from_u64() -> Self {
        if size_of::<core::ffi::c_ulong>() == 8 {
            Self::ULong
        } else {
            Self::ULongLong
        }
    }

    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Short),
            1 => Some(Self::Int),
            2 => Some(Self::Long),
            4 => Some(Self::Double),
            5 => Some(Self::String),
            6 => Some(Self::Object),
            9 => Some(Self::UByte),
            10 => Some(Self::UShort),
            11 => Some(Self::Uint),
            12 => Some(Self::ULong),
            14 => Some(Self::Bool),
            16 => Some(Self::ObjectEx),
            17 => Some(Self::LongLong),
            18 => Some(Self::ULongLong),
            19 => Some(Self::PySsizeT),
            _ => None,
        }
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(transparent)]
    pub struct PyMemberFlags: i32 {
        const READONLY = 1;
        const AUDIT_READ = 2;
        // `_Py_WRITE_RESTRICTED` (4) is deprecated. The bit is reserved and must not be reused.
        const RELATIVE_OFFSET = 8;
        /// The field is atomic storage (`Atomic*` or `AtomicPyTypeFlags`).
        ///
        /// Not a public member flag. The defined flags are `Py_READONLY` (1),
        /// `Py_AUDIT_READ` (2), `_Py_WRITE_RESTRICTED` (4, deprecated, do not reuse),
        /// and `Py_RELATIVE_OFFSET` (8). This bit is set only by `#[pymember]`.
        /// Extension members leave it clear: a readonly extension member is a plain
        /// load, and a writable one keeps an atomic access.
        const ATOMIC = 0x10;
    }
}

/// Kind of a `#[pymember]` field. The macro reads [`MemberLayout::KIND`].
#[doc(hidden)]
pub trait MemberLayout {
    const KIND: MemberKind;
    /// `true` when the field is an atomic cell, not a plain integer or pointer.
    const ATOMIC: bool = false;
}

/// Writable `#[pymember]` field. Only an atomic cell may change after publication.
#[doc(hidden)]
pub trait MemberCell: MemberLayout {}

/// `KIND` of the field named by `probe`. `probe` is not called.
#[doc(hidden)]
#[must_use]
pub const fn member_kind_of<T: MemberLayout, Owner>(
    _: for<'a> fn(&'a Owner) -> &'a T,
) -> MemberKind {
    <T as MemberLayout>::KIND
}

/// [`MemberLayout::ATOMIC`] of the field named by `probe`. `probe` is not called.
#[doc(hidden)]
#[must_use]
pub const fn member_atomic_of<T: MemberLayout, Owner>(_: for<'a> fn(&'a Owner) -> &'a T) -> bool {
    <T as MemberLayout>::ATOMIC
}

impl MemberLayout for i16 {
    const KIND: MemberKind = MemberKind::Short;
}
impl MemberLayout for core::sync::atomic::AtomicI16 {
    const KIND: MemberKind = MemberKind::Short;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicI16 {}

impl MemberLayout for u16 {
    const KIND: MemberKind = MemberKind::UShort;
}
impl MemberLayout for core::sync::atomic::AtomicU16 {
    const KIND: MemberKind = MemberKind::UShort;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicU16 {}

impl MemberLayout for u8 {
    const KIND: MemberKind = MemberKind::UByte;
}
impl MemberLayout for core::sync::atomic::AtomicU8 {
    const KIND: MemberKind = MemberKind::UByte;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicU8 {}

// `i64` is `long` where `c_long` is 8 bytes, and `long long` otherwise.
impl MemberLayout for i64 {
    const KIND: MemberKind = MemberKind::from_i64();
}
impl MemberLayout for core::sync::atomic::AtomicI64 {
    const KIND: MemberKind = MemberKind::from_i64();
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicI64 {}

impl MemberLayout for u64 {
    const KIND: MemberKind = MemberKind::from_u64();
}
impl MemberLayout for core::sync::atomic::AtomicU64 {
    const KIND: MemberKind = MemberKind::from_u64();
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicU64 {}

/// Readonly `char *`. The word is the pointer, not the characters.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct CStrMember(*const core::ffi::c_char);

impl core::fmt::Debug for CStrMember {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CStrMember").field("ptr", &self.0).finish()
    }
}

impl CStrMember {
    #[must_use]
    pub const fn new(ptr: *const core::ffi::c_char) -> Self {
        Self(ptr)
    }

    #[must_use]
    pub const fn as_ptr(self) -> *const core::ffi::c_char {
        self.0
    }
}

// The pointer is not dereferenced by moving the wrapper.
unsafe impl Send for CStrMember {}
unsafe impl Sync for CStrMember {}

const _: () = assert!(
    size_of::<CStrMember>() == size_of::<*const core::ffi::c_char>()
        && align_of::<CStrMember>() == align_of::<*const core::ffi::c_char>()
);

impl MemberLayout for CStrMember {
    const KIND: MemberKind = MemberKind::String;
}

impl MemberLayout for bool {
    const KIND: MemberKind = MemberKind::Bool;
}
impl MemberLayout for core::sync::atomic::AtomicBool {
    const KIND: MemberKind = MemberKind::Bool;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicBool {}

impl MemberLayout for i32 {
    const KIND: MemberKind = MemberKind::Int;
}
impl MemberLayout for core::sync::atomic::AtomicI32 {
    const KIND: MemberKind = MemberKind::Int;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicI32 {}

impl MemberLayout for u32 {
    const KIND: MemberKind = MemberKind::Uint;
}
impl MemberLayout for core::sync::atomic::AtomicU32 {
    const KIND: MemberKind = MemberKind::Uint;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicU32 {}

// One object pointer. Readonly members may be a plain pointer. Writable
// members are an atomic cell: `PyAtomicRef<PyObject>` when the pointer is
// never null, or `PyAtomicRef<Option<PyObject>>` when it may be.
impl MemberLayout for crate::object::PyObjectCell {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}
impl<T> MemberLayout for PyRef<T> {
    const KIND: MemberKind = MemberKind::Object;
}
impl<T> MemberLayout for Option<PyRef<T>> {
    const KIND: MemberKind = MemberKind::Object;
}
impl MemberLayout for crate::object::PyAtomicRef<PyObject> {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}
// `PyAtomicRef<T>` is the same pointer-sized cell as `PyAtomicRef<PyObject>`
// (`PhantomData<T>` is zero-sized). Readonly object members may use it. The
// getter loads the slot as an object pointer. Writable cells stay
// `PyAtomicRef<PyObject>` or `PyAtomicRef<Option<PyObject>>` (`MemberCell`).
impl<T: PyPayload> MemberLayout for crate::object::PyAtomicRef<T> {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}
impl MemberLayout for crate::object::PyAtomicRef<Option<PyObject>> {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}
impl<T: PyPayload> MemberLayout for crate::object::PyAtomicRef<Option<T>> {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}
impl MemberLayout for &'static crate::builtins::PyStrInterned {
    const KIND: MemberKind = MemberKind::Object;
}
impl<T: PyPayload> MemberLayout for &'static Py<T> {
    const KIND: MemberKind = MemberKind::Object;
}
impl MemberCell for crate::object::PyAtomicRef<PyObject> {}
impl MemberCell for crate::object::PyAtomicRef<Option<PyObject>> {}

impl MemberLayout for f64 {
    const KIND: MemberKind = MemberKind::Double;
}
impl MemberLayout for crate::common::atomic::AtomicF64 {
    const KIND: MemberKind = MemberKind::Double;
    const ATOMIC: bool = true;
}
impl MemberCell for crate::common::atomic::AtomicF64 {}

impl MemberLayout for isize {
    const KIND: MemberKind = MemberKind::PySsizeT;
}
impl MemberLayout for core::sync::atomic::AtomicIsize {
    const KIND: MemberKind = MemberKind::PySsizeT;
    const ATOMIC: bool = true;
}
impl MemberCell for core::sync::atomic::AtomicIsize {}

// `usize` and `isize` are the same width. A readonly `Py_ssize_t` member may
// be a `usize` that is not written after publication and whose value fits in
// `isize` (the getter reads the bits as `isize`).
const _: () =
    assert!(size_of::<usize>() == size_of::<isize>() && align_of::<usize>() == align_of::<isize>());
impl MemberLayout for usize {
    const KIND: MemberKind = MemberKind::PySsizeT;
}

// Readonly `co_filename`. The field is `AtomicPtr<PyStrInterned>`: the pointer
// is never null, and `PyStrInterned` is `repr(transparent)` over `Py<PyStr>`,
// so the word is an object pointer. `member_get_one` reads an Object member
// through `PyAtomicRef<Option<PyObject>>` (`get_slot`). That cell is
// `AtomicPtr<u8>` when `threading` is on and `Cell<*mut u8>` otherwise; both
// are one pointer, and `AtomicPtr<PyStrInterned>` is one pointer in either
// build (`Atomic<*mut T>` stores `Align8<*mut T>`, same size and alignment as
// the pointer). The load copies that word and increfs. Interned strings live
// for the process, and the slot does not own the reference.
const _: () = assert!(
    size_of::<core::sync::atomic::AtomicPtr<PyStrInterned>>()
        == size_of::<crate::object::PyAtomicRef<Option<PyObject>>>()
        && align_of::<core::sync::atomic::AtomicPtr<PyStrInterned>>()
            == align_of::<crate::object::PyAtomicRef<Option<PyObject>>>()
);
impl MemberLayout for core::sync::atomic::AtomicPtr<PyStrInterned> {
    const KIND: MemberKind = MemberKind::Object;
    const ATOMIC: bool = true;
}

/// Where `PyMemberDef.offset` points.
///
/// `Offset` is a byte offset from the object to the field.
/// `TupleItem` is a struct-sequence element index: the elements live in the
/// tuple payload, not as separately addressable fields.
#[derive(Clone, Copy, Debug)]
pub enum MemberAccess {
    Offset,
    TupleItem,
}

/// C layout of `PyMemberDef`: name pointer, int type, `Py_ssize_t` offset,
/// int flags, doc pointer. `size_of` of this is `type`'s `tp_itemsize`.
#[repr(C)]
#[allow(dead_code)]
pub struct PyMemberDefLayout {
    _name: *const core::ffi::c_char,
    _type: core::ffi::c_int,
    _offset: isize,
    _flags: PyMemberFlags,
    _doc: *const core::ffi::c_char,
}

const _: () = assert!(
    size_of::<PyMemberFlags>() == size_of::<core::ffi::c_int>()
        && align_of::<PyMemberFlags>() == align_of::<core::ffi::c_int>()
);

/// Same fields as `PyMemberDef`: name, type, offset, flags, doc.
pub struct PyMemberDef {
    pub name: String,
    pub kind: MemberKind,
    pub offset: isize,
    pub flags: PyMemberFlags,
    pub doc: ItemDoc,
}

impl PyMemberDef {
    pub(crate) fn readonly(&self) -> bool {
        self.flags.contains(PyMemberFlags::READONLY)
    }

    /// Atomic load when the field is atomic storage. Writable members are
    /// stored as cells even when an extension did not set [`PyMemberFlags::ATOMIC`].
    pub(crate) fn atomic_storage(&self) -> bool {
        self.flags.contains(PyMemberFlags::ATOMIC) || !self.readonly()
    }

    pub(crate) fn audit_read(&self) -> bool {
        self.flags.contains(PyMemberFlags::AUDIT_READ)
    }
}

impl core::fmt::Debug for PyMemberDef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyMemberDef")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("offset", &self.offset)
            .field("flags", &self.flags)
            .field("doc", &self.doc)
            .finish()
    }
}

/// Const-constructible member spec. Registered as a `PyMemberDef`.
#[derive(Clone, Copy)]
pub struct PyMemberSpec {
    pub name: &'static str,
    pub kind: MemberKind,
    pub offset: isize,
    pub flags: PyMemberFlags,
    pub doc: ItemDoc,
}

impl PyMemberSpec {
    /// Concatenate cfg-gated member groups into one table.
    #[must_use]
    pub const fn concat<const N: usize>(parts: &[&[Self]]) -> [Self; N] {
        const EMPTY: PyMemberSpec = PyMemberSpec {
            name: "",
            kind: MemberKind::Object,
            offset: 0,
            flags: PyMemberFlags::empty(),
            doc: ItemDoc::NONE,
        };
        let mut out = [EMPTY; N];
        let mut index = 0;
        let mut part_index = 0;
        while part_index < parts.len() {
            let part = parts[part_index];
            let mut item_index = 0;
            while item_index < part.len() {
                out[index] = part[item_index];
                index += 1;
                item_index += 1;
            }
            part_index += 1;
        }
        out
    }
}

// = PyMemberDescrObject
#[pyclass(name = "member_descriptor", module = false)]
#[derive(Debug)]
pub struct PyMemberDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptorOwned,
    pub member: PyMemberDef,
    pub access: MemberAccess,
}

impl PyMemberDescriptor {
    /// Byte offset of an object-pointer member. `None` for bool, float, and
    /// struct-sequence indexes, which slot specialization must not treat as cells.
    pub(crate) fn slot_offset(&self) -> Option<isize> {
        if matches!(self.access, MemberAccess::TupleItem) {
            return None;
        }
        match self.member.kind {
            MemberKind::Object | MemberKind::ObjectEx => Some(self.member.offset),
            MemberKind::Bool
            | MemberKind::Double
            | MemberKind::Int
            | MemberKind::Long
            | MemberKind::LongLong
            | MemberKind::Short
            | MemberKind::String
            | MemberKind::UByte
            | MemberKind::Uint
            | MemberKind::ULong
            | MemberKind::ULongLong
            | MemberKind::UShort
            | MemberKind::PySsizeT => None,
        }
    }

    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.member.audit_read() {
            vm.audit("object.__getattr__", || {
                (obj.clone(), vm.ctx.new_str(self.member.name.as_str()))
            })?;
        }
        match self.access {
            MemberAccess::Offset => member_get_one(&obj, self.member.offset, &self.member, vm),
            MemberAccess::TupleItem => {
                let index = self.member.offset as usize;
                let tuple = obj.downcast_ref::<PyTuple>().ok_or_else(|| {
                    vm.new_type_error("unexpected payload for struct sequence member")
                })?;
                tuple
                    .as_slice()
                    .get(index)
                    .cloned()
                    .ok_or_else(|| vm.new_index_error(format!("tuple index {index} out of range")))
            }
        }
    }

    fn set(
        &self,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.member.readonly() {
            return Err(vm.new_attribute_error("readonly attribute"));
        }
        match self.access {
            MemberAccess::Offset => {
                member_set_one(&obj, self.member.offset, &self.member, value, vm)
            }
            MemberAccess::TupleItem => Err(vm.new_attribute_error("readonly attribute")),
        }
    }
}

impl PyPayload for PyMemberDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.member_descriptor_type
    }
}

fn calculate_qualname(descr: &PyDescriptorOwned, vm: &VirtualMachine) -> PyResult<Option<String>> {
    if let Some(qualname) = vm.get_attribute_opt(descr.typ.as_object(), "__qualname__")? {
        let str = qualname.downcast::<PyStr>().map_err(|_| {
            vm.new_type_error("<descriptor>.__objclass__.__qualname__ is not a unicode object")
        })?;
        Ok(Some(format!("{}.{}", str, descr.name)))
    } else {
        Ok(None)
    }
}

#[pyclass(with(GetDescriptor, Representable), flags(DISALLOW_INSTANTIATION))]
impl Py<PyMemberDescriptor> {
    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        plain_doc(self.member.doc)
    }

    #[pygetset]
    fn __qualname__(&self, vm: &VirtualMachine) -> PyResult<Option<String>> {
        let qualname = self.common.qualname.read();
        Ok(if qualname.is_none() {
            drop(qualname);
            let calculated = calculate_qualname(&self.common, vm)?;
            calculated.clone_into(&mut self.common.qualname.write());
            calculated
        } else {
            qualname.to_owned()
        })
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm)?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                builtins_getattr,
                vm.ctx
                    .new_tuple(vec![
                        self.common.typ.clone().into(),
                        vm.ctx.new_str(self.common.name.as_str()).into(),
                    ])
                    .into(),
            ])
            .into())
    }

    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = PyMemberDescriptor::_as_pyref(zelf, vm)?;

        if !obj.class().fast_issubclass(&zelf.common.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' for '{}' objects doesn't apply to a '{}' object",
                zelf.common.name,
                zelf.common.typ.name(),
                obj.class().name()
            )));
        }

        zelf.set(obj, value, vm)
    }
}

fn member_addr(obj: &PyObject, offset: isize) -> *mut u8 {
    (obj as *const PyObject as *const u8).wrapping_add(offset as usize) as *mut u8
}

fn warn_member(vm: &VirtualMachine, message: &str) -> PyResult<()> {
    crate::warn::warn(
        vm.ctx.new_str(message).into(),
        Some(vm.ctx.exceptions.runtime_warning.to_owned()),
        1,
        None,
        vm,
    )
}

fn load_i32(obj: &PyObject, offset: isize, atomic: bool) -> i32 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicI32`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI32>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain int member addresses an `i32` that is not written
        // after publication. The field may be only 4-byte aligned.
        unsafe { addr.cast::<i32>().read() }
    }
}

fn load_i16(obj: &PyObject, offset: isize, atomic: bool) -> i16 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicI16`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI16>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain short member addresses an `i16` that is not written
        // after publication.
        unsafe { addr.cast::<i16>().read() }
    }
}

fn load_u16(obj: &PyObject, offset: isize, atomic: bool) -> u16 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicU16`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU16>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain unsigned short member addresses a `u16` that is not
        // written after publication.
        unsafe { addr.cast::<u16>().read() }
    }
}

fn load_u8(obj: &PyObject, offset: isize, atomic: bool) -> u8 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an `AtomicU8`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU8>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a readonly unsigned char member addresses a `u8` that is not
        // written after publication.
        unsafe { addr.cast::<u8>().read() }
    }
}

fn load_i64(obj: &PyObject, offset: isize, atomic: bool) -> i64 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicI64`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI64>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain `long long` member addresses an `i64` that is not
        // written after publication. The field may be only 4-byte aligned.
        unsafe { addr.cast::<i64>().read() }
    }
}

fn load_u64(obj: &PyObject, offset: isize, atomic: bool) -> u64 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicU64`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU64>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain `unsigned long long` member addresses a `u64` that is
        // not written after publication. The field may be only 4-byte aligned,
        // so this is a plain read and not an `AtomicU64` access.
        unsafe { addr.cast::<u64>().read() }
    }
}

#[allow(clippy::unnecessary_cast)] // `c_long` is `i32` or `i64`
fn load_c_long(obj: &PyObject, offset: isize, atomic: bool) -> core::ffi::c_long {
    let addr = member_addr(obj, offset);
    if !atomic {
        // SAFETY: a plain `long` member addresses a `c_long` that is not
        // written after publication.
        unsafe { addr.cast::<core::ffi::c_long>().read() }
    } else if size_of::<core::ffi::c_long>() == 8 {
        // SAFETY: a writable `long` member addresses an aligned `AtomicI64`.
        // `c_long` is 8 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI64>())
                .load(core::sync::atomic::Ordering::Relaxed) as core::ffi::c_long
        }
    } else {
        // SAFETY: a writable `long` member addresses an aligned `AtomicI32`.
        // `c_long` is 4 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI32>())
                .load(core::sync::atomic::Ordering::Relaxed) as core::ffi::c_long
        }
    }
}

#[allow(clippy::unnecessary_cast)] // `c_long` is `i32` or `i64`
fn store_c_long(obj: &PyObject, offset: isize, value: core::ffi::c_long) {
    let addr = member_addr(obj, offset);
    if size_of::<core::ffi::c_long>() == 8 {
        // SAFETY: a writable `long` member addresses an aligned `AtomicI64`.
        // `c_long` is 8 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI64>())
                .store(value as i64, core::sync::atomic::Ordering::Relaxed);
        }
    } else {
        // SAFETY: a writable `long` member addresses an aligned `AtomicI32`.
        // `c_long` is 4 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI32>())
                .store(value as i32, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[allow(clippy::unnecessary_cast)] // `c_ulong` is `u32` or `u64`
fn load_c_ulong(obj: &PyObject, offset: isize, atomic: bool) -> core::ffi::c_ulong {
    let addr = member_addr(obj, offset);
    if !atomic {
        // SAFETY: a plain `unsigned long` member addresses a `c_ulong` that is
        // not written after publication. A plain integer may be only 4-byte
        // aligned, so this is not an atomic access.
        unsafe { addr.cast::<core::ffi::c_ulong>().read() }
    } else if size_of::<core::ffi::c_ulong>() == 8 {
        // SAFETY: a writable `unsigned long` member addresses an aligned `AtomicU64`.
        // `c_ulong` is 8 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU64>())
                .load(core::sync::atomic::Ordering::Relaxed) as core::ffi::c_ulong
        }
    } else {
        // SAFETY: a writable `unsigned long` member addresses an aligned `AtomicU32`.
        // `c_ulong` is 4 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU32>())
                .load(core::sync::atomic::Ordering::Relaxed) as core::ffi::c_ulong
        }
    }
}

#[allow(clippy::unnecessary_cast)] // `c_ulong` is `u32` or `u64`
fn store_c_ulong(obj: &PyObject, offset: isize, value: core::ffi::c_ulong) {
    let addr = member_addr(obj, offset);
    if size_of::<core::ffi::c_ulong>() == 8 {
        // SAFETY: a writable `unsigned long` member addresses an aligned `AtomicU64`.
        // `c_ulong` is 8 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU64>())
                .store(value as u64, core::sync::atomic::Ordering::Relaxed);
        }
    } else {
        // SAFETY: a writable `unsigned long` member addresses an aligned `AtomicU32`.
        // `c_ulong` is 4 bytes on this target.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU32>())
                .store(value as u32, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn load_u32(obj: &PyObject, offset: isize, atomic: bool) -> u32 {
    let addr = member_addr(obj, offset);
    if atomic {
        // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned `AtomicU32`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU32>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    } else {
        // SAFETY: a plain uint member addresses a `u32` that is not written
        // after publication.
        unsafe { addr.cast::<u32>().read() }
    }
}

fn member_as_c_long(value: &PyObject, vm: &VirtualMachine) -> PyResult<core::ffi::c_long> {
    let int_obj = value.try_index(vm)?;
    core::ffi::c_long::try_from(int_obj.as_bigint())
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))
}

fn member_uint_value(
    value: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(u32, Option<&'static str>)> {
    let int_obj = value.try_index(vm)?;
    let big = int_obj.as_bigint();
    if big.sign() == malachite_bigint::Sign::Minus {
        let long_val = core::ffi::c_long::try_from(big)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
        // Keeps the low 32 bits whether `c_long` is 32 or 64 bits wide.
        let stored = long_val as u32;
        return Ok((stored, Some("Writing negative value into unsigned field")));
    }
    core::ffi::c_ulong::try_from(big)
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C unsigned long"))?;
    let wide = u64::try_from(big).expect("a value that fits c_ulong fits u64");
    let stored = wide as u32;
    let warning = (wide > u64::from(u32::MAX)).then_some("Truncation of value to unsigned int");
    Ok((stored, warning))
}

// PyMember_GetOne. `offset` is a byte offset from the object to the field.
fn member_get_one(
    obj: &PyObject,
    offset: isize,
    member: &PyMemberDef,
    vm: &VirtualMachine,
) -> PyResult {
    let value = match member.kind {
        MemberKind::Object => obj.get_slot(offset).unwrap_or_else(|| vm.ctx.none()),
        MemberKind::ObjectEx => match obj.get_slot(offset) {
            Some(value) => value,
            None => {
                return Err(vm.new_attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    obj.class().fully_qualified_name(vm)?,
                    member.name
                )));
            }
        },
        MemberKind::Bool => {
            let raw = if member.atomic_storage() {
                // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned
                // `AtomicBool`.
                unsafe {
                    (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicBool>())
                        .load(core::sync::atomic::Ordering::Relaxed)
                }
            } else {
                // SAFETY: a plain bool member is one byte and is not written
                // after publication.
                unsafe { member_addr(obj, offset).cast::<bool>().read() }
            };
            vm.ctx.new_bool(raw).into()
        }
        MemberKind::Int => vm
            .ctx
            .new_int(load_i32(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::Uint => vm
            .ctx
            .new_int(load_u32(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::Double => {
            let raw = if member.atomic_storage() {
                // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned
                // `AtomicF64`.
                unsafe {
                    (*member_addr(obj, offset).cast::<crate::common::atomic::AtomicF64>())
                        .load(core::sync::atomic::Ordering::Relaxed)
                }
            } else {
                // SAFETY: a plain double member addresses an `f64` that is not
                // written after publication. A plain `f64` may be only 4-byte
                // aligned, so this is a plain read and not an atomic access.
                unsafe { member_addr(obj, offset).cast::<f64>().read() }
            };
            vm.ctx.new_float(raw).into()
        }
        MemberKind::PySsizeT => {
            let raw = if member.atomic_storage() {
                // SAFETY: `PyMemberFlags::ATOMIC` or a writable member addresses an aligned
                // `AtomicIsize`.
                unsafe {
                    (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicIsize>())
                        .load(core::sync::atomic::Ordering::Relaxed)
                }
            } else {
                // SAFETY: a plain `Py_ssize_t` member addresses an `isize` that
                // is not written after publication.
                unsafe { member_addr(obj, offset).cast::<isize>().read() }
            };
            vm.ctx.new_int(raw).into()
        }
        MemberKind::Short => vm
            .ctx
            .new_int(load_i16(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::UShort => vm
            .ctx
            .new_int(load_u16(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::UByte => vm
            .ctx
            .new_int(load_u8(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::Long => vm
            .ctx
            .new_int(load_c_long(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::LongLong => vm
            .ctx
            .new_int(load_i64(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::ULong => vm
            .ctx
            .new_int(load_c_ulong(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::ULongLong => vm
            .ctx
            .new_int(load_u64(obj, offset, member.atomic_storage()))
            .into(),
        MemberKind::String => {
            // SAFETY: a string member addresses a `*const c_char` that is not
            // written after publication. Null is `None`.
            let ptr = unsafe {
                member_addr(obj, offset)
                    .cast::<*const core::ffi::c_char>()
                    .read()
            };
            if ptr.is_null() {
                vm.ctx.none()
            } else {
                // SAFETY: `ptr` is non-null. The slot points at a `CString` the
                // owner keeps alive, so the bytes stay NUL-terminated for this read.
                let bytes = unsafe { core::ffi::CStr::from_ptr(ptr) }.to_bytes();
                let Ok(text) = core::str::from_utf8(bytes) else {
                    return Err(vm.new_unicode_decode_error(
                        vm.ctx.new_str("utf-8"),
                        vm.ctx.new_bytes(bytes.to_vec()),
                        0,
                        bytes.len(),
                        vm.ctx.new_str("invalid UTF-8"),
                    ));
                };
                vm.ctx.new_str(text).into()
            }
        }
    };
    Ok(value)
}

// PyMember_SetOne.
fn member_set_one(
    obj: &PyObject,
    offset: isize,
    member: &PyMemberDef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if matches!(value, PySetterValue::Delete)
        && !matches!(member.kind, MemberKind::Object | MemberKind::ObjectEx)
    {
        return Err(vm.new_type_error("can't delete numeric/char attribute"));
    }
    match member.kind {
        MemberKind::Object => match value {
            PySetterValue::Assign(v) => obj.set_slot(offset, Some(v)),
            PySetterValue::Delete => obj.set_slot(offset, None),
        },
        MemberKind::ObjectEx => match value {
            PySetterValue::Assign(v) => obj.set_slot(offset, Some(v)),
            PySetterValue::Delete => {
                if obj.get_slot(offset).is_none() {
                    return Err(vm.new_attribute_error(member.name.clone()));
                }
                obj.set_slot(offset, None);
            }
        },
        MemberKind::Bool => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            if !value.class().is(vm.ctx.types.bool_type) {
                return Err(vm.new_type_error("attribute value type must be bool"));
            }
            let stored = value.is(&vm.ctx.true_value);
            // SAFETY: a writable bool member addresses an `AtomicBool`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicBool>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
        }
        MemberKind::Int => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let long_val = member_as_c_long(&value, vm)?;
            let stored = long_val as i32;
            // SAFETY: a writable int member addresses an aligned `AtomicI32`.
            // Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicI32>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            let truncated = long_val > i32::MAX as core::ffi::c_long
                || long_val < i32::MIN as core::ffi::c_long;
            if truncated {
                warn_member(vm, "Truncation of value to int")?;
            }
        }
        MemberKind::Uint => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let (stored, warning) = member_uint_value(&value, vm)?;
            // SAFETY: a writable uint member addresses an aligned `AtomicU32`.
            // Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU32>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if let Some(warning) = warning {
                warn_member(vm, warning)?;
            }
        }
        MemberKind::Double => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let number = value.try_float(vm)?.to_f64();
            // SAFETY: a writable double member addresses an aligned `AtomicF64`.
            // Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<crate::common::atomic::AtomicF64>())
                    .store(number, core::sync::atomic::Ordering::Relaxed);
            }
        }
        MemberKind::PySsizeT => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            // PyLong_AsSsize_t: an int (bool included). No `__index__`.
            if !value.fast_isinstance(vm.ctx.types.int_type) {
                return Err(vm.new_type_error("an integer is required"));
            }
            let Some(int_obj) = value.downcast_ref::<crate::builtins::PyInt>() else {
                return Err(vm.new_type_error("an integer is required"));
            };
            let stored = isize::try_from(int_obj.as_bigint()).map_err(|_| {
                vm.new_overflow_error("Python int too large to convert to C ssize_t")
            })?;
            // SAFETY: a writable `Py_ssize_t` member addresses an aligned
            // `AtomicIsize`. Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicIsize>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
        }
        MemberKind::Short => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let long_val = member_as_c_long(&value, vm)?;
            let stored = long_val as i16;
            // SAFETY: a writable short member addresses an aligned `AtomicI16`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicI16>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if long_val > i16::MAX as core::ffi::c_long || long_val < i16::MIN as core::ffi::c_long
            {
                warn_member(vm, "Truncation of value to short")?;
            }
        }
        MemberKind::UShort => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let long_val = member_as_c_long(&value, vm)?;
            let stored = long_val as u16;
            // SAFETY: a writable unsigned short member addresses an aligned `AtomicU16`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU16>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if long_val > u16::MAX as core::ffi::c_long || long_val < 0 {
                warn_member(vm, "Truncation of value to unsigned short")?;
            }
        }
        MemberKind::UByte => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let long_val = member_as_c_long(&value, vm)?;
            let stored = long_val as u8;
            // SAFETY: a writable unsigned char member addresses an `AtomicU8`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU8>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if long_val > u8::MAX as core::ffi::c_long || long_val < 0 {
                warn_member(vm, "Truncation of value to unsigned char")?;
            }
        }
        MemberKind::Long => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let stored = member_as_c_long(&value, vm)?;
            store_c_long(obj, offset, stored);
        }
        MemberKind::LongLong => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let stored = member_as_c_longlong(&value, vm)?;
            // SAFETY: a writable `long long` member addresses an aligned `AtomicI64`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicI64>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
        }
        MemberKind::ULong => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let (stored, warning) = member_ulong_value(&value, vm)?;
            store_c_ulong(obj, offset, stored);
            if let Some(warning) = warning {
                warn_member(vm, warning)?;
            }
        }
        MemberKind::ULongLong => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let (stored, warning) = member_ulonglong_value(&value, vm)?;
            // SAFETY: a writable `unsigned long long` member addresses an aligned `AtomicU64`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU64>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if let Some(warning) = warning {
                warn_member(vm, warning)?;
            }
        }
        MemberKind::String => {
            return Err(vm.new_type_error("readonly attribute"));
        }
    }
    Ok(())
}

fn member_as_c_longlong(value: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
    let int_obj = value.try_index(vm)?;
    i64::try_from(int_obj.as_bigint())
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long long"))
}

fn member_ulong_value(
    value: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(core::ffi::c_ulong, Option<&'static str>)> {
    let int_obj = value.try_index(vm)?;
    let big = int_obj.as_bigint();
    if big.sign() == malachite_bigint::Sign::Minus {
        let long_val = core::ffi::c_long::try_from(big)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
        return Ok((
            long_val as core::ffi::c_ulong,
            Some("Writing negative value into unsigned field"),
        ));
    }
    let stored = core::ffi::c_ulong::try_from(big)
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C unsigned long"))?;
    Ok((stored, None))
}

fn member_ulonglong_value(
    value: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(u64, Option<&'static str>)> {
    let int_obj = value.try_index(vm)?;
    let big = int_obj.as_bigint();
    if big.sign() == malachite_bigint::Sign::Minus {
        let long_val = core::ffi::c_long::try_from(big)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
        return Ok((
            long_val as u64,
            Some("Writing negative value into unsigned field"),
        ));
    }
    let stored = u64::try_from(big).map_err(|_| {
        vm.new_overflow_error("Python int too large to convert to C unsigned long long")
    })?;
    Ok((stored, None))
}

impl Representable for PyMemberDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<member '{}' of '{}' objects>",
            zelf.common.name,
            zelf.common.typ.slot_name(),
        ))
    }
}

impl GetDescriptor for PyMemberDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm)?;
        match obj {
            Some(x) => {
                if !x.class().fast_issubclass(&descr.common.typ) {
                    return Err(vm.new_type_error(format!(
                        "descriptor '{}' for '{}' objects doesn't apply to a '{}' object",
                        descr.common.name,
                        descr.common.typ.name(),
                        x.class().name()
                    )));
                }
                descr.get(x.to_owned(), vm)
            }
            None => Ok(zelf.to_owned()),
        }
    }
}

fn method_descr_typecheck(
    descr: &PyMethodDescriptor,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if descr.method.flags.contains(PyMethodFlags::STATIC)
        || descr.method.flags.contains(PyMethodFlags::CLASS)
        || obj.fast_isinstance(descr.common.typ)
    {
        return Ok(());
    }
    Err(vm.new_type_error(format!(
        "descriptor '{}' for '{}' objects doesn't apply to a '{}' object",
        descr.common.name.as_str(),
        descr.common.typ.name(),
        obj.class().name()
    )))
}

#[inline]
fn method_descr_check_self(
    descr: &PyMethodDescriptor,
    obj: Option<&PyObject>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let obj = obj.ok_or_else(|| method_descr_missing_self_error(descr, vm))?;
    method_descr_typecheck(descr, obj, vm)
}

#[cold]
fn method_descr_missing_self_error(
    descr: &PyMethodDescriptor,
    vm: &VirtualMachine,
) -> crate::exceptions::types::PyBaseExceptionRef {
    vm.new_type_error(format!(
        "unbound method {}.{}() needs an argument",
        descr.common.typ.name(),
        descr.common.name
    ))
}

#[cold]
fn method_descr_no_keywords_error(
    descr: &PyMethodDescriptor,
    vm: &VirtualMachine,
) -> crate::exceptions::types::PyBaseExceptionRef {
    vm.new_type_error(format!(
        "{}.{}() takes no keyword arguments",
        descr.common.typ.name(),
        descr.common.name
    ))
}

/// Vectorcall for method_descriptor: calls native method directly
fn vectorcall_method_descriptor(
    zelf_obj: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyMethodDescriptor> = zelf_obj.downcast_ref().unwrap();
    method_descr_check_self(
        zelf,
        if nargs > 0 {
            args.first().map(|obj| &**obj)
        } else {
            None
        },
        vm,
    )?;
    if kwnames.is_some_and(|names| !names.is_empty())
        && !zelf.method.flags.contains(PyMethodFlags::KEYWORDS)
    {
        return Err(method_descr_no_keywords_error(zelf, vm));
    }
    let func_args = FuncArgs::from_vectorcall_owned(args, nargs, kwnames);
    (zelf.method.func)(
        vm,
        func_args,
        Callee::named(zelf.method.name).with_instance_arg(true),
    )
}

/// Vectorcall for wrapper_descriptor: calls wrapped slot function
fn vectorcall_wrapper(
    zelf_obj: &PyObject,
    mut args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyWrapper> = zelf_obj.downcast_ref().unwrap();
    // First positional arg is self
    if nargs == 0 {
        return Err(vm.new_type_error(format!(
            "descriptor '{}' of '{}' object needs an argument",
            zelf.name.as_str(),
            zelf.typ.name()
        )));
    }
    let obj = args.remove(0);
    if !obj.fast_isinstance(zelf.typ) {
        return Err(vm.new_type_error(format!(
            "descriptor '{}' requires a '{}' object but received a '{}'",
            zelf.name.as_str(),
            zelf.typ.name(),
            obj.class().name()
        )));
    }
    let rest = FuncArgs::from_vectorcall_owned(args, nargs - 1, kwnames);
    zelf.wrapped.call(obj, rest, vm)
}

pub(crate) fn init(ctx: &'static Context) {
    PyMemberDescriptor::extend_class(ctx, ctx.types.member_descriptor_type);
    PyMethodDescriptor::extend_class(ctx, ctx.types.method_descriptor_type);
    ctx.types
        .method_descriptor_type
        .slots
        .vectorcall
        .store(Some(vectorcall_method_descriptor));
    PyClassMethodDescriptor::extend_class(ctx, ctx.types.classmethod_descriptor_type);
    PyWrapper::extend_class(ctx, ctx.types.wrapper_descriptor_type);
    ctx.types
        .wrapper_descriptor_type
        .slots
        .vectorcall
        .store(Some(vectorcall_wrapper));
    PyMethodWrapper::extend_class(ctx, ctx.types.method_wrapper_type);
}

// PyWrapper - wrapper_descriptor

/// Each variant knows how to call the wrapped function with proper types
#[derive(Clone, Copy)]
pub enum SlotFunc {
    // Basic slots
    Init(InitFunc),
    Hash(HashFunc),
    Str(StringifyFunc),
    Repr(StringifyFunc),
    Iter(IterFunc),
    IterNext(IterNextFunc),
    Call(GenericMethod),
    Del(DelFunc),

    // Attribute access slots
    GetAttro(GetattroFunc),
    SetAttro(SetattroFunc), // __setattr__
    DelAttro(SetattroFunc), // __delattr__ (same func type, different PySetterValue)

    // Rich comparison slots (with comparison op)
    RichCompare(RichCompareFunc, PyComparisonOp),

    // Descriptor slots
    DescrGet(DescrGetFunc),
    DescrSet(DescrSetFunc), // __set__
    DescrDel(DescrSetFunc), // __delete__ (same func type, different PySetterValue)

    // Sequence sub-slots (sq_*)
    SeqLength(SeqLenFunc),
    SeqConcat(SeqConcatFunc),
    SeqRepeat(SeqRepeatFunc),
    SeqItem(SeqItemFunc),
    SeqSetItem(SeqAssItemFunc), // __setitem__ (same func type, value = Some)
    SeqDelItem(SeqAssItemFunc), // __delitem__ (same func type, value = None)
    SeqContains(SeqContainsFunc),

    // Mapping sub-slots (mp_*)
    MapLength(MapLenFunc),
    MapSubscript(MapSubscriptFunc),
    MapSetSubscript(MapAssSubscriptFunc), // __setitem__ (same func type, value = Some)
    MapDelSubscript(MapAssSubscriptFunc), // __delitem__ (same func type, value = None)

    // Number sub-slots (nb_*) - grouped by signature
    NumBoolean(PyNumberUnaryFunc<bool>),  // __bool__
    NumUnary(PyNumberUnaryFunc),          // __int__, __float__, __index__
    NumBinary(PyNumberBinaryFunc),        // __add__, __sub__, __mul__, etc.
    NumBinaryRight(PyNumberBinaryFunc),   // __radd__, __rsub__, etc. (swapped args)
    NumTernary(PyNumberTernaryFunc),      // __pow__
    NumTernaryRight(PyNumberTernaryFunc), // __rpow__ (swapped first two args)

    // Buffer protocol
    GetBuffer(crate::types::AsBufferFunc), // __buffer__
    ReleaseBuffer,                         // __release_buffer__
}

impl core::fmt::Debug for SlotFunc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Init(_) => write!(f, "SlotFunc::Init(...)"),
            Self::Hash(_) => write!(f, "SlotFunc::Hash(...)"),
            Self::Str(_) => write!(f, "SlotFunc::Str(...)"),
            Self::Repr(_) => write!(f, "SlotFunc::Repr(...)"),
            Self::Iter(_) => write!(f, "SlotFunc::Iter(...)"),
            Self::IterNext(_) => write!(f, "SlotFunc::IterNext(...)"),
            Self::Call(_) => write!(f, "SlotFunc::Call(...)"),
            Self::Del(_) => write!(f, "SlotFunc::Del(...)"),
            Self::GetAttro(_) => write!(f, "SlotFunc::GetAttro(...)"),
            Self::SetAttro(_) => write!(f, "SlotFunc::SetAttro(...)"),
            Self::DelAttro(_) => write!(f, "SlotFunc::DelAttro(...)"),
            Self::RichCompare(_, op) => write!(f, "SlotFunc::RichCompare(..., {op:?})"),
            Self::DescrGet(_) => write!(f, "SlotFunc::DescrGet(...)"),
            Self::DescrSet(_) => write!(f, "SlotFunc::DescrSet(...)"),
            Self::DescrDel(_) => write!(f, "SlotFunc::DescrDel(...)"),
            // Sequence sub-slots
            Self::SeqLength(_) => write!(f, "SlotFunc::SeqLength(...)"),
            Self::SeqConcat(_) => write!(f, "SlotFunc::SeqConcat(...)"),
            Self::SeqRepeat(_) => write!(f, "SlotFunc::SeqRepeat(...)"),
            Self::SeqItem(_) => write!(f, "SlotFunc::SeqItem(...)"),
            Self::SeqSetItem(_) => write!(f, "SlotFunc::SeqSetItem(...)"),
            Self::SeqDelItem(_) => write!(f, "SlotFunc::SeqDelItem(...)"),
            Self::SeqContains(_) => write!(f, "SlotFunc::SeqContains(...)"),
            // Mapping sub-slots
            Self::MapLength(_) => write!(f, "SlotFunc::MapLength(...)"),
            Self::MapSubscript(_) => write!(f, "SlotFunc::MapSubscript(...)"),
            Self::MapSetSubscript(_) => write!(f, "SlotFunc::MapSetSubscript(...)"),
            Self::MapDelSubscript(_) => write!(f, "SlotFunc::MapDelSubscript(...)"),
            // Number sub-slots
            Self::NumBoolean(_) => write!(f, "SlotFunc::NumBoolean(...)"),
            Self::NumUnary(_) => write!(f, "SlotFunc::NumUnary(...)"),
            Self::NumBinary(_) => write!(f, "SlotFunc::NumBinary(...)"),
            Self::NumBinaryRight(_) => write!(f, "SlotFunc::NumBinaryRight(...)"),
            Self::NumTernary(_) => write!(f, "SlotFunc::NumTernary(...)"),
            Self::NumTernaryRight(_) => write!(f, "SlotFunc::NumTernaryRight(...)"),
            Self::GetBuffer(_) => write!(f, "SlotFunc::GetBuffer(...)"),
            Self::ReleaseBuffer => write!(f, "SlotFunc::ReleaseBuffer"),
        }
    }
}

impl SlotFunc {
    /// Call the wrapped slot function with proper type handling
    pub fn call(&self, obj: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Init(func) => {
                func(&obj, args, vm)?;
                Ok(vm.ctx.none())
            }
            Self::Hash(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__hash__() takes no arguments (1 given)"));
                }
                let hash = func(&obj, vm)?;
                Ok(vm.ctx.new_int(hash).into())
            }
            Self::Repr(func) | Self::Str(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    let name = match self {
                        Self::Repr(_) => "__repr__",
                        Self::Str(_) => "__str__",
                        _ => unreachable!(),
                    };
                    return Err(vm.new_type_error(format!("{name}() takes no arguments (1 given)")));
                }
                let s = func(&obj, vm)?;
                Ok(s.into())
            }
            Self::Iter(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__iter__() takes no arguments (1 given)"));
                }
                func(obj, vm)
            }
            Self::IterNext(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__next__() takes no arguments (1 given)"));
                }
                func(&obj, vm).to_pyresult(vm)
            }
            Self::Call(func) => func(&obj, args, vm),
            Self::Del(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__del__() takes no arguments (1 given)"));
                }
                func(&obj, vm)?;
                Ok(vm.ctx.none())
            }
            Self::GetAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                func(&obj, &name, vm)
            }
            Self::SetAttro(func) => {
                let (name, value): (PyRef<PyStr>, PyObjectRef) = args.bind(vm)?;
                crate::types::hackcheck_setattro(&obj, *func, "__setattr__", vm)?;
                func(&obj, &name, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::DelAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                crate::types::hackcheck_setattro(&obj, *func, "__delattr__", vm)?;
                func(&obj, &name, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
            Self::RichCompare(func, op) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, &other, *op, vm).map(|r| match r {
                    crate::function::Either::A(obj) => obj,
                    crate::function::Either::B(cmp_val) => cmp_val.to_pyobject(vm),
                })
            }
            Self::DescrGet(func) => {
                let (instance, owner): (PyObjectRef, crate::function::OptionalArg<PyObjectRef>) =
                    args.bind(vm)?;
                let owner = owner.into_option();
                let instance_opt = if vm.is_none(&instance) {
                    None
                } else {
                    Some(instance)
                };
                func(&obj, instance_opt.as_deref(), owner.as_deref(), vm)
            }
            Self::DescrSet(func) => {
                let (instance, value): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::DescrDel(func) => {
                let (instance,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
            // Sequence sub-slots
            Self::SeqLength(func) => {
                args.bind::<()>(vm)?;
                let len = func(obj.sequence_unchecked(), vm)?;
                Ok(vm.ctx.new_int(len).into())
            }
            Self::SeqConcat(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), &other, vm)
            }
            Self::SeqRepeat(func) => {
                let (n,): (PySsize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), n, vm)
            }
            Self::SeqItem(func) => {
                let (index,): (isize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, vm)
            }
            Self::SeqSetItem(func) => {
                let (index, value): (isize, PyObjectRef) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, Some(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::SeqDelItem(func) => {
                let (index,): (isize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, None, vm)?;
                Ok(vm.ctx.none())
            }
            Self::SeqContains(func) => {
                let (item,): (PyObjectRef,) = args.bind(vm)?;
                let result = func(obj.sequence_unchecked(), &item, vm)?;
                Ok(vm.ctx.new_bool(result).into())
            }
            // Mapping sub-slots
            Self::MapLength(func) => {
                args.bind::<()>(vm)?;
                let len = func(obj.mapping_unchecked(), vm)?;
                Ok(vm.ctx.new_int(len).into())
            }
            Self::MapSubscript(func) => {
                let (key,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, vm)
            }
            Self::MapSetSubscript(func) => {
                let (key, value): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, Some(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::MapDelSubscript(func) => {
                let (key,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, None, vm)?;
                Ok(vm.ctx.none())
            }
            // Number sub-slots
            Self::NumBoolean(func) => {
                args.bind::<()>(vm)?;
                let result = func(obj.number(), vm)?;
                Ok(vm.ctx.new_bool(result).into())
            }
            Self::NumUnary(func) => {
                args.bind::<()>(vm)?;
                func(obj.number(), vm)
            }
            Self::NumBinary(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, &other, vm)
            }
            Self::NumBinaryRight(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&other, &obj, vm) // Swapped: other op obj
            }
            Self::NumTernary(func) => {
                let (y, z) = pow_args(args, vm)?;
                func(&obj, &y, &z, vm)
            }
            Self::NumTernaryRight(func) => {
                let (y, z) = pow_args(args, vm)?;
                func(&y, &obj, &z, vm)
            }
            // Buffer protocol
            Self::GetBuffer(func) => {
                let (flags_obj,): (PyObjectRef,) = args.bind(vm)?;
                let buffer = func(&obj, parse_buffer_flags(&flags_obj, vm)?, vm)?;
                crate::builtins::PyMemoryView::from_buffer(buffer, vm)
                    .map(|mv| mv.into_pyobject(vm))
            }
            Self::ReleaseBuffer => {
                let (mv_obj,): (PyObjectRef,) = args.bind(vm)?;
                let mv = mv_obj
                    .downcast::<crate::builtins::PyMemoryView>()
                    .map_err(|_| vm.new_type_error("expected a memoryview object"))?;
                crate::builtins::memory::release_buffer_from_python(&obj, &mv, vm)?;
                Ok(vm.ctx.none())
            }
        }
    }
}

/// wrap_ternaryfunc / check_pow_args
fn pow_args(args: FuncArgs, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
    if let Some(err) = args.check_kwargs_empty(vm) {
        return Err(err);
    }
    let size = args.args.len();
    if !(1..=2).contains(&size) {
        return Err(vm.new_type_error(format!("expected 1 or 2 arguments, got {size}")));
    }
    let y = args.args[0].clone();
    let z = if size == 2 {
        args.args[1].clone()
    } else {
        vm.ctx.none()
    };
    Ok((y, z))
}

/// Parse the `flags` argument of `__buffer__`. wrap_buffer
fn parse_buffer_flags(
    arg: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<crate::protocol::BufferFlags> {
    use num_traits::ToPrimitive;
    let idx = arg.try_index(vm)?;
    let flags = idx
        .as_bigint()
        .to_isize()
        .ok_or_else(|| vm.new_overflow_error("cannot fit 'int' into an index-sized integer"))?;
    let flags =
        i32::try_from(flags).map_err(|_| vm.new_overflow_error("buffer flags out of range"))?;
    Ok(crate::protocol::BufferFlags::from_bits_retain(flags as u32))
}

// wrapper_descriptor: wraps a slot function as a Python method
// = PyWrapperDescrObject
#[pyclass(name = "wrapper_descriptor", module = false)]
#[derive(Debug)]
pub(crate) struct PyWrapper {
    #[pymember(name = "__objclass__")]
    pub typ: &'static Py<PyType>,
    #[pymember(name = "__name__")]
    pub name: &'static PyStrInterned,
    pub wrapped: SlotFunc,
    /// Slot text, including the text signature.
    pub doc: Option<&'static str>,
    /// Plain docstring for this slot when the table has one.
    pub plain_off: u32,
    pub plain_len: u32,
}

impl PyPayload for PyWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.wrapper_descriptor_type
    }
}

impl GetDescriptor for PyWrapper {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match obj {
            None => Ok(zelf.to_owned()),
            Some(obj) => {
                let zelf = zelf.to_owned().downcast::<Self>().unwrap();
                Ok(PyMethodWrapper {
                    wrapper: zelf,
                    obj: obj.to_owned(),
                }
                .into_pyobject(vm))
            }
        }
    }
}

impl Callable for PyWrapper {
    type Args = FuncArgs;

    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // list.__init__(l, [1,2,3]) form - first arg is self
        let (obj, rest): (PyObjectRef, FuncArgs) = args.bind(vm)?;

        if !obj.fast_isinstance(zelf.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a '{}' object but received a '{}'",
                zelf.name.as_str(),
                zelf.typ.name(),
                obj.class().name()
            )));
        }

        zelf.wrapped.call(obj, rest, vm)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(DISALLOW_INSTANTIATION)
)]
impl Py<PyWrapper> {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
        let getattr = vm.builtins.get_attr("getattr", vm)?;
        let args = vm.ctx.new_tuple(vec![
            self.typ.to_owned().into(),
            self.name.to_owned().into(),
        ]);
        Ok(vm.ctx.new_tuple(vec![getattr, args.into()]).into())
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.typ.name(), self.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        if self.plain_len != 0 {
            return crate::function::db_doc(self.plain_off, self.plain_len);
        }
        let doc = self.doc?;
        type_::get_doc_from_internal_doc(self.name.as_str(), doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.doc.and_then(|doc| {
            type_::get_text_signature_from_internal_doc(self.name.as_str(), doc)
                .map(|signature| signature.to_string())
        })
    }
}

impl Representable for PyWrapper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<slot wrapper '{}' of '{}' objects>",
            zelf.name.as_str(),
            zelf.typ.name()
        ))
    }
}

// PyMethodWrapper - method-wrapper

// method-wrapper: a slot wrapper bound to an instance
// Returned when accessing l.__init__ on an instance
#[pyclass(name = "method-wrapper", module = false, traverse)]
#[derive(Debug)]
pub(crate) struct PyMethodWrapper {
    pub wrapper: PyRef<PyWrapper>,
    #[pymember(name = "__self__")]
    #[pytraverse(skip)]
    pub obj: PyObjectRef,
}

impl PyPayload for PyMethodWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.method_wrapper_type
    }
}

impl Callable for PyMethodWrapper {
    type Args = FuncArgs;

    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // bpo-37619: Check type compatibility before calling wrapped slot
        if !zelf.obj.fast_isinstance(zelf.wrapper.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a '{}' object but received a '{}'",
                zelf.wrapper.name.as_str(),
                zelf.wrapper.typ.name(),
                zelf.obj.class().name()
            )));
        }
        zelf.wrapper.wrapped.call(zelf.obj.clone(), args, vm)
    }
}

#[pyclass(
    with(Callable, Representable, Hashable, Comparable),
    flags(DISALLOW_INSTANTIATION)
)]
impl Py<PyMethodWrapper> {
    #[pygetset]
    fn __name__(&self) -> &'static PyStrInterned {
        self.wrapper.name
    }

    #[pygetset]
    fn __objclass__(&self) -> PyTypeRef {
        self.wrapper.typ.to_owned()
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.wrapper.typ.name(), self.wrapper.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        if self.wrapper.plain_len != 0 {
            return crate::function::db_doc(self.wrapper.plain_off, self.wrapper.plain_len);
        }
        let doc = self.wrapper.doc?;
        type_::get_doc_from_internal_doc(self.wrapper.name.as_str(), doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.wrapper.doc.and_then(|doc| {
            type_::get_text_signature_from_internal_doc(self.wrapper.name.as_str(), doc)
                .map(|signature| signature.to_string())
        })
    }

    #[pymethod]
    fn __reduce__(zelf: PyRef<PyMethodWrapper>, vm: &VirtualMachine) -> PyResult {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm)?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                builtins_getattr,
                vm.ctx
                    .new_tuple(vec![
                        zelf.obj.clone(),
                        vm.ctx.new_str(zelf.wrapper.name.as_str()).into(),
                    ])
                    .into(),
            ])
            .into())
    }
}

impl Representable for PyMethodWrapper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method-wrapper '{}' of {} object at {:#x}>",
            zelf.wrapper.name.as_str(),
            zelf.obj.class().name(),
            zelf.obj.get_id()
        ))
    }
}

impl Hashable for PyMethodWrapper {
    fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
        // wrapperobject_hash: pointer hash of descr xor self
        let mut hash =
            (zelf.wrapper.as_object().get_id() as PyHash) ^ (zelf.obj.get_id() as PyHash);
        if hash == -1 {
            hash = -2;
        }
        Ok(hash)
    }
}

impl Comparable for PyMethodWrapper {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<crate::function::PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let eq = zelf.wrapper.is(&other.wrapper) && zelf.obj.is(&other.obj);
            Ok(eq.into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{protocol::PyNumberMethods, types::AsNumber};
    use core::sync::atomic::{AtomicU8, Ordering};

    #[pyclass(name = "UByteMembers", module = false)]
    #[derive(Debug, PyPayload)]
    #[repr(C)]
    struct UByteMembers {
        prefix: u8,
        #[pymember]
        readonly: u8,
        #[pymember(writable)]
        writable: AtomicU8,
        suffix: u8,
    }

    #[pyclass(with(AsNumber))]
    impl UByteMembers {}

    impl AsNumber for UByteMembers {
        fn as_number() -> &'static PyNumberMethods {
            static METHODS: PyNumberMethods = PyNumberMethods {
                index: Some(|_, vm| Ok(vm.ctx.new_int(42).into())),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &METHODS
        }
    }

    #[pyclass(name = "ReadonlyAtomicMembers", module = false)]
    #[derive(Debug, PyPayload)]
    struct ReadonlyAtomicMembers {
        #[pymember]
        byte: AtomicU8,
        #[pymember]
        object: crate::object::PyObjectCell,
    }

    #[pyclass]
    impl ReadonlyAtomicMembers {}

    #[test]
    fn readonly_atomic_members_use_atomic_storage() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let _class = ReadonlyAtomicMembers::make_static_type();
            let obj = ReadonlyAtomicMembers {
                byte: AtomicU8::new(128),
                object: None.into(),
            }
            .into_ref(&vm.ctx);

            for name in ["byte", "object"] {
                let descriptor = obj.class().as_object().get_attr(name, vm).unwrap();
                let descriptor = descriptor.downcast_ref::<PyMemberDescriptor>().unwrap();
                assert!(descriptor.member.readonly());
                assert!(descriptor.member.atomic_storage());
                let err = obj
                    .as_object()
                    .set_attr(name, vm.ctx.none(), vm)
                    .unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.attribute_error));
            }

            assert!(vm.is_none(&obj.as_object().get_attr("object", vm).unwrap()));
            obj.byte.store(255, Ordering::Relaxed);
            let value: PyObjectRef = vm.ctx.new_int(42).into();
            obj.object.store(Some(value.clone()));
            assert!(obj.as_object().get_attr("object", vm).unwrap().is(&value));
            let byte = obj.as_object().get_attr("byte", vm).unwrap();
            assert_eq!(
                u8::try_from(
                    byte.downcast_ref::<crate::builtins::PyInt>()
                        .unwrap()
                        .as_bigint()
                )
                .unwrap(),
                255
            );
            obj.object.store(None);
            assert!(vm.is_none(&obj.as_object().get_attr("object", vm).unwrap()));
        });
    }

    #[test]
    fn unsigned_byte_members_preserve_adjacent_fields() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let _class = UByteMembers::make_static_type();
            let obj = UByteMembers {
                prefix: 17,
                readonly: 255,
                writable: AtomicU8::new(128),
                suffix: 29,
            }
            .into_ref(&vm.ctx);

            assert_eq!(MemberKind::from_i32(9), Some(MemberKind::UByte));
            for (name, expected) in [("readonly", 255), ("writable", 128)] {
                let value = obj.as_object().get_attr(name, vm).unwrap();
                let value = value.downcast_ref::<crate::builtins::PyInt>().unwrap();
                assert_eq!(i32::try_from(value.as_bigint()).unwrap(), expected);
            }
            obj.as_object()
                .set_attr("writable", vm.ctx.new_int(255), vm)
                .unwrap();
            assert_eq!(obj.writable.load(Ordering::Relaxed), 255);
            assert_eq!((obj.prefix, obj.readonly, obj.suffix), (17, 255, 29));

            let err = obj
                .as_object()
                .set_attr("readonly", vm.ctx.new_int(0), vm)
                .unwrap_err();
            assert!(err.fast_isinstance(vm.ctx.exceptions.attribute_error));
            let err = obj.as_object().del_attr("writable", vm).unwrap_err();
            assert!(err.fast_isinstance(vm.ctx.exceptions.type_error));
            assert_eq!(obj.writable.load(Ordering::Relaxed), 255);

            let err = obj
                .as_object()
                .set_attr("writable", vm.ctx.new_int(i128::MAX), vm)
                .unwrap_err();
            assert!(err.fast_isinstance(vm.ctx.exceptions.overflow_error));
            assert_eq!(obj.writable.load(Ordering::Relaxed), 255);

            obj.as_object()
                .set_attr("writable", obj.to_owned(), vm)
                .unwrap();
            assert_eq!(obj.writable.load(Ordering::Relaxed), 42);

            let filters = vm.state.warnings.filters.to_owned();
            filters.borrow_vec_mut().insert(
                0,
                vm.ctx
                    .new_tuple(vec![
                        vm.ctx.new_str("error").into(),
                        vm.ctx.none(),
                        vm.ctx.exceptions.runtime_warning.to_owned().into(),
                        vm.ctx.none(),
                        vm.ctx.new_int(0).into(),
                    ])
                    .into(),
            );
            vm.state.warnings.filters_mutated();
            for (input, expected) in [(256, 0), (-1, 255)] {
                let err = obj
                    .as_object()
                    .set_attr("writable", vm.ctx.new_int(input), vm)
                    .unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.runtime_warning));
                // As in CPython, truncation is stored before the warning is raised.
                assert_eq!(obj.writable.load(Ordering::Relaxed), expected);
                assert_eq!((obj.prefix, obj.readonly, obj.suffix), (17, 255, 29));
            }
        });
    }
}

use super::{
    PositionIterInternal, PyBytesRef, PyDict, PyTupleRef, PyType, PyTypeRef,
    int::{PyInt, PyIntRef},
    iter::IterStatus::{self, Exhausted},
};
use crate::{
    AsObject, Context, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromBorrowedObject, VirtualMachine,
    anystr::{self, AnyStr, AnyStrContainer, AnyStrWrapper, adjust_indices},
    atomic_func,
    cformat::cformat_string,
    class::PyClassImpl,
    common::str::{PyKindStr, StrData, StrKind},
    convert::{IntoPyException, ToPyException, ToPyObject, ToPyResult},
    format::{format, format_map},
    function::{ArgIterable, ArgSize, FuncArgs, OptionalArg, OptionalOption, PyComparisonValue},
    intern::PyInterned,
    object::{Traverse, TraverseFn},
    protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    sequence::SequenceExt,
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, Hashable, IterNext, Iterable,
        PyComparisonOp, Representable, SelfIter, Unconstructible,
    },
};
use ascii::{AsciiChar, AsciiStr, AsciiString};
use bstr::ByteSlice;
use itertools::Itertools;
use num_traits::ToPrimitive;
use rustpython_common::{
    ascii,
    atomic::{self, PyAtomic, Radium},
    format::{FormatSpec, FormatString, FromTemplate},
    hash,
    lock::PyMutex,
    str::DeduceStrKind,
    wtf8::{CodePoint, Wtf8, Wtf8Buf, Wtf8Chunk},
};
use std::{borrow::Cow, char, fmt, ops::Range};
use std::{mem, sync::LazyLock};
use unic_ucd_bidi::BidiClass;
use unic_ucd_category::GeneralCategory;
use unic_ucd_ident::{is_xid_continue, is_xid_start};
use unicode_casing::CharExt;

impl<'a> TryFromBorrowedObject<'a> for String {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(|pystr: &PyStr| Ok(pystr.as_str().to_owned()), vm)
    }
}

impl<'a> TryFromBorrowedObject<'a> for &'a str {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        let pystr: &Py<PyStr> = TryFromBorrowedObject::try_from_borrowed_object(vm, obj)?;
        Ok(pystr.as_str())
    }
}

impl<'a> TryFromBorrowedObject<'a> for &'a Wtf8 {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        let pystr: &Py<PyStr> = TryFromBorrowedObject::try_from_borrowed_object(vm, obj)?;
        Ok(pystr.as_wtf8())
    }
}

#[pyclass(module = false, name = "str")]
pub struct PyStr {
    data: StrData,
    hash: PyAtomic<hash::PyHash>,
}

impl fmt::Debug for PyStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PyStr")
            .field("value", &self.as_wtf8())
            .field("kind", &self.data.kind())
            .field("hash", &self.hash)
            .finish()
    }
}

#[repr(transparent)]
#[derive(Debug)]
pub struct PyUtf8Str(PyStr);

// TODO: Remove this Deref which may hide missing optimized methods of PyUtf8Str
impl std::ops::Deref for PyUtf8Str {
    type Target = PyStr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PyUtf8Str {
    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        debug_assert!(
            self.0.is_utf8(),
            "PyUtf8Str invariant violated: inner string is not valid UTF-8"
        );
        // Safety: This is safe because the type invariant guarantees UTF-8 validity.
        unsafe { self.0.to_str().unwrap_unchecked() }
    }
}

impl AsRef<str> for PyStr {
    #[track_caller] // <- can remove this once it doesn't panic
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Py<PyStr> {
    #[track_caller] // <- can remove this once it doesn't panic
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for PyStrRef {
    #[track_caller] // <- can remove this once it doesn't panic
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<Wtf8> for PyStr {
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl AsRef<Wtf8> for Py<PyStr> {
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl AsRef<Wtf8> for PyStrRef {
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl<'a> From<&'a AsciiStr> for PyStr {
    fn from(s: &'a AsciiStr) -> Self {
        s.to_owned().into()
    }
}

impl From<AsciiString> for PyStr {
    fn from(s: AsciiString) -> Self {
        s.into_boxed_ascii_str().into()
    }
}

impl From<Box<AsciiStr>> for PyStr {
    fn from(s: Box<AsciiStr>) -> Self {
        StrData::from(s).into()
    }
}

impl From<AsciiChar> for PyStr {
    fn from(ch: AsciiChar) -> Self {
        AsciiString::from(ch).into()
    }
}

impl<'a> From<&'a str> for PyStr {
    fn from(s: &'a str) -> Self {
        s.to_owned().into()
    }
}

impl<'a> From<&'a Wtf8> for PyStr {
    fn from(s: &'a Wtf8) -> Self {
        s.to_owned().into()
    }
}

impl From<String> for PyStr {
    fn from(s: String) -> Self {
        s.into_boxed_str().into()
    }
}

impl From<Wtf8Buf> for PyStr {
    fn from(w: Wtf8Buf) -> Self {
        w.into_box().into()
    }
}

impl From<char> for PyStr {
    fn from(ch: char) -> Self {
        StrData::from(ch).into()
    }
}

impl From<CodePoint> for PyStr {
    fn from(ch: CodePoint) -> Self {
        StrData::from(ch).into()
    }
}

impl From<StrData> for PyStr {
    fn from(data: StrData) -> Self {
        Self {
            data,
            hash: Radium::new(hash::SENTINEL),
        }
    }
}

impl<'a> From<std::borrow::Cow<'a, str>> for PyStr {
    fn from(s: std::borrow::Cow<'a, str>) -> Self {
        s.into_owned().into()
    }
}

impl From<Box<str>> for PyStr {
    #[inline]
    fn from(value: Box<str>) -> Self {
        StrData::from(value).into()
    }
}

impl From<Box<Wtf8>> for PyStr {
    #[inline]
    fn from(value: Box<Wtf8>) -> Self {
        StrData::from(value).into()
    }
}

impl Default for PyStr {
    fn default() -> Self {
        Self {
            data: StrData::default(),
            hash: Radium::new(hash::SENTINEL),
        }
    }
}

pub type PyStrRef = PyRef<PyStr>;

impl fmt::Display for PyStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_wtf8().fmt(f)
    }
}

pub trait AsPyStr<'a>
where
    Self: 'a,
{
    #[allow(clippy::wrong_self_convention)] // to implement on refs
    fn as_pystr(self, ctx: &Context) -> &'a Py<PyStr>;
}

impl<'a> AsPyStr<'a> for &'a Py<PyStr> {
    #[inline]
    fn as_pystr(self, _ctx: &Context) -> &'a Py<PyStr> {
        self
    }
}

impl<'a> AsPyStr<'a> for &'a PyStrRef {
    #[inline]
    fn as_pystr(self, _ctx: &Context) -> &'a Py<PyStr> {
        self
    }
}

impl AsPyStr<'static> for &'static str {
    #[inline]
    fn as_pystr(self, ctx: &Context) -> &'static Py<PyStr> {
        ctx.intern_str(self)
    }
}

impl<'a> AsPyStr<'a> for &'a PyStrInterned {
    #[inline]
    fn as_pystr(self, _ctx: &Context) -> &'a Py<PyStr> {
        self
    }
}

#[pyclass(module = false, name = "str_iterator", traverse = "manual")]
#[derive(Debug)]
pub struct PyStrIterator {
    internal: PyMutex<(PositionIterInternal<PyStrRef>, usize)>,
}

unsafe impl Traverse for PyStrIterator {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        // No need to worry about deadlock, for inner is a PyStr and can't make ref cycle
        self.internal.lock().0.traverse(tracer);
    }
}

impl PyPayload for PyStrIterator {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.str_iterator_type
    }
}

#[pyclass(with(Unconstructible, IterNext, Iterable))]
impl PyStrIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().0.length_hint(|obj| obj.char_len())
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut internal = self.internal.lock();
        internal.1 = usize::MAX;
        internal
            .0
            .set_state(state, |obj, pos| pos.min(obj.char_len()), vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .0
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }
}

impl Unconstructible for PyStrIterator {}

impl SelfIter for PyStrIterator {}

impl IterNext for PyStrIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut internal = zelf.internal.lock();

        if let IterStatus::Active(s) = &internal.0.status {
            let value = s.as_wtf8();

            if internal.1 == usize::MAX {
                if let Some((offset, ch)) = value.code_point_indices().nth(internal.0.position) {
                    internal.0.position += 1;
                    internal.1 = offset + ch.len_wtf8();
                    return Ok(PyIterReturn::Return(ch.to_pyobject(vm)));
                }
            } else if let Some(value) = value.get(internal.1..) {
                if let Some(ch) = value.code_points().next() {
                    internal.0.position += 1;
                    internal.1 += ch.len_wtf8();
                    return Ok(PyIterReturn::Return(ch.to_pyobject(vm)));
                }
            }
            internal.0.status = Exhausted;
        }
        Ok(PyIterReturn::StopIteration(None))
    }
}

#[derive(FromArgs)]
pub struct StrArgs {
    #[pyarg(any, optional)]
    object: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    encoding: OptionalArg<PyStrRef>,
    #[pyarg(any, optional)]
    errors: OptionalArg<PyStrRef>,
}

impl Constructor for PyStr {
    type Args = StrArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let string: PyStrRef = match args.object {
            OptionalArg::Present(input) => {
                if let OptionalArg::Present(enc) = args.encoding {
                    vm.state.codec_registry.decode_text(
                        input,
                        enc.as_str(),
                        args.errors.into_option(),
                        vm,
                    )?
                } else {
                    input.str(vm)?
                }
            }
            OptionalArg::Missing => {
                Self::from(String::new()).into_ref_with_type(vm, cls.clone())?
            }
        };
        if string.class().is(&cls) {
            Ok(string.into())
        } else {
            Self::from(string.as_wtf8())
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }
}

impl PyStr {
    /// # Safety: Given `bytes` must be valid data for given `kind`
    unsafe fn new_str_unchecked(data: Box<Wtf8>, kind: StrKind) -> Self {
        unsafe { StrData::new_str_unchecked(data, kind) }.into()
    }

    unsafe fn new_with_char_len<T: DeduceStrKind + Into<Box<Wtf8>>>(s: T, char_len: usize) -> Self {
        let kind = s.str_kind();
        unsafe { StrData::new_with_char_len(s.into(), kind, char_len) }.into()
    }

    /// # Safety
    /// Given `bytes` must be ascii
    pub unsafe fn new_ascii_unchecked(bytes: Vec<u8>) -> Self {
        unsafe { AsciiString::from_ascii_unchecked(bytes) }.into()
    }

    pub fn new_ref(zelf: impl Into<Self>, ctx: &Context) -> PyRef<Self> {
        let zelf = zelf.into();
        PyRef::new_ref(zelf, ctx.types.str_type.to_owned(), None)
    }

    fn new_substr(&self, s: Wtf8Buf) -> Self {
        let kind = if self.kind().is_ascii() || s.is_ascii() {
            StrKind::Ascii
        } else if self.kind().is_utf8() || s.is_utf8() {
            StrKind::Utf8
        } else {
            StrKind::Wtf8
        };
        unsafe {
            // SAFETY: kind is properly decided for substring
            Self::new_str_unchecked(s.into(), kind)
        }
    }

    #[inline]
    pub const fn as_wtf8(&self) -> &Wtf8 {
        self.data.as_wtf8()
    }

    pub const fn as_bytes(&self) -> &[u8] {
        self.data.as_wtf8().as_bytes()
    }

    // FIXME: make this return an Option
    #[inline]
    #[track_caller] // <- can remove this once it doesn't panic
    pub fn as_str(&self) -> &str {
        self.data.as_str().expect("str has surrogates")
    }

    pub fn to_str(&self) -> Option<&str> {
        self.data.as_str()
    }

    pub fn ensure_valid_utf8(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.is_utf8() {
            Ok(())
        } else {
            let start = self
                .as_wtf8()
                .code_points()
                .position(|c| c.to_char().is_none())
                .unwrap();
            Err(vm.new_unicode_encode_error_real(
                identifier!(vm, utf_8).to_owned(),
                vm.ctx.new_str(self.data.clone()),
                start,
                start + 1,
                vm.ctx.new_str("surrogates not allowed"),
            ))
        }
    }

    pub fn try_to_str(&self, vm: &VirtualMachine) -> PyResult<&str> {
        self.ensure_valid_utf8(vm)?;
        // SAFETY: ensure_valid_utf8 passed, so unwrap is safe.
        Ok(unsafe { self.to_str().unwrap_unchecked() })
    }

    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.to_str()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| self.as_wtf8().to_string_lossy())
    }

    pub const fn kind(&self) -> StrKind {
        self.data.kind()
    }

    #[inline]
    pub fn as_str_kind(&self) -> PyKindStr<'_> {
        self.data.as_str_kind()
    }

    pub const fn is_utf8(&self) -> bool {
        self.kind().is_utf8()
    }

    fn char_all<F>(&self, test: F) -> bool
    where
        F: Fn(char) -> bool,
    {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s.chars().all(|ch| test(ch.into())),
            PyKindStr::Utf8(s) => s.chars().all(test),
            PyKindStr::Wtf8(w) => w.code_points().all(|ch| ch.is_char_and(&test)),
        }
    }

    fn repeat(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if value == 0 && zelf.class().is(vm.ctx.types.str_type) {
            // Special case: when some `str` is multiplied by `0`,
            // returns the empty `str`.
            return Ok(vm.ctx.empty_str.to_owned());
        }
        if (value == 1 || zelf.is_empty()) && zelf.class().is(vm.ctx.types.str_type) {
            // Special case: when some `str` is multiplied by `1` or is the empty `str`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `str` itself, not its subclasses.
            return Ok(zelf);
        }
        zelf.as_wtf8()
            .as_bytes()
            .mul(vm, value)
            .map(|x| Self::from(unsafe { Wtf8Buf::from_bytes_unchecked(x) }).into_ref(&vm.ctx))
    }
}

#[pyclass(
    flags(BASETYPE),
    with(
        PyRef,
        AsMapping,
        AsNumber,
        AsSequence,
        Representable,
        Hashable,
        Comparable,
        Iterable,
        Constructor
    )
)]
impl PyStr {
    #[pymethod]
    fn __add__(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.downcast_ref::<Self>() {
            let bytes = zelf.as_wtf8().py_add(other.as_wtf8());
            Ok(unsafe {
                // SAFETY: `kind` is safely decided
                let kind = zelf.kind() | other.kind();
                Self::new_str_unchecked(bytes.into(), kind)
            }
            .to_pyobject(vm))
        } else if let Some(radd) = vm.get_method(other.clone(), identifier!(vm, __radd__)) {
            // hack to get around not distinguishing number add from seq concat
            radd?.call((zelf,), vm)
        } else {
            Err(vm.new_type_error(format!(
                r#"can only concatenate str (not "{}") to str"#,
                other.class().name()
            )))
        }
    }

    fn _contains(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(needle) = needle.downcast_ref::<Self>() {
            Ok(memchr::memmem::find(self.as_bytes(), needle.as_bytes()).is_some())
        } else {
            Err(vm.new_type_error(format!(
                "'in <string>' requires string as left operand, not {}",
                needle.class().name()
            )))
        }
    }

    #[pymethod]
    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self._contains(&needle, vm)
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        let item = match SequenceIndex::try_from_borrowed_object(vm, needle, "str")? {
            SequenceIndex::Int(i) => self.getitem_by_index(vm, i)?.to_pyobject(vm),
            SequenceIndex::Slice(slice) => self.getitem_by_slice(vm, slice)?.to_pyobject(vm),
        };
        Ok(item)
    }

    #[pymethod]
    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    #[inline]
    pub(crate) fn hash(&self, vm: &VirtualMachine) -> hash::PyHash {
        match self.hash.load(atomic::Ordering::Relaxed) {
            hash::SENTINEL => self._compute_hash(vm),
            hash => hash,
        }
    }

    #[cold]
    fn _compute_hash(&self, vm: &VirtualMachine) -> hash::PyHash {
        let hash_val = vm.state.hash_secret.hash_bytes(self.as_bytes());
        debug_assert_ne!(hash_val, hash::SENTINEL);
        // spell-checker:ignore cmpxchg
        // like with char_len, we don't need a cmpxchg loop, since it'll always be the same value
        self.hash.store(hash_val, atomic::Ordering::Relaxed);
        hash_val
    }

    #[inline]
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[pymethod(name = "__len__")]
    #[inline]
    pub fn char_len(&self) -> usize {
        self.data.char_len()
    }

    #[pymethod(name = "isascii")]
    #[inline(always)]
    pub const fn is_ascii(&self) -> bool {
        matches!(self.kind(), StrKind::Ascii)
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        std::mem::size_of::<Self>() + self.byte_len() * std::mem::size_of::<u8>()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod]
    fn __mul__(zelf: PyRef<Self>, value: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::repeat(zelf, value.into(), vm)
    }

    #[inline]
    pub(crate) fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        use crate::literal::escape::UnicodeEscape;
        UnicodeEscape::new_repr(self.as_wtf8())
            .str_repr()
            .to_string()
            .ok_or_else(|| vm.new_overflow_error("string is too long to generate repr"))
    }

    #[pymethod]
    fn lower(&self) -> Self {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s.to_ascii_lowercase().into(),
            PyKindStr::Utf8(s) => s.to_lowercase().into(),
            PyKindStr::Wtf8(w) => w
                .chunks()
                .map(|c| match c {
                    Wtf8Chunk::Utf8(s) => s.to_lowercase().into(),
                    Wtf8Chunk::Surrogate(c) => Wtf8Buf::from(c),
                })
                .collect::<Wtf8Buf>()
                .into(),
        }
    }

    // casefold is much more aggressive than lower
    #[pymethod]
    fn casefold(&self) -> String {
        caseless::default_case_fold_str(self.as_str())
    }

    #[pymethod]
    fn upper(&self) -> Self {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s.to_ascii_uppercase().into(),
            PyKindStr::Utf8(s) => s.to_uppercase().into(),
            PyKindStr::Wtf8(w) => w
                .chunks()
                .map(|c| match c {
                    Wtf8Chunk::Utf8(s) => s.to_uppercase().into(),
                    Wtf8Chunk::Surrogate(c) => Wtf8Buf::from(c),
                })
                .collect::<Wtf8Buf>()
                .into(),
        }
    }

    #[pymethod]
    fn capitalize(&self) -> Wtf8Buf {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => {
                let mut s = s.to_owned();
                if let [first, rest @ ..] = s.as_mut_slice() {
                    first.make_ascii_uppercase();
                    ascii::AsciiStr::make_ascii_lowercase(rest.into());
                }
                s.into()
            }
            PyKindStr::Utf8(s) => {
                let mut chars = s.chars();
                let mut out = String::with_capacity(s.len());
                if let Some(c) = chars.next() {
                    out.extend(c.to_titlecase());
                    out.push_str(&chars.as_str().to_lowercase());
                }
                out.into()
            }
            PyKindStr::Wtf8(s) => {
                let mut out = Wtf8Buf::with_capacity(s.len());
                let mut chars = s.code_points();
                if let Some(ch) = chars.next() {
                    match ch.to_char() {
                        Some(ch) => out.extend(ch.to_titlecase()),
                        None => out.push(ch),
                    }
                    for chunk in chars.as_wtf8().chunks() {
                        match chunk {
                            Wtf8Chunk::Utf8(s) => out.push_str(&s.to_lowercase()),
                            Wtf8Chunk::Surrogate(ch) => out.push(ch),
                        }
                    }
                }
                out
            }
        }
    }

    #[pymethod]
    fn split(zelf: &Py<Self>, args: SplitArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let elements = match zelf.as_str_kind() {
            PyKindStr::Ascii(s) => s.py_split(
                args,
                vm,
                || zelf.as_object().to_owned(),
                |v, s, vm| {
                    v.as_bytes()
                        .split_str(s)
                        .map(|s| unsafe { AsciiStr::from_ascii_unchecked(s) }.to_pyobject(vm))
                        .collect()
                },
                |v, s, n, vm| {
                    v.as_bytes()
                        .splitn_str(n, s)
                        .map(|s| unsafe { AsciiStr::from_ascii_unchecked(s) }.to_pyobject(vm))
                        .collect()
                },
                |v, n, vm| {
                    v.as_bytes().py_split_whitespace(n, |s| {
                        unsafe { AsciiStr::from_ascii_unchecked(s) }.to_pyobject(vm)
                    })
                },
            ),
            PyKindStr::Utf8(s) => s.py_split(
                args,
                vm,
                || zelf.as_object().to_owned(),
                |v, s, vm| v.split(s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, s, n, vm| v.splitn(n, s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, n, vm| v.py_split_whitespace(n, |s| vm.ctx.new_str(s).into()),
            ),
            PyKindStr::Wtf8(w) => w.py_split(
                args,
                vm,
                || zelf.as_object().to_owned(),
                |v, s, vm| v.split(s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, s, n, vm| v.splitn(n, s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, n, vm| v.py_split_whitespace(n, |s| vm.ctx.new_str(s).into()),
            ),
        }?;
        Ok(elements)
    }

    #[pymethod]
    fn rsplit(zelf: &Py<Self>, args: SplitArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let mut elements = zelf.as_wtf8().py_split(
            args,
            vm,
            || zelf.as_object().to_owned(),
            |v, s, vm| v.rsplit(s).map(|s| vm.ctx.new_str(s).into()).collect(),
            |v, s, n, vm| v.rsplitn(n, s).map(|s| vm.ctx.new_str(s).into()).collect(),
            |v, n, vm| v.py_rsplit_whitespace(n, |s| vm.ctx.new_str(s).into()),
        )?;
        // Unlike Python rsplit, Rust rsplitn returns an iterator that
        // starts from the end of the string.
        elements.reverse();
        Ok(elements)
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyStrRef>) -> Self {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s
                .py_strip(
                    chars,
                    |s, chars| {
                        let s = s
                            .as_str()
                            .trim_matches(|c| memchr::memchr(c as _, chars.as_bytes()).is_some());
                        unsafe { AsciiStr::from_ascii_unchecked(s.as_bytes()) }
                    },
                    |s| s.trim(),
                )
                .into(),
            PyKindStr::Utf8(s) => s
                .py_strip(
                    chars,
                    |s, chars| s.trim_matches(|c| chars.contains(c)),
                    |s| s.trim(),
                )
                .into(),
            PyKindStr::Wtf8(w) => w
                .py_strip(
                    chars,
                    |s, chars| s.trim_matches(|c| chars.code_points().contains(&c)),
                    |s| s.trim(),
                )
                .into(),
        }
    }

    #[pymethod]
    fn lstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let s = zelf.as_wtf8();
        let stripped = s.py_strip(
            chars,
            |s, chars| s.trim_start_matches(|c| chars.contains_code_point(c)),
            |s| s.trim_start(),
        );
        if s == stripped {
            zelf
        } else {
            vm.ctx.new_str(stripped)
        }
    }

    #[pymethod]
    fn rstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let s = zelf.as_wtf8();
        let stripped = s.py_strip(
            chars,
            |s, chars| s.trim_end_matches(|c| chars.contains_code_point(c)),
            |s| s.trim_end(),
        );
        if s == stripped {
            zelf
        } else {
            vm.ctx.new_str(stripped)
        }
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(self.as_wtf8(), self.len(), |s, r| s.get_chars(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "endswith",
            "str",
            |s, x: &Py<Self>| s.ends_with(x.as_wtf8()),
            vm,
        )
    }

    #[pymethod]
    fn startswith(
        &self,
        options: anystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(self.as_wtf8(), self.len(), |s, r| s.get_chars(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "startswith",
            "str",
            |s, x: &Py<Self>| s.starts_with(x.as_wtf8()),
            vm,
        )
    }

    /// Return a str with the given prefix string removed if present.
    ///
    /// If the string starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removeprefix(&self, pref: PyStrRef) -> Wtf8Buf {
        self.as_wtf8()
            .py_removeprefix(pref.as_wtf8(), pref.byte_len(), |s, p| s.starts_with(p))
            .to_owned()
    }

    /// Return a str with the given suffix string removed if present.
    ///
    /// If the string ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removesuffix(&self, suffix: PyStrRef) -> Wtf8Buf {
        self.as_wtf8()
            .py_removesuffix(suffix.as_wtf8(), suffix.byte_len(), |s, p| s.ends_with(p))
            .to_owned()
    }

    #[pymethod]
    fn isalnum(&self) -> bool {
        !self.data.is_empty() && self.char_all(char::is_alphanumeric)
    }

    #[pymethod]
    fn isnumeric(&self) -> bool {
        !self.data.is_empty() && self.char_all(char::is_numeric)
    }

    #[pymethod]
    fn isdigit(&self) -> bool {
        // python's isdigit also checks if exponents are digits, these are the unicode codepoints for exponents
        !self.data.is_empty()
            && self.char_all(|c| {
                c.is_ascii_digit()
                    || matches!(c, '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹')
            })
    }

    #[pymethod]
    fn isdecimal(&self) -> bool {
        !self.data.is_empty()
            && self.char_all(|c| GeneralCategory::of(c) == GeneralCategory::DecimalNumber)
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        cformat_string(vm, self.as_wtf8(), values)
    }

    #[pymethod]
    fn __rmod__(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod]
    fn format(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let format_str =
            FormatString::from_str(self.as_wtf8()).map_err(|e| e.to_pyexception(vm))?;
        format(&format_str, &args, vm)
    }

    /// S.format_map(mapping) -> str
    ///
    /// Return a formatted version of S, using substitutions from mapping.
    /// The substitutions are identified by braces ('{' and '}').
    #[pymethod]
    fn format_map(&self, mapping: PyObjectRef, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let format_string =
            FormatString::from_str(self.as_wtf8()).map_err(|err| err.to_pyexception(vm))?;
        format_map(&format_string, &mapping, vm)
    }

    #[pymethod(name = "__format__")]
    fn __format__(zelf: PyRef<Self>, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let spec = spec.as_str();
        if spec.is_empty() {
            return if zelf.class().is(vm.ctx.types.str_type) {
                Ok(zelf)
            } else {
                zelf.as_object().str(vm)
            };
        }

        let s = FormatSpec::parse(spec)
            .and_then(|format_spec| {
                format_spec.format_string(&CharLenStr(zelf.as_str(), zelf.char_len()))
            })
            .map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_str(s))
    }

    /// Return a titlecased version of the string where words start with an
    /// uppercase character and the remaining characters are lowercase.
    #[pymethod]
    fn title(&self) -> Wtf8Buf {
        let mut title = Wtf8Buf::with_capacity(self.data.len());
        let mut previous_is_cased = false;
        for c_orig in self.as_wtf8().code_points() {
            let c = c_orig.to_char_lossy();
            if c.is_lowercase() {
                if !previous_is_cased {
                    title.extend(c.to_titlecase());
                } else {
                    title.push_char(c);
                }
                previous_is_cased = true;
            } else if c.is_uppercase() || c.is_titlecase() {
                if previous_is_cased {
                    title.extend(c.to_lowercase());
                } else {
                    title.push_char(c);
                }
                previous_is_cased = true;
            } else {
                previous_is_cased = false;
                title.push(c_orig);
            }
        }
        title
    }

    #[pymethod]
    fn swapcase(&self) -> Wtf8Buf {
        let mut swapped_str = Wtf8Buf::with_capacity(self.data.len());
        for c_orig in self.as_wtf8().code_points() {
            let c = c_orig.to_char_lossy();
            // to_uppercase returns an iterator, to_ascii_uppercase returns the char
            if c.is_lowercase() {
                swapped_str.push_char(c.to_ascii_uppercase());
            } else if c.is_uppercase() {
                swapped_str.push_char(c.to_ascii_lowercase());
            } else {
                swapped_str.push(c_orig);
            }
        }
        swapped_str
    }

    #[pymethod]
    fn isalpha(&self) -> bool {
        !self.data.is_empty() && self.char_all(char::is_alphabetic)
    }

    #[pymethod]
    fn replace(&self, args: ReplaceArgs) -> Wtf8Buf {
        use std::cmp::Ordering;

        let s = self.as_wtf8();
        let ReplaceArgs { old, new, count } = args;

        match count.cmp(&0) {
            Ordering::Less => s.replace(old.as_wtf8(), new.as_wtf8()),
            Ordering::Equal => s.to_owned(),
            Ordering::Greater => {
                let s_is_empty = s.is_empty();
                let old_is_empty = old.is_empty();

                if s_is_empty && !old_is_empty {
                    s.to_owned()
                } else if s_is_empty && old_is_empty {
                    new.as_wtf8().to_owned()
                } else {
                    s.replacen(old.as_wtf8(), new.as_wtf8(), count as usize)
                }
            }
        }
    }

    /// Return true if all characters in the string are printable or the string is empty,
    /// false otherwise.  Nonprintable characters are those characters defined in the
    /// Unicode character database as `Other` or `Separator`,
    /// excepting the ASCII space (0x20) which is considered printable.
    ///
    /// All characters except those characters defined in the Unicode character
    /// database as following categories are considered printable.
    ///   * Cc (Other, Control)
    ///   * Cf (Other, Format)
    ///   * Cs (Other, Surrogate)
    ///   * Co (Other, Private Use)
    ///   * Cn (Other, Not Assigned)
    ///   * Zl Separator, Line ('\u2028', LINE SEPARATOR)
    ///   * Zp Separator, Paragraph ('\u2029', PARAGRAPH SEPARATOR)
    ///   * Zs (Separator, Space) other than ASCII space('\x20').
    #[pymethod]
    fn isprintable(&self) -> bool {
        self.char_all(|c| c == '\u{0020}' || rustpython_literal::char::is_printable(c))
    }

    #[pymethod]
    fn isspace(&self) -> bool {
        use unic_ucd_bidi::bidi_class::abbr_names::*;
        !self.data.is_empty()
            && self.char_all(|c| {
                GeneralCategory::of(c) == GeneralCategory::SpaceSeparator
                    || matches!(BidiClass::of(c), WS | B | S)
            })
    }

    // Return true if all cased characters in the string are lowercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s.py_islower(),
            PyKindStr::Utf8(s) => s.py_islower(),
            PyKindStr::Wtf8(w) => w.py_islower(),
        }
    }

    // Return true if all cased characters in the string are uppercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s.py_isupper(),
            PyKindStr::Utf8(s) => s.py_isupper(),
            PyKindStr::Wtf8(w) => w.py_isupper(),
        }
    }

    #[pymethod]
    fn splitlines(&self, args: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        let into_wrapper = |s: &Wtf8| self.new_substr(s.to_owned()).to_pyobject(vm);
        let mut elements = Vec::new();
        let mut last_i = 0;
        let self_str = self.as_wtf8();
        let mut enumerated = self_str.code_point_indices().peekable();
        while let Some((i, ch)) = enumerated.next() {
            let end_len = match ch.to_char_lossy() {
                '\n' => 1,
                '\r' => {
                    let is_rn = enumerated.next_if(|(_, ch)| *ch == '\n').is_some();
                    if is_rn { 2 } else { 1 }
                }
                '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{0085}' | '\u{2028}'
                | '\u{2029}' => ch.len_wtf8(),
                _ => continue,
            };
            let range = if args.keepends {
                last_i..i + end_len
            } else {
                last_i..i
            };
            last_i = i + end_len;
            elements.push(into_wrapper(&self_str[range]));
        }
        if last_i != self_str.len() {
            elements.push(into_wrapper(&self_str[last_i..]));
        }
        elements
    }

    #[pymethod]
    fn join(
        zelf: PyRef<Self>,
        iterable: ArgIterable<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        let iter = iterable.iter(vm)?;
        let joined = match iter.exactly_one() {
            Ok(first) => {
                let first = first?;
                if first.as_object().class().is(vm.ctx.types.str_type) {
                    return Ok(first);
                } else {
                    first.as_wtf8().to_owned()
                }
            }
            Err(iter) => zelf.as_wtf8().py_join(iter)?,
        };
        Ok(vm.ctx.new_str(joined))
    }

    // FIXME: two traversals of str is expensive
    #[inline]
    fn _to_char_idx(r: &Wtf8, byte_idx: usize) -> usize {
        r[..byte_idx].code_points().count()
    }

    #[inline]
    fn _find<F>(&self, args: FindArgs, find: F) -> Option<usize>
    where
        F: Fn(&Wtf8, &Wtf8) -> Option<usize>,
    {
        let (sub, range) = args.get_value(self.len());
        self.as_wtf8().py_find(sub.as_wtf8(), range, find)
    }

    #[pymethod]
    fn find(&self, args: FindArgs) -> isize {
        self._find(args, |r, s| Some(Self::_to_char_idx(r, r.find(s)?)))
            .map_or(-1, |v| v as isize)
    }

    #[pymethod]
    fn rfind(&self, args: FindArgs) -> isize {
        self._find(args, |r, s| Some(Self::_to_char_idx(r, r.rfind(s)?)))
            .map_or(-1, |v| v as isize)
    }

    #[pymethod]
    fn index(&self, args: FindArgs, vm: &VirtualMachine) -> PyResult<usize> {
        self._find(args, |r, s| Some(Self::_to_char_idx(r, r.find(s)?)))
            .ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn rindex(&self, args: FindArgs, vm: &VirtualMachine) -> PyResult<usize> {
        self._find(args, |r, s| Some(Self::_to_char_idx(r, r.rfind(s)?)))
            .ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn partition(&self, sep: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (front, has_mid, back) = self.as_wtf8().py_partition(
            sep.as_wtf8(),
            || self.as_wtf8().splitn(2, sep.as_wtf8()),
            vm,
        )?;
        let partition = (
            self.new_substr(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_str(ascii!(""))
            },
            self.new_substr(back),
        );
        Ok(partition.to_pyobject(vm))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (back, has_mid, front) = self.as_wtf8().py_partition(
            sep.as_wtf8(),
            || self.as_wtf8().rsplitn(2, sep.as_wtf8()),
            vm,
        )?;
        Ok((
            self.new_substr(front),
            if has_mid {
                sep
            } else {
                vm.ctx.empty_str.to_owned()
            },
            self.new_substr(back),
        )
            .to_pyobject(vm))
    }

    /// Return `true` if the sequence is ASCII titlecase and the sequence is not
    /// empty, `false` otherwise.
    #[pymethod]
    fn istitle(&self) -> bool {
        if self.data.is_empty() {
            return false;
        }

        let mut cased = false;
        let mut previous_is_cased = false;
        for c in self.as_wtf8().code_points().map(CodePoint::to_char_lossy) {
            if c.is_uppercase() || c.is_titlecase() {
                if previous_is_cased {
                    return false;
                }
                previous_is_cased = true;
                cased = true;
            } else if c.is_lowercase() {
                if !previous_is_cased {
                    return false;
                }
                previous_is_cased = true;
                cased = true;
            } else {
                previous_is_cased = false;
            }
        }
        cased
    }

    #[pymethod]
    fn count(&self, args: FindArgs) -> usize {
        let (needle, range) = args.get_value(self.len());
        self.as_wtf8()
            .py_count(needle.as_wtf8(), range, |h, n| h.find_iter(n).count())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> Wtf8Buf {
        unsafe {
            // SAFETY: this is safe-guaranteed because the original self.as_wtf8() is valid wtf8
            Wtf8Buf::from_bytes_unchecked(self.as_wtf8().py_zfill(width))
        }
    }

    #[inline]
    fn _pad(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        pad: fn(&Wtf8, usize, CodePoint, usize) -> Wtf8Buf,
        vm: &VirtualMachine,
    ) -> PyResult<Wtf8Buf> {
        let fillchar = fillchar.map_or(Ok(' '.into()), |ref s| {
            s.as_wtf8().code_points().exactly_one().map_err(|_| {
                vm.new_type_error("The fill character must be exactly one character long")
            })
        })?;
        Ok(if self.len() as isize >= width {
            self.as_wtf8().to_owned()
        } else {
            pad(self.as_wtf8(), width as usize, fillchar, self.len())
        })
    }

    #[pymethod]
    fn center(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Wtf8Buf> {
        self._pad(width, fillchar, AnyStr::py_center, vm)
    }

    #[pymethod]
    fn ljust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Wtf8Buf> {
        self._pad(width, fillchar, AnyStr::py_ljust, vm)
    }

    #[pymethod]
    fn rjust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Wtf8Buf> {
        self._pad(width, fillchar, AnyStr::py_rjust, vm)
    }

    #[pymethod]
    fn expandtabs(&self, args: anystr::ExpandTabsArgs) -> String {
        rustpython_common::str::expandtabs(self.as_str(), args.tabsize())
    }

    #[pymethod]
    fn isidentifier(&self) -> bool {
        let Some(s) = self.to_str() else { return false };
        let mut chars = s.chars();
        let is_identifier_start = chars.next().is_some_and(|c| c == '_' || is_xid_start(c));
        // a string is not an identifier if it has whitespace or starts with a number
        is_identifier_start && chars.all(is_xid_continue)
    }

    // https://docs.python.org/3/library/stdtypes.html#str.translate
    #[pymethod]
    fn translate(&self, table: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        vm.get_method_or_type_error(table.clone(), identifier!(vm, __getitem__), || {
            format!("'{}' object is not subscriptable", table.class().name())
        })?;

        let mut translated = String::new();
        for c in self.as_str().chars() {
            match table.get_item(&*(c as u32).to_pyobject(vm), vm) {
                Ok(value) => {
                    if let Some(text) = value.downcast_ref::<Self>() {
                        translated.push_str(text.as_str());
                    } else if let Some(bigint) = value.downcast_ref::<PyInt>() {
                        let ch = bigint
                            .as_bigint()
                            .to_u32()
                            .and_then(std::char::from_u32)
                            .ok_or_else(|| {
                                vm.new_value_error("character mapping must be in range(0x110000)")
                            })?;
                        translated.push(ch);
                    } else if !vm.is_none(&value) {
                        return Err(
                            vm.new_type_error("character mapping must return integer, None or str")
                        );
                    }
                }
                _ => translated.push(c),
            }
        }
        Ok(translated)
    }

    #[pystaticmethod]
    fn maketrans(
        dict_or_str: PyObjectRef,
        to_str: OptionalArg<PyStrRef>,
        none_str: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let new_dict = vm.ctx.new_dict();
        if let OptionalArg::Present(to_str) = to_str {
            match dict_or_str.downcast::<Self>() {
                Ok(from_str) => {
                    if to_str.len() == from_str.len() {
                        for (c1, c2) in from_str.as_str().chars().zip(to_str.as_str().chars()) {
                            new_dict.set_item(
                                &*vm.new_pyobj(c1 as u32),
                                vm.new_pyobj(c2 as u32),
                                vm,
                            )?;
                        }
                        if let OptionalArg::Present(none_str) = none_str {
                            for c in none_str.as_str().chars() {
                                new_dict.set_item(&*vm.new_pyobj(c as u32), vm.ctx.none(), vm)?;
                            }
                        }
                        Ok(new_dict.to_pyobject(vm))
                    } else {
                        Err(vm.new_value_error(
                            "the first two maketrans arguments must have equal length",
                        ))
                    }
                }
                _ => Err(vm.new_type_error(
                    "first maketrans argument must be a string if there is a second argument",
                )),
            }
        } else {
            // dict_str must be a dict
            match dict_or_str.downcast::<PyDict>() {
                Ok(dict) => {
                    for (key, val) in dict {
                        // FIXME: ints are key-compatible
                        if let Some(num) = key.downcast_ref::<PyInt>() {
                            new_dict.set_item(
                                &*num.as_bigint().to_i32().to_pyobject(vm),
                                val,
                                vm,
                            )?;
                        } else if let Some(string) = key.downcast_ref::<Self>() {
                            if string.len() == 1 {
                                let num_value = string.as_str().chars().next().unwrap() as u32;
                                new_dict.set_item(&*num_value.to_pyobject(vm), val, vm)?;
                            } else {
                                return Err(vm.new_value_error(
                                    "string keys in translate table must be of length 1",
                                ));
                            }
                        } else {
                            return Err(vm.new_type_error(
                                "keys in translate table must be strings or integers",
                            ));
                        }
                    }
                    Ok(new_dict.to_pyobject(vm))
                }
                _ => Err(vm.new_value_error(
                    "if you give only one argument to maketrans it must be a dict",
                )),
            }
        }
    }

    #[pymethod]
    fn encode(zelf: PyRef<Self>, args: EncodeArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        encode_string(zelf, args.encoding, args.errors, vm)
    }

    #[pymethod]
    fn __getnewargs__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
        (zelf.as_str(),).to_pyobject(vm)
    }
}

struct CharLenStr<'a>(&'a str, usize);
impl std::ops::Deref for CharLenStr<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}
impl crate::common::format::CharLen for CharLenStr<'_> {
    fn char_len(&self) -> usize {
        self.1
    }
}

#[pyclass]
impl PyRef<PyStr> {
    #[pymethod]
    fn __str__(self, vm: &VirtualMachine) -> PyRefExact<PyStr> {
        self.into_exact_or(&vm.ctx, |zelf| {
            PyStr::from(zelf.data.clone()).into_exact_ref(&vm.ctx)
        })
    }
}

impl PyStrRef {
    pub fn is_empty(&self) -> bool {
        (**self).is_empty()
    }

    pub fn concat_in_place(&mut self, other: &Wtf8, vm: &VirtualMachine) {
        // TODO: call [A]Rc::get_mut on the str to try to mutate the data in place
        if other.is_empty() {
            return;
        }
        let mut s = Wtf8Buf::with_capacity(self.byte_len() + other.len());
        s.push_wtf8(self.as_ref());
        s.push_wtf8(other);
        *self = PyStr::from(s).into_ref(&vm.ctx);
    }

    pub fn try_into_utf8(self, vm: &VirtualMachine) -> PyResult<PyRef<PyUtf8Str>> {
        self.ensure_valid_utf8(vm)?;
        Ok(unsafe { mem::transmute::<PyRef<PyStr>, PyRef<PyUtf8Str>>(self) })
    }
}

impl Representable for PyStr {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.repr(vm)
    }
}

impl Hashable for PyStr {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.hash(vm))
    }
}

impl Comparable for PyStr {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        Ok(op.eval_ord(zelf.as_wtf8().cmp(other.as_wtf8())).into())
    }
}

impl Iterable for PyStr {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyStrIterator {
            internal: PyMutex::new((PositionIterInternal::new(zelf, 0), 0)),
        }
        .into_pyobject(vm))
    }
}

impl AsMapping for PyStr {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyStr::mapping_downcast(mapping).len())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyStr::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsNumber for PyStr {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            remainder: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyStr>() {
                    a.modulo(b.to_owned(), vm).to_pyresult(vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl AsSequence for PyStr {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyStr::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                PyStr::__add__(zelf.to_owned(), other.to_owned(), vm)
            }),
            repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                PyStr::repeat(zelf.to_owned(), n, vm).map(|x| x.into())
            }),
            item: atomic_func!(|seq, i, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                zelf.getitem_by_index(vm, i).to_pyresult(vm)
            }),
            contains: atomic_func!(
                |seq, needle, vm| PyStr::sequence_downcast(seq)._contains(needle, vm)
            ),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

#[derive(FromArgs)]
struct EncodeArgs {
    #[pyarg(any, default)]
    encoding: Option<PyStrRef>,
    #[pyarg(any, default)]
    errors: Option<PyStrRef>,
}

pub(crate) fn encode_string(
    s: PyStrRef,
    encoding: Option<PyStrRef>,
    errors: Option<PyStrRef>,
    vm: &VirtualMachine,
) -> PyResult<PyBytesRef> {
    let encoding = encoding
        .as_ref()
        .map_or(crate::codecs::DEFAULT_ENCODING, |s| s.as_str());
    vm.state.codec_registry.encode_text(s, encoding, errors, vm)
}

impl PyPayload for PyStr {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.str_type
    }
}

impl ToPyObject for String {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for Wtf8Buf {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for char {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for CodePoint {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for &str {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for &String {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self.clone()).into()
    }
}

impl ToPyObject for &Wtf8 {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for &Wtf8Buf {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self.clone()).into()
    }
}

impl ToPyObject for &AsciiStr {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for AsciiString {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for AsciiChar {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

type SplitArgs = anystr::SplitArgs<PyStrRef>;

#[derive(FromArgs)]
pub struct FindArgs {
    #[pyarg(positional)]
    sub: PyStrRef,
    #[pyarg(positional, default)]
    start: Option<PyIntRef>,
    #[pyarg(positional, default)]
    end: Option<PyIntRef>,
}

impl FindArgs {
    fn get_value(self, len: usize) -> (PyStrRef, std::ops::Range<usize>) {
        let range = adjust_indices(self.start, self.end, len);
        (self.sub, range)
    }
}

#[derive(FromArgs)]
struct ReplaceArgs {
    #[pyarg(positional)]
    old: PyStrRef,

    #[pyarg(positional)]
    new: PyStrRef,

    #[pyarg(any, default = -1)]
    count: isize,
}

pub fn init(ctx: &Context) {
    PyStr::extend_class(ctx, ctx.types.str_type);

    PyStrIterator::extend_class(ctx, ctx.types.str_iterator_type);
}

impl SliceableSequenceOp for PyStr {
    type Item = CodePoint;
    type Sliced = Self;

    fn do_get(&self, index: usize) -> Self::Item {
        self.data.nth_char(index)
    }

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s[range].into(),
            PyKindStr::Utf8(s) => {
                let char_len = range.len();
                let out = rustpython_common::str::get_chars(s, range);
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
            PyKindStr::Wtf8(w) => {
                let char_len = range.len();
                let out = rustpython_common::str::get_codepoints(w, range);
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
        }
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => {
                let mut out = s[range].to_owned();
                out.as_mut_slice().reverse();
                out.into()
            }
            PyKindStr::Utf8(s) => {
                let char_len = range.len();
                let mut out = String::with_capacity(2 * char_len);
                out.extend(
                    s.chars()
                        .rev()
                        .skip(self.char_len() - range.end)
                        .take(range.len()),
                );
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, range.len()) }
            }
            PyKindStr::Wtf8(w) => {
                let char_len = range.len();
                let mut out = Wtf8Buf::with_capacity(2 * char_len);
                out.extend(
                    w.code_points()
                        .rev()
                        .skip(self.char_len() - range.end)
                        .take(range.len()),
                );
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
        }
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s[range]
                .as_slice()
                .iter()
                .copied()
                .step_by(step)
                .collect::<AsciiString>()
                .into(),
            PyKindStr::Utf8(s) => {
                let char_len = (range.len() / step) + 1;
                let mut out = String::with_capacity(2 * char_len);
                out.extend(s.chars().skip(range.start).take(range.len()).step_by(step));
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
            PyKindStr::Wtf8(w) => {
                let char_len = (range.len() / step) + 1;
                let mut out = Wtf8Buf::with_capacity(2 * char_len);
                out.extend(
                    w.code_points()
                        .skip(range.start)
                        .take(range.len())
                        .step_by(step),
                );
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
        }
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        match self.as_str_kind() {
            PyKindStr::Ascii(s) => s[range]
                .chars()
                .rev()
                .step_by(step)
                .collect::<AsciiString>()
                .into(),
            PyKindStr::Utf8(s) => {
                let char_len = (range.len() / step) + 1;
                // not ascii, so the codepoints have to be at least 2 bytes each
                let mut out = String::with_capacity(2 * char_len);
                out.extend(
                    s.chars()
                        .rev()
                        .skip(self.char_len() - range.end)
                        .take(range.len())
                        .step_by(step),
                );
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
            PyKindStr::Wtf8(w) => {
                let char_len = (range.len() / step) + 1;
                // not ascii, so the codepoints have to be at least 2 bytes each
                let mut out = Wtf8Buf::with_capacity(2 * char_len);
                out.extend(
                    w.code_points()
                        .rev()
                        .skip(self.char_len() - range.end)
                        .take(range.len())
                        .step_by(step),
                );
                // SAFETY: char_len is accurate
                unsafe { Self::new_with_char_len(out, char_len) }
            }
        }
    }

    fn empty() -> Self::Sliced {
        Self::default()
    }

    fn len(&self) -> usize {
        self.char_len()
    }
}

impl AsRef<str> for PyRefExact<PyStr> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for PyExact<PyStr> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<Wtf8> for PyRefExact<PyStr> {
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl AsRef<Wtf8> for PyExact<PyStr> {
    fn as_ref(&self) -> &Wtf8 {
        self.as_wtf8()
    }
}

impl AnyStrWrapper<Wtf8> for PyStrRef {
    fn as_ref(&self) -> Option<&Wtf8> {
        Some(self.as_wtf8())
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl AnyStrWrapper<str> for PyStrRef {
    fn as_ref(&self) -> Option<&str> {
        self.data.as_str()
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl AnyStrWrapper<AsciiStr> for PyStrRef {
    fn as_ref(&self) -> Option<&AsciiStr> {
        self.data.as_ascii()
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl AnyStrContainer<str> for String {
    fn new() -> Self {
        Self::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &str) {
        Self::push_str(self, other)
    }
}

impl anystr::AnyChar for char {
    fn is_lowercase(self) -> bool {
        self.is_lowercase()
    }

    fn is_uppercase(self) -> bool {
        self.is_uppercase()
    }

    fn bytes_len(self) -> usize {
        self.len_utf8()
    }
}

impl AnyStr for str {
    type Char = char;
    type Container = String;

    fn to_container(&self) -> Self::Container {
        self.to_owned()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn elements(&self) -> impl Iterator<Item = char> {
        Self::chars(self)
    }

    fn get_bytes(&self, range: std::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn get_chars(&self, range: std::ops::Range<usize>) -> &Self {
        rustpython_common::str::get_chars(self, range)
    }

    fn is_empty(&self) -> bool {
        Self::is_empty(self)
    }

    fn bytes_len(&self) -> usize {
        Self::len(self)
    }

    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython split_whitespace
        let mut splits = Vec::new();
        let mut last_offset = 0;
        let mut count = maxsplit;
        for (offset, _) in self.match_indices(|c: char| c.is_ascii_whitespace() || c == '\x0b') {
            if last_offset == offset {
                last_offset += 1;
                continue;
            }
            if count == 0 {
                break;
            }
            splits.push(convert(&self[last_offset..offset]));
            last_offset = offset + 1;
            count -= 1;
        }
        if last_offset != self.len() {
            splits.push(convert(&self[last_offset..]));
        }
        splits
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython rsplit_whitespace
        let mut splits = Vec::new();
        let mut last_offset = self.len();
        let mut count = maxsplit;
        for (offset, _) in self.rmatch_indices(|c: char| c.is_ascii_whitespace() || c == '\x0b') {
            if last_offset == offset + 1 {
                last_offset -= 1;
                continue;
            }
            if count == 0 {
                break;
            }
            splits.push(convert(&self[offset + 1..last_offset]));
            last_offset = offset;
            count -= 1;
        }
        if last_offset != 0 {
            splits.push(convert(&self[..last_offset]));
        }
        splits
    }
}

impl AnyStrContainer<Wtf8> for Wtf8Buf {
    fn new() -> Self {
        Self::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &Wtf8) {
        self.push_wtf8(other)
    }
}

impl anystr::AnyChar for CodePoint {
    fn is_lowercase(self) -> bool {
        self.is_char_and(char::is_lowercase)
    }
    fn is_uppercase(self) -> bool {
        self.is_char_and(char::is_uppercase)
    }
    fn bytes_len(self) -> usize {
        self.len_wtf8()
    }
}

impl AnyStr for Wtf8 {
    type Char = CodePoint;
    type Container = Wtf8Buf;

    fn to_container(&self) -> Self::Container {
        self.to_owned()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn elements(&self) -> impl Iterator<Item = Self::Char> {
        self.code_points()
    }

    fn get_bytes(&self, range: std::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn get_chars(&self, range: std::ops::Range<usize>) -> &Self {
        rustpython_common::str::get_codepoints(self, range)
    }

    fn bytes_len(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython split_whitespace
        let mut splits = Vec::new();
        let mut last_offset = 0;
        let mut count = maxsplit;
        for (offset, _) in self
            .code_point_indices()
            .filter(|(_, c)| c.is_char_and(|c| c.is_ascii_whitespace() || c == '\x0b'))
        {
            if last_offset == offset {
                last_offset += 1;
                continue;
            }
            if count == 0 {
                break;
            }
            splits.push(convert(&self[last_offset..offset]));
            last_offset = offset + 1;
            count -= 1;
        }
        if last_offset != self.len() {
            splits.push(convert(&self[last_offset..]));
        }
        splits
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython rsplit_whitespace
        let mut splits = Vec::new();
        let mut last_offset = self.len();
        let mut count = maxsplit;
        for (offset, _) in self
            .code_point_indices()
            .rev()
            .filter(|(_, c)| c.is_char_and(|c| c.is_ascii_whitespace() || c == '\x0b'))
        {
            if last_offset == offset + 1 {
                last_offset -= 1;
                continue;
            }
            if count == 0 {
                break;
            }
            splits.push(convert(&self[offset + 1..last_offset]));
            last_offset = offset;
            count -= 1;
        }
        if last_offset != 0 {
            splits.push(convert(&self[..last_offset]));
        }
        splits
    }
}

impl AnyStrContainer<AsciiStr> for AsciiString {
    fn new() -> Self {
        Self::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &AsciiStr) {
        Self::push_str(self, other)
    }
}

impl anystr::AnyChar for ascii::AsciiChar {
    fn is_lowercase(self) -> bool {
        self.is_lowercase()
    }

    fn is_uppercase(self) -> bool {
        self.is_uppercase()
    }

    fn bytes_len(self) -> usize {
        1
    }
}

const ASCII_WHITESPACES: [u8; 6] = [0x20, 0x09, 0x0a, 0x0c, 0x0d, 0x0b];

impl AnyStr for AsciiStr {
    type Char = AsciiChar;
    type Container = AsciiString;

    fn to_container(&self) -> Self::Container {
        self.to_ascii_string()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn elements(&self) -> impl Iterator<Item = Self::Char> {
        self.chars()
    }

    fn get_bytes(&self, range: std::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn get_chars(&self, range: std::ops::Range<usize>) -> &Self {
        &self[range]
    }

    fn bytes_len(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn py_split_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        let mut splits = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.as_bytes().find_byteset(ASCII_WHITESPACES) {
            if offset != 0 {
                if count == 0 {
                    break;
                }
                splits.push(convert(&haystack[..offset]));
                count -= 1;
            }
            haystack = &haystack[offset + 1..];
        }
        if !haystack.is_empty() {
            splits.push(convert(haystack));
        }
        splits
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython rsplit_whitespace
        let mut splits = Vec::new();
        let mut count = maxsplit;
        let mut haystack = self;
        while let Some(offset) = haystack.as_bytes().rfind_byteset(ASCII_WHITESPACES) {
            if offset + 1 != haystack.len() {
                if count == 0 {
                    break;
                }
                splits.push(convert(&haystack[offset + 1..]));
                count -= 1;
            }
            haystack = &haystack[..offset];
        }
        if !haystack.is_empty() {
            splits.push(convert(haystack));
        }
        splits
    }
}

/// The unique reference of interned PyStr
/// Always intended to be used as a static reference
pub type PyStrInterned = PyInterned<PyStr>;

impl PyStrInterned {
    #[inline]
    pub fn to_exact(&'static self) -> PyRefExact<PyStr> {
        unsafe { PyRefExact::new_unchecked(self.to_owned()) }
    }
}

impl std::fmt::Display for PyStrInterned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.data.fmt(f)
    }
}

impl AsRef<str> for PyStrInterned {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn str_title() {
        let tests = vec![
            (" Hello ", " hello "),
            ("Hello ", "hello "),
            ("Hello ", "Hello "),
            ("Format This As Title String", "fOrMaT thIs aS titLe String"),
            ("Format,This-As*Title;String", "fOrMaT,thIs-aS*titLe;String"),
            ("Getint", "getInt"),
            // spell-checker:disable-next-line
            ("Greek Ωppercases ...", "greek ωppercases ..."),
            // spell-checker:disable-next-line
            ("Greek ῼitlecases ...", "greek ῳitlecases ..."),
        ];
        for (title, input) in tests {
            assert_eq!(PyStr::from(input).title().as_str(), Ok(title));
        }
    }

    #[test]
    fn str_istitle() {
        let pos = vec![
            "A",
            "A Titlecased Line",
            "A\nTitlecased Line",
            "A Titlecased, Line",
            // spell-checker:disable-next-line
            "Greek Ωppercases ...",
            // spell-checker:disable-next-line
            "Greek ῼitlecases ...",
        ];

        for s in pos {
            assert!(PyStr::from(s).istitle());
        }

        let neg = vec![
            "",
            "a",
            "\n",
            "Not a capitalized String",
            "Not\ta Titlecase String",
            "Not--a Titlecase String",
            "NOT",
        ];
        for s in neg {
            assert!(!PyStr::from(s).istitle());
        }
    }

    #[test]
    fn str_maketrans_and_translate() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let table = vm.ctx.new_dict();
            table
                .set_item("a", vm.ctx.new_str("🎅").into(), vm)
                .unwrap();
            table.set_item("b", vm.ctx.none(), vm).unwrap();
            table
                .set_item("c", vm.ctx.new_str(ascii!("xda")).into(), vm)
                .unwrap();
            let translated =
                PyStr::maketrans(table.into(), OptionalArg::Missing, OptionalArg::Missing, vm)
                    .unwrap();
            let text = PyStr::from("abc");
            let translated = text.translate(translated, vm).unwrap();
            assert_eq!(translated, "🎅xda".to_owned());
            let translated = text.translate(vm.ctx.new_int(3).into(), vm);
            assert_eq!("TypeError", &*translated.unwrap_err().class().name(),);
        })
    }
}

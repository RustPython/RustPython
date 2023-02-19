use super::{
    int::{PyInt, PyIntRef},
    iter::IterStatus::{self, Exhausted},
    PositionIterInternal, PyBytesRef, PyDict, PyTupleRef, PyType, PyTypeRef,
};
use crate::{
    anystr::{self, adjust_indices, AnyStr, AnyStrContainer, AnyStrWrapper},
    atomic_func,
    class::PyClassImpl,
    common::{
        format::{FormatSpec, FormatString, FromTemplate},
        str::{BorrowedStr, PyStrKind, PyStrKindData},
    },
    convert::{IntoPyException, ToPyException, ToPyObject},
    format::{format, format_map},
    function::{ArgIterable, FuncArgs, OptionalArg, OptionalOption, PyComparisonValue},
    intern::PyInterned,
    protocol::{PyIterReturn, PyMappingMethods, PySequenceMethods},
    sequence::SequenceExt,
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsMapping, AsSequence, Comparable, Constructor, Hashable, IterNext, IterNextIterable,
        Iterable, PyComparisonOp, Unconstructible,
    },
    AsObject, Context, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromBorrowedObject, VirtualMachine,
};
use ascii::{AsciiStr, AsciiString};
use bstr::ByteSlice;
use itertools::Itertools;
use num_traits::ToPrimitive;
use once_cell::sync::Lazy;
use rustpython_common::{
    ascii,
    atomic::{self, PyAtomic, Radium},
    hash,
    lock::PyMutex,
};
use std::{char, fmt, ops::Range, string::ToString};
use unic_ucd_bidi::BidiClass;
use unic_ucd_category::GeneralCategory;
use unic_ucd_ident::{is_xid_continue, is_xid_start};
use unicode_casing::CharExt;

impl TryFromBorrowedObject for String {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
        obj.try_value_with(|pystr: &PyStr| Ok(pystr.as_str().to_owned()), vm)
    }
}

#[pyclass(module = false, name = "str")]
pub struct PyStr {
    bytes: Box<[u8]>,
    kind: PyStrKindData,
    hash: PyAtomic<hash::PyHash>,
}

impl fmt::Debug for PyStr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("PyStr")
            .field("value", &self.as_str())
            .field("kind", &self.kind)
            .field("hash", &self.hash)
            .finish()
    }
}

impl AsRef<str> for PyStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Py<PyStr> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for PyStrRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'a> From<&'a AsciiStr> for PyStr {
    fn from(s: &'a AsciiStr) -> Self {
        s.to_owned().into()
    }
}

impl From<AsciiString> for PyStr {
    fn from(s: AsciiString) -> Self {
        unsafe { Self::new_ascii_unchecked(s.into()) }
    }
}

impl<'a> From<&'a str> for PyStr {
    fn from(s: &'a str) -> Self {
        s.to_owned().into()
    }
}

impl From<String> for PyStr {
    fn from(s: String) -> Self {
        s.into_boxed_str().into()
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
        // doing the check is ~10x faster for ascii, and is actually only 2% slower worst case for
        // non-ascii; see https://github.com/RustPython/RustPython/pull/2586#issuecomment-844611532
        let is_ascii = value.is_ascii();
        let bytes = value.into_boxed_bytes();
        let kind = if is_ascii {
            PyStrKind::Ascii
        } else {
            PyStrKind::Utf8
        }
        .new_data();
        Self {
            bytes,
            kind,
            hash: Radium::new(hash::SENTINEL),
        }
    }
}

pub type PyStrRef = PyRef<PyStr>;

impl fmt::Display for PyStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

pub trait IntoPyStrRef {
    fn into_pystr_ref(self, vm: &VirtualMachine) -> PyStrRef;
}

impl IntoPyStrRef for PyStrRef {
    #[inline]
    fn into_pystr_ref(self, _vm: &VirtualMachine) -> PyRef<PyStr> {
        self
    }
}

impl IntoPyStrRef for PyStr {
    #[inline]
    fn into_pystr_ref(self, vm: &VirtualMachine) -> PyRef<PyStr> {
        self.into_ref(vm)
    }
}

impl IntoPyStrRef for AsciiString {
    #[inline]
    fn into_pystr_ref(self, vm: &VirtualMachine) -> PyRef<PyStr> {
        PyStr::from(self).into_ref(vm)
    }
}

impl IntoPyStrRef for String {
    #[inline]
    fn into_pystr_ref(self, vm: &VirtualMachine) -> PyRef<PyStr> {
        PyStr::from(self).into_ref(vm)
    }
}

impl IntoPyStrRef for &str {
    #[inline]
    fn into_pystr_ref(self, vm: &VirtualMachine) -> PyRef<PyStr> {
        PyStr::from(self).into_ref(vm)
    }
}

impl IntoPyStrRef for &'static PyStrInterned {
    #[inline]
    fn into_pystr_ref(self, _vm: &VirtualMachine) -> PyRef<PyStr> {
        self.to_owned()
    }
}

#[pyclass(module = false, name = "str_iterator")]
#[derive(Debug)]
pub struct PyStrIterator {
    internal: PyMutex<(PositionIterInternal<PyStrRef>, usize)>,
}

impl PyPayload for PyStrIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.str_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyStrIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().0.length_hint(|obj| obj.char_len())
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut internal = self.internal.lock();
        internal.1 = usize::MAX;
        internal
            .0
            .set_state(state, |obj, pos| pos.min(obj.char_len()), vm)
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .0
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }
}
impl Unconstructible for PyStrIterator {}

impl IterNextIterable for PyStrIterator {}
impl IterNext for PyStrIterator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut internal = zelf.internal.lock();

        if let IterStatus::Active(s) = &internal.0.status {
            let value = s.as_str();

            if internal.1 == usize::MAX {
                if let Some((offset, ch)) = value.char_indices().nth(internal.0.position) {
                    internal.0.position += 1;
                    internal.1 = offset + ch.len_utf8();
                    return Ok(PyIterReturn::Return(ch.to_pyobject(vm)));
                }
            } else if let Some(value) = value.get(internal.1..) {
                if let Some(ch) = value.chars().next() {
                    internal.0.position += 1;
                    internal.1 += ch.len_utf8();
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
                PyStr::from(String::new()).into_ref_with_type(vm, cls.clone())?
            }
        };
        if string.class().is(&cls) {
            Ok(string.into())
        } else {
            PyStr::from(string.as_str())
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }
}

impl PyStr {
    /// # Safety: Given `bytes` must be valid data for given `kind`
    pub(crate) unsafe fn new_str_unchecked(bytes: Vec<u8>, kind: PyStrKind) -> Self {
        let s = Self {
            bytes: bytes.into_boxed_slice(),
            kind: kind.new_data(),
            hash: Radium::new(hash::SENTINEL),
        };
        debug_assert!(matches!(s.kind, PyStrKindData::Ascii) || !s.as_str().is_ascii());
        s
    }

    /// # Safety
    /// Given `bytes` must be ascii
    pub unsafe fn new_ascii_unchecked(bytes: Vec<u8>) -> Self {
        Self::new_str_unchecked(bytes, PyStrKind::Ascii)
    }

    pub fn new_ref(s: impl Into<Self>, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(s.into(), ctx.types.str_type.to_owned(), None)
    }

    fn new_substr(&self, s: String) -> Self {
        let kind = if self.kind.kind() == PyStrKind::Ascii || s.is_ascii() {
            PyStrKind::Ascii
        } else {
            PyStrKind::Utf8
        };
        unsafe {
            // SAFETY: kind is properly decided for substring
            Self::new_str_unchecked(s.into_bytes(), kind)
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        unsafe {
            // SAFETY: Both PyStrKind::{Ascii, Utf8} are valid utf8 string
            std::str::from_utf8_unchecked(&self.bytes)
        }
    }

    fn char_all<F>(&self, test: F) -> bool
    where
        F: Fn(char) -> bool,
    {
        match self.kind.kind() {
            PyStrKind::Ascii => self.bytes.iter().all(|&x| test(char::from(x))),
            PyStrKind::Utf8 => self.as_str().chars().all(test),
        }
    }

    fn borrow(&self) -> &BorrowedStr {
        unsafe { std::mem::transmute(self) }
    }
}

#[pyclass(
    flags(BASETYPE),
    with(AsMapping, AsSequence, Hashable, Comparable, Iterable, Constructor)
)]
impl PyStr {
    #[pymethod(magic)]
    fn add(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyStr>() {
            let bytes = zelf.as_str().py_add(other.as_ref());
            Ok(unsafe {
                // SAFETY: `kind` is safely decided
                let kind = zelf.kind.kind() | other.kind.kind();
                Self::new_str_unchecked(bytes.into_bytes(), kind)
            }
            .to_pyobject(vm))
        } else if let Some(radd) = vm.get_method(other.clone(), identifier!(vm, __radd__)) {
            // hack to get around not distinguishing number add from seq concat
            vm.invoke(&radd?, (zelf,))
        } else {
            Err(vm.new_type_error(format!(
                "can only concatenate str (not \"{}\") to str",
                other.class().name()
            )))
        }
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.bytes.is_empty()
    }

    fn _contains(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(needle) = needle.payload::<Self>() {
            Ok(self.as_str().contains(needle.as_str()))
        } else {
            Err(vm.new_type_error(format!(
                "'in <string>' requires string as left operand, not {}",
                needle.class().name()
            )))
        }
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self._contains(&needle, vm)
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "str")? {
            SequenceIndex::Int(i) => self.getitem_by_index(vm, i).map(|x| x.to_string()),
            SequenceIndex::Slice(slice) => self.getitem_by_slice(vm, slice),
        }
        .map(|x| self.new_substr(x).into_ref(vm).into())
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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
        let hash_val = vm.state.hash_secret.hash_str(self.as_str());
        debug_assert_ne!(hash_val, hash::SENTINEL);
        // like with char_len, we don't need a cmpxchg loop, since it'll always be the same value
        self.hash.store(hash_val, atomic::Ordering::Relaxed);
        hash_val
    }

    #[inline]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[pymethod(name = "__len__")]
    #[inline]
    pub fn char_len(&self) -> usize {
        self.borrow().char_len()
    }

    #[pymethod(name = "isascii")]
    #[inline(always)]
    pub fn is_ascii(&self) -> bool {
        match self.kind {
            PyStrKindData::Ascii => true,
            PyStrKindData::Utf8(_) => false,
        }
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        std::mem::size_of::<Self>() + self.byte_len() * std::mem::size_of::<u8>()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if value == 0 && zelf.class().is(vm.ctx.types.str_type) {
            // Special case: when some `str` is multiplied by `0`,
            // returns the empty `str`.
            return Ok(vm.ctx.empty_str.clone());
        }
        if (value == 1 || zelf.is_empty()) && zelf.class().is(vm.ctx.types.str_type) {
            // Special case: when some `str` is multiplied by `1` or is the empty `str`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `str` itself, not its subclasses.
            return Ok(zelf);
        }
        zelf.as_str()
            .as_bytes()
            .mul(vm, value)
            .map(|x| Self::from(unsafe { String::from_utf8_unchecked(x) }).into_ref(vm))
    }

    #[pymethod(magic)]
    fn str(zelf: PyRef<Self>) -> PyStrRef {
        zelf
    }

    #[pymethod(magic)]
    pub(crate) fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        rustpython_common::str::repr(self.as_str())
            .to_string_checked()
            .map_err(|err| vm.new_overflow_error(err.to_string()))
    }

    #[pymethod]
    fn lower(&self) -> String {
        match self.kind.kind() {
            PyStrKind::Ascii => self.as_str().to_ascii_lowercase(),
            PyStrKind::Utf8 => self.as_str().to_lowercase(),
        }
    }

    // casefold is much more aggressive than lower
    #[pymethod]
    fn casefold(&self) -> String {
        caseless::default_case_fold_str(self.as_str())
    }

    #[pymethod]
    fn upper(&self) -> String {
        match self.kind.kind() {
            PyStrKind::Ascii => self.as_str().to_ascii_uppercase(),
            PyStrKind::Utf8 => self.as_str().to_uppercase(),
        }
    }

    #[pymethod]
    fn capitalize(&self) -> String {
        let mut chars = self.as_str().chars();
        if let Some(first_char) = chars.next() {
            format!(
                "{}{}",
                first_char.to_uppercase(),
                &chars.as_str().to_lowercase(),
            )
        } else {
            "".to_owned()
        }
    }

    #[pymethod]
    fn split(&self, args: SplitArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let elements = match self.kind.kind() {
            PyStrKind::Ascii => self.as_str().py_split(
                args,
                vm,
                |v, s, vm| {
                    v.as_bytes()
                        .split_str(s)
                        .map(|s| {
                            unsafe { PyStr::new_ascii_unchecked(s.to_owned()) }.to_pyobject(vm)
                        })
                        .collect()
                },
                |v, s, n, vm| {
                    v.as_bytes()
                        .splitn_str(n, s)
                        .map(|s| {
                            unsafe { PyStr::new_ascii_unchecked(s.to_owned()) }.to_pyobject(vm)
                        })
                        .collect()
                },
                |v, n, vm| {
                    v.as_bytes().py_split_whitespace(n, |s| {
                        unsafe { PyStr::new_ascii_unchecked(s.to_owned()) }.to_pyobject(vm)
                    })
                },
            ),
            PyStrKind::Utf8 => self.as_str().py_split(
                args,
                vm,
                |v, s, vm| v.split(s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, s, n, vm| v.splitn(n, s).map(|s| vm.ctx.new_str(s).into()).collect(),
                |v, n, vm| v.py_split_whitespace(n, |s| vm.ctx.new_str(s).into()),
            ),
        }?;
        Ok(elements)
    }

    #[pymethod]
    fn rsplit(&self, args: SplitArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let mut elements = self.as_str().py_split(
            args,
            vm,
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
    fn strip(&self, chars: OptionalOption<PyStrRef>) -> String {
        self.as_str()
            .py_strip(
                chars,
                |s, chars| s.trim_matches(|c| chars.contains(c)),
                |s| s.trim(),
            )
            .to_owned()
    }

    #[pymethod]
    fn lstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let s = zelf.as_str();
        let stripped = s.py_strip(
            chars,
            |s, chars| s.trim_start_matches(|c| chars.contains(c)),
            |s| s.trim_start(),
        );
        if s == stripped {
            zelf
        } else {
            stripped.into_pystr_ref(vm)
        }
    }

    #[pymethod]
    fn rstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let s = zelf.as_str();
        let stripped = s.py_strip(
            chars,
            |s, chars| s.trim_end_matches(|c| chars.contains(c)),
            |s| s.trim_end(),
        );
        if s == stripped {
            zelf
        } else {
            stripped.into_pystr_ref(vm)
        }
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(self.as_str(), self.len(), |s, r| s.get_chars(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_startsendswith(
            affix,
            "endswith",
            "str",
            |s, x: &PyStrRef| s.ends_with(x.as_str()),
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
            match options.prepare(self.as_str(), self.len(), |s, r| s.get_chars(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_startsendswith(
            affix,
            "startswith",
            "str",
            |s, x: &PyStrRef| s.starts_with(x.as_str()),
            vm,
        )
    }

    /// Return a str with the given prefix string removed if present.
    ///
    /// If the string starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removeprefix(&self, pref: PyStrRef) -> String {
        self.as_str()
            .py_removeprefix(pref.as_str(), pref.byte_len(), |s, p| s.starts_with(p))
            .to_owned()
    }

    /// Return a str with the given suffix string removed if present.
    ///
    /// If the string ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removesuffix(&self, suff: PyStrRef) -> String {
        self.as_str()
            .py_removesuffix(suff.as_str(), suff.byte_len(), |s, p| s.ends_with(p))
            .to_owned()
    }

    #[pymethod]
    fn isalnum(&self) -> bool {
        !self.bytes.is_empty() && self.char_all(char::is_alphanumeric)
    }

    #[pymethod]
    fn isnumeric(&self) -> bool {
        !self.bytes.is_empty() && self.char_all(char::is_numeric)
    }

    #[pymethod]
    fn isdigit(&self) -> bool {
        // python's isdigit also checks if exponents are digits, these are the unicodes for exponents
        let valid_unicodes: [u16; 10] = [
            0x2070, 0x00B9, 0x00B2, 0x00B3, 0x2074, 0x2075, 0x2076, 0x2077, 0x2078, 0x2079,
        ];
        let s = self.as_str();
        !s.is_empty()
            && s.chars()
                .filter(|c| !c.is_ascii_digit())
                .all(|c| valid_unicodes.contains(&(c as u16)))
    }

    #[pymethod]
    fn isdecimal(&self) -> bool {
        !self.bytes.is_empty()
            && self.char_all(|c| GeneralCategory::of(c) == GeneralCategory::DecimalNumber)
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        let formatted = self.as_str().py_cformat(values, vm)?;
        Ok(formatted)
    }

    #[pymethod(magic)]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod]
    fn format(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<String> {
        let format_str = FormatString::from_str(self.as_str()).map_err(|e| e.to_pyexception(vm))?;
        format(&format_str, &args, vm)
    }

    /// S.format_map(mapping) -> str
    ///
    /// Return a formatted version of S, using substitutions from mapping.
    /// The substitutions are identified by braces ('{' and '}').
    #[pymethod]
    fn format_map(&self, mapping: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        let format_string =
            FormatString::from_str(self.as_str()).map_err(|err| err.to_pyexception(vm))?;
        format_map(&format_string, &mapping, vm)
    }

    #[pymethod(name = "__format__")]
    fn format_str(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_string(self.borrow()))
            .map_err(|err| err.into_pyexception(vm))
    }

    /// Return a titlecased version of the string where words start with an
    /// uppercase character and the remaining characters are lowercase.
    #[pymethod]
    fn title(&self) -> String {
        let mut title = String::with_capacity(self.bytes.len());
        let mut previous_is_cased = false;
        for c in self.as_str().chars() {
            if c.is_lowercase() {
                if !previous_is_cased {
                    title.extend(c.to_titlecase());
                } else {
                    title.push(c);
                }
                previous_is_cased = true;
            } else if c.is_uppercase() || c.is_titlecase() {
                if previous_is_cased {
                    title.extend(c.to_lowercase());
                } else {
                    title.push(c);
                }
                previous_is_cased = true;
            } else {
                previous_is_cased = false;
                title.push(c);
            }
        }
        title
    }

    #[pymethod]
    fn swapcase(&self) -> String {
        let mut swapped_str = String::with_capacity(self.bytes.len());
        for c in self.as_str().chars() {
            // to_uppercase returns an iterator, to_ascii_uppercase returns the char
            if c.is_lowercase() {
                swapped_str.push(c.to_ascii_uppercase());
            } else if c.is_uppercase() {
                swapped_str.push(c.to_ascii_lowercase());
            } else {
                swapped_str.push(c);
            }
        }
        swapped_str
    }

    #[pymethod]
    fn isalpha(&self) -> bool {
        !self.bytes.is_empty() && self.char_all(char::is_alphabetic)
    }

    #[pymethod]
    fn replace(&self, old: PyStrRef, new: PyStrRef, count: OptionalArg<isize>) -> String {
        let s = self.as_str();
        match count {
            OptionalArg::Present(maxcount) if maxcount >= 0 => {
                if maxcount == 0 || s.is_empty() {
                    // nothing to do; return the original bytes
                    s.into()
                } else {
                    s.replacen(old.as_str(), new.as_str(), maxcount as usize)
                }
            }
            _ => s.replace(old.as_str(), new.as_str()),
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
        self.char_all(|c| c == '\u{0020}' || rustpython_common::char::is_printable(c))
    }

    #[pymethod]
    fn isspace(&self) -> bool {
        use unic_ucd_bidi::bidi_class::abbr_names::*;
        !self.bytes.is_empty()
            && self.char_all(|c| {
                GeneralCategory::of(c) == GeneralCategory::SpaceSeparator
                    || matches!(BidiClass::of(c), WS | B | S)
            })
    }

    // Return true if all cased characters in the string are lowercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        match self.kind.kind() {
            PyStrKind::Ascii => self.bytes.py_iscase(char::is_lowercase, char::is_uppercase),
            PyStrKind::Utf8 => self
                .as_str()
                .py_iscase(char::is_lowercase, char::is_uppercase),
        }
    }

    // Return true if all cased characters in the string are uppercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        match self.kind.kind() {
            PyStrKind::Ascii => self.bytes.py_iscase(char::is_uppercase, char::is_lowercase),
            PyStrKind::Utf8 => self
                .as_str()
                .py_iscase(char::is_uppercase, char::is_lowercase),
        }
    }

    #[pymethod]
    fn splitlines(&self, args: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        self.as_str()
            .py_splitlines(args, |s| self.new_substr(s.to_owned()).to_pyobject(vm))
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
                    first.as_str().to_owned()
                }
            }
            Err(iter) => zelf.as_str().py_join(iter)?,
        };
        Ok(joined.into_pystr_ref(vm))
    }

    // FIXME: two traversals of str is expensive
    #[inline]
    fn _to_char_idx(r: &str, byte_idx: usize) -> usize {
        r[..byte_idx].chars().count()
    }

    #[inline]
    fn _find<F>(&self, args: FindArgs, find: F) -> Option<usize>
    where
        F: Fn(&str, &str) -> Option<usize>,
    {
        let (sub, range) = args.get_value(self.len());
        self.as_str().py_find(sub.as_str(), range, find)
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
            .ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn rindex(&self, args: FindArgs, vm: &VirtualMachine) -> PyResult<usize> {
        self._find(args, |r, s| Some(Self::_to_char_idx(r, r.rfind(s)?)))
            .ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn partition(&self, sep: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (front, has_mid, back) = self.as_str().py_partition(
            sep.as_str(),
            || self.as_str().splitn(2, sep.as_str()),
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
        let (back, has_mid, front) = self.as_str().py_partition(
            sep.as_str(),
            || self.as_str().rsplitn(2, sep.as_str()),
            vm,
        )?;
        Ok((
            self.new_substr(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_str(ascii!(""))
            },
            self.new_substr(back),
        )
            .to_pyobject(vm))
    }

    /// Return `true` if the sequence is ASCII titlecase and the sequence is not
    /// empty, `false` otherwise.
    #[pymethod]
    fn istitle(&self) -> bool {
        if self.bytes.is_empty() {
            return false;
        }

        let mut cased = false;
        let mut previous_is_cased = false;
        for c in self.as_str().chars() {
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
        self.as_str()
            .py_count(needle.as_str(), range, |h, n| h.matches(n).count())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> String {
        unsafe {
            // SAFETY: this is safe-guaranteed because the original self.as_str() is valid utf8
            String::from_utf8_unchecked(self.as_str().py_zfill(width))
        }
    }

    #[inline]
    fn _pad(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        pad: fn(&str, usize, char, usize) -> String,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let fillchar = fillchar.map_or(Ok(' '), |ref s| {
            s.as_str().chars().exactly_one().map_err(|_| {
                vm.new_type_error(
                    "The fill character must be exactly one character long".to_owned(),
                )
            })
        })?;
        Ok(if self.len() as isize >= width {
            String::from(self.as_str())
        } else {
            pad(self.as_str(), width as usize, fillchar, self.len())
        })
    }

    #[pymethod]
    fn center(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self._pad(width, fillchar, AnyStr::py_center, vm)
    }

    #[pymethod]
    fn ljust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self._pad(width, fillchar, AnyStr::py_ljust, vm)
    }

    #[pymethod]
    fn rjust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self._pad(width, fillchar, AnyStr::py_rjust, vm)
    }

    #[pymethod]
    fn expandtabs(&self, args: anystr::ExpandTabsArgs) -> String {
        let tab_stop = args.tabsize();
        let mut expanded_str = String::with_capacity(self.byte_len());
        let mut tab_size = tab_stop;
        let mut col_count = 0usize;
        for ch in self.as_str().chars() {
            match ch {
                '\t' => {
                    let num_spaces = tab_size - col_count;
                    col_count += num_spaces;
                    let expand = " ".repeat(num_spaces);
                    expanded_str.push_str(&expand);
                }
                '\r' | '\n' => {
                    expanded_str.push(ch);
                    col_count = 0;
                    tab_size = 0;
                }
                _ => {
                    expanded_str.push(ch);
                    col_count += 1;
                }
            }
            if col_count >= tab_size {
                tab_size += tab_stop;
            }
        }
        expanded_str
    }

    #[pymethod]
    fn isidentifier(&self) -> bool {
        let mut chars = self.as_str().chars();
        let is_identifier_start = chars.next().map_or(false, |c| c == '_' || is_xid_start(c));
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
                    if let Some(text) = value.payload::<PyStr>() {
                        translated.push_str(text.as_str());
                    } else if let Some(bigint) = value.payload::<PyInt>() {
                        let ch = bigint
                            .as_bigint()
                            .to_u32()
                            .and_then(std::char::from_u32)
                            .ok_or_else(|| {
                                vm.new_value_error(
                                    "character mapping must be in range(0x110000)".to_owned(),
                                )
                            })?;
                        translated.push(ch);
                    } else if !vm.is_none(&value) {
                        return Err(vm.new_type_error(
                            "character mapping must return integer, None or str".to_owned(),
                        ));
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
            match dict_or_str.downcast::<PyStr>() {
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
                            "the first two maketrans arguments must have equal length".to_owned(),
                        ))
                    }
                }
                _ => Err(vm.new_type_error(
                    "first maketrans argument must be a string if there is a second argument"
                        .to_owned(),
                )),
            }
        } else {
            // dict_str must be a dict
            match dict_or_str.downcast::<PyDict>() {
                Ok(dict) => {
                    for (key, val) in dict {
                        // FIXME: ints are key-compatible
                        if let Some(num) = key.payload::<PyInt>() {
                            new_dict.set_item(
                                &*num.as_bigint().to_i32().to_pyobject(vm),
                                val,
                                vm,
                            )?;
                        } else if let Some(string) = key.payload::<PyStr>() {
                            if string.len() == 1 {
                                let num_value = string.as_str().chars().next().unwrap() as u32;
                                new_dict.set_item(&*num_value.to_pyobject(vm), val, vm)?;
                            } else {
                                return Err(vm.new_value_error(
                                    "string keys in translate table must be of length 1".to_owned(),
                                ));
                            }
                        }
                    }
                    Ok(new_dict.to_pyobject(vm))
                }
                _ => Err(vm.new_value_error(
                    "if you give only one argument to maketrans it must be a dict".to_owned(),
                )),
            }
        }
    }

    #[pymethod]
    fn encode(zelf: PyRef<Self>, args: EncodeArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        encode_string(zelf, args.encoding, args.errors, vm)
    }

    #[pymethod(magic)]
    fn getnewargs(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
        (zelf.as_str(),).to_pyobject(vm)
    }
}

impl PyStrRef {
    pub fn concat_in_place(&mut self, other: &str, vm: &VirtualMachine) {
        // TODO: call [A]Rc::get_mut on the str to try to mutate the data in place
        if other.is_empty() {
            return;
        }
        let mut s = String::with_capacity(self.byte_len() + other.len());
        s.push_str(self.as_ref());
        s.push_str(other);
        *self = PyStr::from(s).into_ref(vm);
    }
}

impl Hashable for PyStr {
    #[inline]
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.hash(vm))
    }
}

impl Comparable for PyStr {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        let other = class_or_notimplemented!(Self, other);
        Ok(op.eval_ord(zelf.as_str().cmp(other.as_str())).into())
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
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyStr::mapping_downcast(mapping).len())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyStr::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyStr {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyStr::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                PyStr::add(zelf.to_owned(), other.to_owned(), vm)
            }),
            repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                PyStr::mul(zelf.to_owned(), n as isize, vm).map(|x| x.into())
            }),
            item: atomic_func!(|seq, i, vm| {
                let zelf = PyStr::sequence_downcast(seq);
                zelf.getitem_by_index(vm, i)
                    .map(|x| zelf.new_substr(x.to_string()).into_ref(vm).into())
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
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.str_type
    }
}

impl ToPyObject for String {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }
}

impl ToPyObject for char {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self.to_string()).into()
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

type SplitArgs<'a> = anystr::SplitArgs<'a, PyStrRef>;

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

pub fn init(ctx: &Context) {
    PyStr::extend_class(ctx, ctx.types.str_type);

    PyStrIterator::extend_class(ctx, ctx.types.str_iterator_type);
}

impl SliceableSequenceOp for PyStr {
    type Item = char;
    type Sliced = String;

    fn do_get(&self, index: usize) -> Self::Item {
        if self.is_ascii() {
            self.bytes[index] as char
        } else {
            self.as_str().chars().nth(index).unwrap()
        }
    }

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        let value = self.as_str();
        if self.is_ascii() {
            value[range].to_owned()
        } else {
            rustpython_common::str::get_chars(value, range).to_owned()
        }
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        if self.is_ascii() {
            // this is an ascii string
            let mut v = self.bytes[range].to_vec();
            v.reverse();
            unsafe {
                // SAFETY: an ascii string is always utf8
                String::from_utf8_unchecked(v)
            }
        } else {
            let mut s = String::with_capacity(self.bytes.len());
            s.extend(
                self.as_str()
                    .chars()
                    .rev()
                    .skip(self.char_len() - range.end)
                    .take(range.end - range.start),
            );
            s
        }
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        if self.is_ascii() {
            let v = self.bytes[range].iter().copied().step_by(step).collect();
            unsafe {
                // SAFETY: Any subset of ascii string is a valid utf8 string
                String::from_utf8_unchecked(v)
            }
        } else {
            let mut s = String::with_capacity(2 * ((range.len() / step) + 1));
            s.extend(
                self.as_str()
                    .chars()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .step_by(step),
            );
            s
        }
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        if self.is_ascii() {
            // this is an ascii string
            let v: Vec<u8> = self.bytes[range]
                .iter()
                .rev()
                .copied()
                .step_by(step)
                .collect();
            // TODO: from_utf8_unchecked?
            String::from_utf8(v).unwrap()
        } else {
            // not ascii, so the codepoints have to be at least 2 bytes each
            let mut s = String::with_capacity(2 * ((range.len() / step) + 1));
            s.extend(
                self.as_str()
                    .chars()
                    .rev()
                    .skip(self.char_len() - range.end)
                    .take(range.end - range.start)
                    .step_by(step),
            );
            s
        }
    }

    fn empty() -> Self::Sliced {
        String::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;
    use std::ops::Deref;

    #[test]
    fn str_title() {
        let tests = vec![
            (" Hello ", " hello "),
            ("Hello ", "hello "),
            ("Hello ", "Hello "),
            ("Format This As Title String", "fOrMaT thIs aS titLe String"),
            ("Format,This-As*Title;String", "fOrMaT,thIs-aS*titLe;String"),
            ("Getint", "getInt"),
            ("Greek Ωppercases ...", "greek ωppercases ..."),
            ("Greek ῼitlecases ...", "greek ῳitlecases ..."),
        ];
        for (title, input) in tests {
            assert_eq!(PyStr::from(input).title().as_str(), title);
        }
    }

    #[test]
    fn str_istitle() {
        let pos = vec![
            "A",
            "A Titlecased Line",
            "A\nTitlecased Line",
            "A Titlecased, Line",
            "Greek Ωppercases ...",
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
            assert_eq!(
                translated.unwrap_err().class().name().deref(),
                "TypeError".to_owned()
            );
        })
    }
}

impl<'s> AnyStrWrapper<'s> for PyStrRef {
    type Str = str;
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AnyStrContainer<str> for String {
    fn new() -> Self {
        String::new()
    }

    fn with_capacity(capacity: usize) -> Self {
        String::with_capacity(capacity)
    }

    fn push_str(&mut self, other: &str) {
        String::push_str(self, other)
    }
}

impl<'s> AnyStr<'s> for str {
    type Char = char;
    type Container = String;
    type CharIter = std::str::Chars<'s>;
    type ElementIter = std::str::Chars<'s>;

    fn element_bytes_len(c: char) -> usize {
        c.len_utf8()
    }

    fn to_container(&self) -> Self::Container {
        self.to_owned()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn as_utf8_str(&self) -> Result<&str, std::str::Utf8Error> {
        Ok(self)
    }

    fn chars(&'s self) -> Self::CharIter {
        str::chars(self)
    }

    fn elements(&'s self) -> Self::ElementIter {
        str::chars(self)
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
        let mut splited = Vec::new();
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
            splited.push(convert(&self[last_offset..offset]));
            last_offset = offset + 1;
            count -= 1;
        }
        if last_offset != self.len() {
            splited.push(convert(&self[last_offset..]));
        }
        splited
    }

    fn py_rsplit_whitespace<F>(&self, maxsplit: isize, convert: F) -> Vec<PyObjectRef>
    where
        F: Fn(&Self) -> PyObjectRef,
    {
        // CPython rsplit_whitespace
        let mut splited = Vec::new();
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
            splited.push(convert(&self[offset + 1..last_offset]));
            last_offset = offset;
            count -= 1;
        }
        if last_offset != 0 {
            splited.push(convert(&self[..last_offset]));
        }
        splited
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
        std::fmt::Display::fmt(self.as_str(), f)
    }
}

impl AsRef<str> for PyStrInterned {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

use std::mem::size_of;
use std::ops::Range;
use std::string::ToString;
use std::{char, ffi, fmt};

use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use num_traits::ToPrimitive;
use unic_ucd_bidi::BidiClass;
use unic_ucd_category::GeneralCategory;
use unic_ucd_ident::{is_xid_continue, is_xid_start};
use unicode_casing::CharExt;

use super::bytes::PyBytesRef;
use super::dict::PyDict;
use super::int::{try_to_primitive, PyInt, PyIntRef};
use super::iter::{
    IterStatus,
    IterStatus::{Active, Exhausted},
};
use super::pytype::PyTypeRef;
use crate::anystr::{self, adjust_indices, AnyStr, AnyStrContainer, AnyStrWrapper};
use crate::exceptions::IntoPyException;
use crate::format::{FormatSpec, FormatString, FromTemplate};
use crate::function::{FuncArgs, OptionalArg, OptionalOption};
use crate::sliceable::PySliceableSequence;
use crate::slots::{Comparable, Hashable, Iterable, PyComparisonOp, PyIter, SlotConstructor};
use crate::utils::Either;
use crate::VirtualMachine;
use crate::{
    IdProtocol, IntoPyObject, ItemProtocol, PyClassDef, PyClassImpl, PyComparisonValue, PyContext,
    PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryIntoRef, TypeProtocol,
};
use rustpython_common::atomic::{self, PyAtomic, Radium};
use rustpython_common::hash;

/// str(object='') -> str
/// str(bytes_or_buffer[, encoding[, errors]]) -> str
///
/// Create a new string object from the given object. If encoding or
/// errors is specified, then the object must expose a data buffer
/// that will be decoded using the given encoding and error handler.
/// Otherwise, returns the result of object.__str__() (if defined)
/// or repr(object).
/// encoding defaults to sys.getdefaultencoding().
/// errors defaults to 'strict'."
#[pyclass(module = false, name = "str")]
#[derive(Debug)]
pub struct PyStr {
    value: Box<str>,
    hash: PyAtomic<hash::PyHash>,
    // uses usize::MAX as a sentinel for "uncomputed"
    char_len: PyAtomic<usize>,
}

impl AsRef<str> for PyStr {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl<T> From<&T> for PyStr
where
    T: AsRef<str> + ?Sized,
{
    fn from(s: &T) -> PyStr {
        s.as_ref().to_owned().into()
    }
}
impl AsRef<str> for PyStrRef {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl From<String> for PyStr {
    fn from(s: String) -> PyStr {
        s.into_boxed_str().into()
    }
}

impl From<Box<str>> for PyStr {
    #[inline]
    fn from(value: Box<str>) -> PyStr {
        PyStr {
            value,
            hash: Radium::new(hash::SENTINEL),
            char_len: Radium::new(usize::MAX),
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

impl TryIntoRef<PyStr> for String {
    #[inline]
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        Ok(PyStr::from(self).into_ref(vm))
    }
}

impl TryIntoRef<PyStr> for &str {
    #[inline]
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        Ok(PyStr::from(self).into_ref(vm))
    }
}

#[pyclass(module = false, name = "str_iterator")]
#[derive(Debug)]
pub struct PyStrIterator {
    string: PyStrRef,
    position: PyAtomic<usize>,
    status: AtomicCell<IterStatus>,
}

impl PyValue for PyStrIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.str_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyStrIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        match self.status.load() {
            Active => {
                let pos = self.position.load(atomic::Ordering::SeqCst);
                self.string.len().saturating_sub(pos)
            }
            Exhausted => 0,
        }
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // When we're exhausted, just return.
        if let Exhausted = self.status.load() {
            return Ok(());
        }
        let pos = state
            .payload::<PyInt>()
            .ok_or_else(|| vm.new_type_error("an integer is required.".to_owned()))?;
        let pos = std::cmp::min(
            try_to_primitive(pos.as_bigint(), vm).unwrap_or(0),
            self.string.len(),
        );
        self.position.store(pos, atomic::Ordering::SeqCst);
        Ok(())
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyResult {
        let iter = vm.get_attribute(vm.builtins.clone(), "iter")?;
        Ok(vm.ctx.new_tuple(match self.status.load() {
            Exhausted => vec![iter, vm.ctx.new_tuple(vec![vm.ctx.new_str("")])],
            Active => vec![
                iter,
                vm.ctx.new_tuple(vec![self.string.clone().into_object()]),
                vm.ctx
                    .new_int(self.position.load(atomic::Ordering::Relaxed)),
            ],
        }))
    }
}

impl PyIter for PyStrIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        if let Exhausted = zelf.status.load() {
            return Err(vm.new_stop_iteration());
        }
        let value = &*zelf.string.value;
        let mut start = zelf.position.load(atomic::Ordering::SeqCst);
        loop {
            if start == value.len() {
                zelf.status.store(Exhausted);
                return Err(vm.new_stop_iteration());
            }
            let ch = value[start..].chars().next().ok_or_else(|| {
                zelf.status.store(Exhausted);
                vm.new_stop_iteration()
            })?;

            match zelf.position.compare_exchange_weak(
                start,
                start + ch.len_utf8(),
                atomic::Ordering::Release,
                atomic::Ordering::Relaxed,
            ) {
                Ok(_) => break Ok(ch.into_pyobject(vm)),
                Err(cur) => start = cur,
            }
        }
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

impl SlotConstructor for PyStr {
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
                    vm.to_str(&input)?
                }
            }
            OptionalArg::Missing => {
                PyStr::from(String::new()).into_ref_with_type(vm, cls.clone())?
            }
        };
        if string.class().is(&cls) {
            Ok(string.into_object())
        } else {
            PyStr::from(string.as_str()).into_pyresult_with_type(vm, cls)
        }
    }
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, Iterable, SlotConstructor))]
impl PyStr {
    #[pymethod(magic)]
    fn add(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(other) = other.payload::<PyStr>() {
            Ok(vm.ctx.new_str(zelf.value.py_add(other.as_ref())))
        } else if let Some(radd) = vm.get_method(other.clone(), "__radd__") {
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
        !self.value.is_empty()
    }

    #[pymethod(magic)]
    fn contains(&self, needle: PyStrRef) -> bool {
        self.value.contains(needle.as_str())
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let s = match self.get_item(vm, needle, Self::NAME)? {
            Either::A(ch) => ch.to_string(),
            Either::B(s) => s,
        };
        Ok(vm.ctx.new_str(s))
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

    pub fn as_str(&self) -> &str {
        &self.value
    }

    #[inline]
    pub fn byte_len(&self) -> usize {
        self.value.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    #[pymethod(name = "__len__")]
    #[inline]
    pub fn char_len(&self) -> usize {
        match self.char_len.load(atomic::Ordering::Relaxed) {
            usize::MAX => self._compute_char_len(),
            len => len,
        }
    }
    #[cold]
    fn _compute_char_len(&self) -> usize {
        // doing the check is ~10x faster for ascii, and is actually only 2% slower worst case for
        // non-ascii; see https://github.com/RustPython/RustPython/pull/2586#issuecomment-844611532
        let len = if self.value.is_ascii() {
            self.value.len()
        } else {
            self.value.chars().count()
        };
        // len cannot be usize::MAX, since vec.capacity() < isize::MAX
        self.char_len.store(len, atomic::Ordering::Relaxed);
        len
    }

    #[pymethod(name = "isascii")]
    #[inline(always)]
    pub fn is_ascii(&self) -> bool {
        self.char_len() == self.byte_len()
    }

    pub fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<ffi::CString> {
        ffi::CString::new(self.as_str()).map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.as_str().len() * size_of::<u8>()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if value == 1 && zelf.class().is(&vm.ctx.types.str_type) {
            // Special case: when some `str` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `str` itself, not its subclasses.
            return Ok(zelf);
        }
        // todo: map err to overflow.
        vm.check_repeat_or_memory_error(zelf.len(), value)
            .map(|value| Self::from(zelf.value.repeat(value)).into_ref(vm))
            // see issue 45044 on b.p.o.
            .map_err(|_| vm.new_overflow_error("repeated bytes are too long".to_owned()))
    }

    #[pymethod(magic)]
    fn str(zelf: PyRef<Self>) -> PyStrRef {
        zelf
    }

    #[pymethod(magic)]
    pub(crate) fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let in_len = self.value.len();
        let mut out_len = 0usize;
        // let mut max = 127;
        let mut squote = 0;
        let mut dquote = 0;

        for ch in self.value.chars() {
            let incr = match ch {
                '\'' => {
                    squote += 1;
                    1
                }
                '"' => {
                    dquote += 1;
                    1
                }
                '\\' | '\t' | '\r' | '\n' => 2,
                ch if ch < ' ' || ch as u32 == 0x7f => 4, // \xHH
                ch if ch.is_ascii() => 1,
                ch if char_is_printable(ch) => {
                    // max = std::cmp::max(ch, max);
                    ch.len_utf8()
                }
                ch if (ch as u32) < 0x100 => 4,   // \xHH
                ch if (ch as u32) < 0x10000 => 6, // \uHHHH
                _ => 10,                          // \uHHHHHHHH
            };
            if out_len > (isize::MAX as usize) - incr {
                return Err(vm.new_overflow_error("string is too long to generate repr".to_owned()));
            }
            out_len += incr;
        }

        let (quote, num_escaped_quotes) = anystr::choose_quotes_for_repr(squote, dquote);
        // we'll be adding backslashes in front of the existing inner quotes
        out_len += num_escaped_quotes;

        // if we don't need to escape anything we can just copy
        let unchanged = out_len == in_len;

        // start and ending quotes
        out_len += 2;

        let mut repr = String::with_capacity(out_len);
        repr.push(quote);
        if unchanged {
            repr.push_str(self.as_str());
        } else {
            for ch in self.value.chars() {
                use std::fmt::Write;
                match ch {
                    '\n' => repr.push_str("\\n"),
                    '\t' => repr.push_str("\\t"),
                    '\r' => repr.push_str("\\r"),
                    // these 2 branches *would* be handled below, but we shouldn't have to do a
                    // unicodedata lookup just for ascii characters
                    '\x20'..='\x7e' => {
                        // printable ascii range
                        if ch == quote || ch == '\\' {
                            repr.push('\\');
                        }
                        repr.push(ch);
                    }
                    ch if ch.is_ascii() => {
                        write!(repr, "\\x{:02x}", ch as u8).unwrap();
                    }
                    ch if char_is_printable(ch) => {
                        repr.push(ch);
                    }
                    '\0'..='\u{ff}' => {
                        write!(repr, "\\x{:02x}", ch as u32).unwrap();
                    }
                    '\0'..='\u{ffff}' => {
                        write!(repr, "\\u{:04x}", ch as u32).unwrap();
                    }
                    _ => {
                        write!(repr, "\\U{:08x}", ch as u32).unwrap();
                    }
                }
            }
        }
        repr.push(quote);

        Ok(repr)
    }

    #[pymethod]
    fn lower(&self) -> String {
        self.value.to_lowercase()
    }

    // casefold is much more aggressive than lower
    #[pymethod]
    fn casefold(&self) -> String {
        caseless::default_case_fold_str(self.as_str())
    }

    #[pymethod]
    fn upper(&self) -> String {
        self.value.to_uppercase()
    }

    #[pymethod]
    fn capitalize(&self) -> String {
        let mut chars = self.value.chars();
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
    fn split(&self, args: SplitArgs, vm: &VirtualMachine) -> PyResult {
        let elements = self.value.py_split(
            args,
            vm,
            |v, s, vm| v.split(s).map(|s| vm.ctx.new_str(s)).collect(),
            |v, s, n, vm| v.splitn(n, s).map(|s| vm.ctx.new_str(s)).collect(),
            |v, n, vm| v.py_split_whitespace(n, |s| vm.ctx.new_str(s)),
        )?;
        Ok(vm.ctx.new_list(elements))
    }

    #[pymethod]
    fn rsplit(&self, args: SplitArgs, vm: &VirtualMachine) -> PyResult {
        let mut elements = self.value.py_split(
            args,
            vm,
            |v, s, vm| v.rsplit(s).map(|s| vm.ctx.new_str(s)).collect(),
            |v, s, n, vm| v.rsplitn(n, s).map(|s| vm.ctx.new_str(s)).collect(),
            |v, n, vm| v.py_rsplit_whitespace(n, |s| vm.ctx.new_str(s)),
        )?;
        // Unlike Python rsplit, Rust rsplitn returns an iterator that
        // starts from the end of the string.
        elements.reverse();
        Ok(vm.ctx.new_list(elements))
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyStrRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_matches(|c| chars.contains(c)),
                |s| s.trim(),
            )
            .to_owned()
    }

    #[pymethod]
    fn lstrip(&self, chars: OptionalOption<PyStrRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_start_matches(|c| chars.contains(c)),
                |s| s.trim_start(),
            )
            .to_owned()
    }

    #[pymethod]
    fn rstrip(&self, chars: OptionalOption<PyStrRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_end_matches(|c| chars.contains(c)),
                |s| s.trim_end(),
            )
            .to_owned()
    }

    #[pymethod]
    fn endswith(&self, args: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.value.py_startsendswith(
            args,
            "endswith",
            "str",
            |s, x: &PyStrRef| s.ends_with(x.as_str()),
            vm,
        )
    }

    #[pymethod]
    fn startswith(&self, args: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.value.py_startsendswith(
            args,
            "startswith",
            "str",
            |s, x: &PyStrRef| s.starts_with(x.as_str()),
            vm,
        )
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a str with the given prefix string removed if present.
    ///
    /// If the string starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removeprefix(&self, pref: PyStrRef) -> String {
        self.value
            .py_removeprefix(pref.as_str(), pref.value.len(), |s, p| s.starts_with(p))
            .to_owned()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a str with the given suffix string removed if present.
    ///
    /// If the string ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original string.
    #[pymethod]
    fn removesuffix(&self, suff: PyStrRef) -> String {
        self.value
            .py_removesuffix(suff.as_str(), suff.value.len(), |s, p| s.ends_with(p))
            .to_owned()
    }

    #[pymethod]
    fn isalnum(&self) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_alphanumeric)
    }

    #[pymethod]
    fn isnumeric(&self) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_numeric)
    }

    #[pymethod]
    fn isdigit(&self) -> bool {
        // python's isdigit also checks if exponents are digits, these are the unicodes for exponents
        let valid_unicodes: [u16; 10] = [
            0x2070, 0x00B9, 0x00B2, 0x00B3, 0x2074, 0x2075, 0x2076, 0x2077, 0x2078, 0x2079,
        ];

        !self.value.is_empty()
            && self
                .value
                .chars()
                .filter(|c| !c.is_digit(10))
                .all(|c| valid_unicodes.contains(&(c as u16)))
    }

    #[pymethod]
    fn isdecimal(&self) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii_digit())
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        let formatted = self.value.py_cformat(values, vm)?;
        Ok(formatted)
    }

    #[pymethod(magic)]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod]
    fn format(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<String> {
        match FormatString::from_str(self.as_str()) {
            Ok(format_string) => format_string.format(&args, vm),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    /// S.format_map(mapping) -> str
    ///
    /// Return a formatted version of S, using substitutions from mapping.
    /// The substitutions are identified by braces ('{' and '}').
    #[pymethod]
    fn format_map(&self, mapping: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatString::from_str(self.as_str()) {
            Ok(format_string) => format_string.format_map(&mapping, vm),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pymethod(name = "__format__")]
    fn format_str(&self, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_string(self.as_str()))
        {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    /// Return a titlecased version of the string where words start with an
    /// uppercase character and the remaining characters are lowercase.
    #[pymethod]
    fn title(&self) -> String {
        let mut title = String::with_capacity(self.value.len());
        let mut previous_is_cased = false;
        for c in self.value.chars() {
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
        let mut swapped_str = String::with_capacity(self.value.len());
        for c in self.value.chars() {
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
        !self.value.is_empty() && self.value.chars().all(char::is_alphabetic)
    }

    #[pymethod]
    fn replace(&self, old: PyStrRef, new: PyStrRef, count: OptionalArg<isize>) -> String {
        match count {
            OptionalArg::Present(maxcount) if maxcount >= 0 => {
                if maxcount == 0 || self.value.is_empty() {
                    // nothing to do; return the original bytes
                    return String::from(self.as_str());
                }
                self.value
                    .replacen(old.as_str(), new.as_str(), maxcount as usize)
            }
            _ => self.value.replace(old.as_str(), new.as_str()),
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
        self.value
            .chars()
            .all(|c| c == '\u{0020}' || char_is_printable(c))
    }

    #[pymethod]
    fn isspace(&self) -> bool {
        if self.value.is_empty() {
            return false;
        }
        use unic_ucd_bidi::bidi_class::abbr_names::*;
        self.value.chars().all(|c| {
            GeneralCategory::of(c) == GeneralCategory::SpaceSeparator
                || matches!(BidiClass::of(c), WS | B | S)
        })
    }

    // Return true if all cased characters in the string are lowercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        self.value.py_iscase(char::is_lowercase, char::is_uppercase)
    }

    // Return true if all cased characters in the string are uppercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        self.value.py_iscase(char::is_uppercase, char::is_lowercase)
    }

    #[pymethod]
    fn splitlines(&self, args: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_list(self.value.py_splitlines(args, |s| vm.ctx.new_str(s)))
    }

    #[pymethod]
    fn join(&self, iterable: PyIterable<PyStrRef>, vm: &VirtualMachine) -> PyResult<String> {
        let iter = iterable.iter(vm)?;
        self.value.py_join(iter)
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
        self.value.py_find(sub.as_str(), range, find)
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
        let (front, has_mid, back) =
            self.value
                .py_partition(sep.as_str(), || self.value.splitn(2, sep.as_str()), vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_str(front),
            if has_mid {
                sep.into_object()
            } else {
                vm.ctx.new_str("")
            },
            vm.ctx.new_str(back),
        ]))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (back, has_mid, front) =
            self.value
                .py_partition(sep.as_str(), || self.value.rsplitn(2, sep.as_str()), vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_str(front),
            if has_mid {
                sep.into_object()
            } else {
                vm.ctx.new_str("")
            },
            vm.ctx.new_str(back),
        ]))
    }

    /// Return `true` if the sequence is ASCII titlecase and the sequence is not
    /// empty, `false` otherwise.
    #[pymethod]
    fn istitle(&self) -> bool {
        if self.value.is_empty() {
            return false;
        }

        let mut cased = false;
        let mut previous_is_cased = false;
        for c in self.value.chars() {
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
        self.value
            .py_count(needle.as_str(), range, |h, n| h.matches(n).count())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> String {
        // this is safe-guaranteed because the original self.value is valid utf8
        unsafe { String::from_utf8_unchecked(self.value.py_zfill(width)) }
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
            s.value.chars().exactly_one().map_err(|_| {
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
        let mut expanded_str = String::with_capacity(self.value.len());
        let mut tab_size = tab_stop;
        let mut col_count = 0usize;
        for ch in self.value.chars() {
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
        let mut chars = self.value.chars();
        let is_identifier_start = chars.next().map_or(false, |c| c == '_' || is_xid_start(c));
        // a string is not an identifier if it has whitespace or starts with a number
        is_identifier_start && chars.all(is_xid_continue)
    }

    // https://docs.python.org/3/library/stdtypes.html#str.translate
    #[pymethod]
    fn translate(&self, table: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        vm.get_method_or_type_error(table.clone(), "__getitem__", || {
            format!("'{}' object is not subscriptable", table.class().name())
        })?;

        let mut translated = String::new();
        for c in self.value.chars() {
            match table.get_item((c as u32).into_pyobject(vm), vm) {
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
                        translated.push(ch as char);
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

    #[pymethod]
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
                        for (c1, c2) in from_str.value.chars().zip(to_str.value.chars()) {
                            new_dict.set_item(
                                vm.ctx.new_int(c1 as u32),
                                vm.ctx.new_int(c2 as u32),
                                vm,
                            )?;
                        }
                        if let OptionalArg::Present(none_str) = none_str {
                            for c in none_str.value.chars() {
                                new_dict.set_item(vm.ctx.new_int(c as u32), vm.ctx.none(), vm)?;
                            }
                        }
                        Ok(new_dict.into_pyobject(vm))
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
                        if let Some(num) = key.payload::<PyInt>() {
                            new_dict.set_item(
                                num.as_bigint().to_i32().into_pyobject(vm),
                                val,
                                vm,
                            )?;
                        } else if let Some(string) = key.payload::<PyStr>() {
                            if string.len() == 1 {
                                let num_value = string.value.chars().next().unwrap() as u32;
                                new_dict.set_item(num_value.into_pyobject(vm), val, vm)?;
                            } else {
                                return Err(vm.new_value_error(
                                    "string keys in translate table must be of length 1".to_owned(),
                                ));
                            }
                        }
                    }
                    Ok(new_dict.into_pyobject(vm))
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
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.hash(vm))
    }
}

impl Comparable for PyStr {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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
            position: Radium::new(0),
            string: zelf,
            status: AtomicCell::new(Active),
        }
        .into_object(vm))
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

impl PyValue for PyStr {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.str_type
    }
}

impl IntoPyObject for String {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self)
    }
}

impl IntoPyObject for char {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self.to_string())
    }
}

impl IntoPyObject for &str {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self)
    }
}

impl IntoPyObject for &String {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self.clone())
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

pub fn init(ctx: &PyContext) {
    PyStr::extend_class(ctx, &ctx.types.str_type);

    PyStrIterator::extend_class(ctx, &ctx.types.str_iterator_type);
}

impl PySliceableSequence for PyStr {
    type Item = char;
    type Sliced = String;

    fn do_get(&self, index: usize) -> Self::Item {
        if self.is_ascii() {
            self.value.as_bytes()[index] as char
        } else {
            self.value.chars().nth(index).unwrap()
        }
    }

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        let value = &*self.value;
        if self.is_ascii() {
            value[range].to_owned()
        } else {
            rustpython_common::str::get_chars(value, range).to_owned()
        }
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        let value = &*self.value;
        let char_len = self.char_len();
        if char_len == self.byte_len() {
            // this is an ascii string
            let mut v = value.as_bytes()[range].to_vec();
            v.reverse();
            // TODO: from_utf8_unchecked?
            String::from_utf8(v).unwrap()
        } else {
            let mut s = String::with_capacity(value.len());
            s.extend(
                value
                    .chars()
                    .rev()
                    .skip(char_len - range.end)
                    .take(range.end - range.start),
            );
            s
        }
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        let value = &*self.value;
        if self.is_ascii() {
            let v = value.as_bytes()[range]
                .iter()
                .copied()
                .step_by(step)
                .collect();
            // TODO: from_utf8_unchecked?
            String::from_utf8(v).unwrap()
        } else {
            let mut s = String::with_capacity(2 * ((range.len() / step) + 1));
            s.extend(
                value
                    .chars()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .step_by(step),
            );
            s
        }
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        let value = &*self.value;
        let char_len = self.char_len();
        if char_len == self.byte_len() {
            // this is an ascii string
            let v: Vec<u8> = value.as_bytes()[range]
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
                value
                    .chars()
                    .rev()
                    .skip(char_len - range.end)
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

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

// According to python following categories aren't printable:
// * Cc (Other, Control)
// * Cf (Other, Format)
// * Cs (Other, Surrogate)
// * Co (Other, Private Use)
// * Cn (Other, Not Assigned)
// * Zl Separator, Line ('\u2028', LINE SEPARATOR)
// * Zp Separator, Paragraph ('\u2029', PARAGRAPH SEPARATOR)
// * Zs (Separator, Space) other than ASCII space('\x20').
fn char_is_printable(c: char) -> bool {
    let cat = GeneralCategory::of(c);
    !(cat.is_other() || cat.is_separator())
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
        Interpreter::default().enter(|vm| {
            let table = vm.ctx.new_dict();
            table.set_item("a", vm.ctx.new_str("🎅"), &vm).unwrap();
            table.set_item("b", vm.ctx.none(), &vm).unwrap();
            table.set_item("c", vm.ctx.new_str("xda"), &vm).unwrap();
            let translated = PyStr::maketrans(
                table.into_object(),
                OptionalArg::Missing,
                OptionalArg::Missing,
                &vm,
            )
            .unwrap();
            let text = PyStr::from("abc");
            let translated = text.translate(translated, &vm).unwrap();
            assert_eq!(translated, "🎅xda".to_owned());
            let translated = text.translate(vm.ctx.new_int(3), &vm);
            assert_eq!(
                translated.unwrap_err().class().name(),
                "TypeError".to_owned()
            );
        })
    }
}

impl<'s> AnyStrWrapper<'s> for PyStrRef {
    type Str = str;
    fn as_ref(&self) -> &str {
        &*self.value
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

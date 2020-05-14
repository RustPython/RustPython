use std::char;
use std::fmt;
use std::mem::size_of;
use std::ops::Range;
use std::str::FromStr;
use std::string::ToString;

use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use unic_ucd_category::GeneralCategory;
use unic_ucd_ident::{is_xid_continue, is_xid_start};
use unicode_casing::CharExt;

use super::objbytes::{PyBytes, PyBytesRef};
use super::objdict::PyDict;
use super::objfloat;
use super::objint::{self, PyInt, PyIntRef};
use super::objiter;
use super::objnone::PyNone;
use super::objsequence::PySliceableSequence;
use super::objslice::PySliceRef;
use super::objtuple;
use super::objtype::{self, PyClassRef};
use super::pystr::{self, adjust_indices, PyCommonString, PyCommonStringWrapper};
use crate::cformat::{
    CFormatPart, CFormatPreconversor, CFormatQuantity, CFormatSpec, CFormatString, CFormatType,
    CNumberType,
};
use crate::format::{FormatParseError, FormatPart, FormatPreconversor, FormatSpec, FormatString};
use crate::function::{OptionalArg, OptionalOption, PyFuncArgs};
use crate::pyhash;
use crate::pyobject::{
    Either, IdProtocol, IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, ThreadSafe, TryFromObject, TryIntoRef, TypeProtocol,
};
use crate::vm::VirtualMachine;

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
#[pyclass(name = "str")]
#[derive(Debug)]
pub struct PyString {
    value: String,
    hash: AtomicCell<Option<pyhash::PyHash>>,
    len: AtomicCell<Option<usize>>,
}
impl ThreadSafe for PyString {}

impl PyString {
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl From<&str> for PyString {
    fn from(s: &str) -> PyString {
        s.to_owned().into()
    }
}

impl From<String> for PyString {
    fn from(s: String) -> PyString {
        PyString {
            value: s,
            hash: AtomicCell::default(),
            len: AtomicCell::default(),
        }
    }
}

pub type PyStringRef = PyRef<PyString>;

impl fmt::Display for PyString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

impl TryIntoRef<PyString> for String {
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<PyString>> {
        Ok(PyString::from(self).into_ref(vm))
    }
}

impl TryIntoRef<PyString> for &str {
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<PyString>> {
        Ok(PyString::from(self).into_ref(vm))
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyStringIterator {
    pub string: PyStringRef,
    position: AtomicCell<usize>,
}
impl ThreadSafe for PyStringIterator {}

impl PyValue for PyStringIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.striterator_type()
    }
}

#[pyimpl]
impl PyStringIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<String> {
        // TODO: use something more performant than chars().nth() that's still atomic
        let pos = self.position.fetch_add(1);

        if let Some(c) = self.string.value.chars().nth(pos) {
            Ok(c.to_string())
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyStringReverseIterator {
    pub position: AtomicCell<isize>,
    pub string: PyStringRef,
}
impl ThreadSafe for PyStringReverseIterator {}

impl PyValue for PyStringReverseIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.strreverseiterator_type()
    }
}

#[pyimpl]
impl PyStringReverseIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<String> {
        let pos = self.position.fetch_sub(1);
        if pos > 0 {
            if let Some(c) = self.string.value.chars().nth(pos as usize - 1) {
                return Ok(c.to_string());
            }
        }
        Err(objiter::new_stop_iteration(vm))
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

#[derive(FromArgs)]
struct StrArgs {
    #[pyarg(positional_or_keyword, optional = true)]
    object: OptionalArg<PyObjectRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    encoding: OptionalArg<PyStringRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    errors: OptionalArg<PyStringRef>,
}

#[pyimpl(flags(BASETYPE))]
impl PyString {
    #[pyslot]
    fn tp_new(cls: PyClassRef, args: StrArgs, vm: &VirtualMachine) -> PyResult<PyStringRef> {
        let string: PyStringRef = match args.object {
            OptionalArg::Present(input) => {
                if let OptionalArg::Present(enc) = args.encoding {
                    vm.decode(input, Some(enc.clone()), args.errors.into_option())?
                        .downcast()
                        .map_err(|obj| {
                            vm.new_type_error(format!(
                                "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                                 encode arbitrary types",
                                enc,
                                obj.class().name,
                            ))
                        })?
                } else {
                    vm.to_str(&input)?
                }
            }
            OptionalArg::Missing => {
                PyString::from(String::new()).into_ref_with_type(vm, cls.clone())?
            }
        };
        if string.class().is(&cls) {
            Ok(string)
        } else {
            PyString::from(string.as_str()).into_ref_with_type(vm, cls)
        }
    }
    #[pymethod(name = "__add__")]
    fn add(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(format!("{}{}", self.value, borrow_value(&rhs)))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        !self.value.is_empty()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            vm.new_bool(self.value == borrow_value(&rhs))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            vm.new_bool(self.value != borrow_value(&rhs))
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyStringRef) -> bool {
        self.value.contains(&needle.value)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: Either<PyIntRef, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        match needle {
            Either::A(pos) => match pos.as_bigint().to_isize() {
                Some(pos) => {
                    let index: usize = if pos.is_negative() {
                        (self.value.chars().count() as isize + pos) as usize
                    } else {
                        pos.abs() as usize
                    };

                    if let Some(character) = self.value.chars().nth(index) {
                        Ok(vm.new_str(character.to_string()))
                    } else {
                        Err(vm.new_index_error("string index out of range".to_owned()))
                    }
                }
                None => {
                    Err(vm
                        .new_index_error("cannot fit 'int' into an index-sized integer".to_owned()))
                }
            },
            Either::B(slice) => {
                let string = self
                    .value
                    .to_owned()
                    .get_slice_items(vm, slice.as_object())?;
                Ok(vm.new_str(string))
            }
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyStringRef) -> bool {
        self.value > other.value
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyStringRef) -> bool {
        self.value >= other.value
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyStringRef) -> bool {
        self.value < other.value
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyStringRef) -> bool {
        self.value <= other.value
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self) -> pyhash::PyHash {
        self.hash.load().unwrap_or_else(|| {
            let hash = pyhash::hash_value(&self.value);
            self.hash.store(Some(hash));
            hash
        })
    }

    #[pymethod(name = "__len__")]
    #[inline]
    fn len(&self) -> usize {
        self.len.load().unwrap_or_else(|| {
            let len = self.value.chars().count();
            self.len.store(Some(len));
            len
        })
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.value.capacity() * size_of::<u8>()
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, value: isize) -> String {
        self.value.repeat(value.max(0) as usize)
    }

    #[pymethod(name = "__str__")]
    fn str(zelf: PyRef<Self>) -> PyStringRef {
        zelf
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
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
                    1
                }
                ch if (ch as u32) < 0x100 => 4,   // \xHH
                ch if (ch as u32) < 0x10000 => 6, // \uHHHH
                _ => 10,                          // \uHHHHHHHH
            };
            if out_len > (std::isize::MAX as usize) - incr {
                return Err(vm.new_overflow_error("string is too long to generate repr".to_owned()));
            }
            out_len += incr;
        }

        let (quote, unchanged) = {
            let mut quote = '\'';
            let mut unchanged = out_len == in_len;
            if squote > 0 {
                unchanged = false;
                if dquote > 0 {
                    // Both squote and dquote present. Use squote, and escape them
                    out_len += squote;
                } else {
                    quote = '"';
                }
            }
            (quote, unchanged)
        };

        out_len += 2; // quotes

        let mut repr = String::with_capacity(out_len);
        repr.push(quote);
        if unchanged {
            repr.push_str(&self.value);
        } else {
            for ch in self.value.chars() {
                if ch == quote || ch == '\\' {
                    repr.push('\\');
                    repr.push(ch);
                } else if ch == '\n' {
                    repr.push_str("\\n")
                } else if ch == '\t' {
                    repr.push_str("\\t");
                } else if ch == '\r' {
                    repr.push_str("\\r");
                } else if ch < ' ' || ch as u32 == 0x7F {
                    repr.push_str(&format!("\\x{:02x}", ch as u32));
                } else if ch.is_ascii() {
                    repr.push(ch);
                } else if !char_is_printable(ch) {
                    let code = ch as u32;
                    let escaped = if code < 0xff {
                        format!("\\x{:02x}", code)
                    } else if code < 0xffff {
                        format!("\\u{:04x}", code)
                    } else {
                        format!("\\U{:08x}", code)
                    };
                    repr.push_str(&escaped);
                } else {
                    repr.push(ch)
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
        caseless::default_case_fold_str(&self.value)
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
    fn strip(&self, chars: OptionalOption<PyStringRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_matches(|c| chars.contains(c)),
                |s| s.trim(),
            )
            .to_owned()
    }

    #[pymethod]
    fn lstrip(&self, chars: OptionalOption<PyStringRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_start_matches(|c| chars.contains(c)),
                |s| s.trim_start(),
            )
            .to_owned()
    }

    #[pymethod]
    fn rstrip(&self, chars: OptionalOption<PyStringRef>) -> String {
        self.value
            .py_strip(
                chars,
                |s, chars| s.trim_end_matches(|c| chars.contains(c)),
                |s| s.trim_end(),
            )
            .to_owned()
    }

    #[pymethod]
    fn endswith(&self, args: pystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.value.as_str().py_startsendswith(
            args,
            "endswith",
            "str",
            |s, x: &PyStringRef| s.ends_with(x.as_str()),
            vm,
        )
    }

    #[pymethod]
    fn startswith(&self, args: pystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.value.as_str().py_startsendswith(
            args,
            "startswith",
            "str",
            |s, x: &PyStringRef| s.starts_with(x.as_str()),
            vm,
        )
    }

    #[pymethod]
    fn removeprefix(&self, pref: PyStringRef) -> PyResult<String> {
        if self.value.as_str().starts_with(&pref.value) {
            return Ok(self.value[pref.value.len()..].to_string());
        }
        Ok(self.value.to_string())
    }

    #[pymethod]
    fn removesuffix(&self, suff: PyStringRef) -> PyResult<String> {
        if self.value.as_str().ends_with(&suff.value) {
            return Ok(self.value[..self.value.len() - suff.value.len()].to_string());
        }
        Ok(self.value.to_string())
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

        if self.value.is_empty() {
            false
        } else {
            self.value
                .chars()
                .filter(|c| !c.is_digit(10))
                .all(|c| valid_unicodes.contains(&(c as u16)))
        }
    }

    #[pymethod]
    fn isdecimal(&self) -> bool {
        if self.value.is_empty() {
            false
        } else {
            self.value.chars().all(|c| c.is_ascii_digit())
        }
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let format_string_text = &self.value;
        let format_string = CFormatString::from_str(format_string_text)
            .map_err(|err| vm.new_value_error(err.to_string()))?;
        do_cformat(vm, format_string, values.clone())
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod]
    fn format(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        if args.args.is_empty() {
            return Err(vm.new_type_error(
                "descriptor 'format' of 'str' object needs an argument".to_owned(),
            ));
        }

        let zelf = &args.args[0];
        if !objtype::isinstance(&zelf, &vm.ctx.str_type()) {
            let zelf_typ = zelf.class();
            let actual_type = vm.to_pystr(&zelf_typ)?;
            return Err(vm.new_type_error(format!(
                "descriptor 'format' requires a 'str' object but received a '{}'",
                actual_type
            )));
        }
        let format_string_text = borrow_value(zelf);
        match FormatString::from_str(format_string_text) {
            Ok(format_string) => perform_format(vm, &format_string, &args),
            Err(err) => match err {
                FormatParseError::UnmatchedBracket => {
                    Err(vm.new_value_error("expected '}' before end of string".to_owned()))
                }
                _ => Err(vm.new_value_error("Unexpected error parsing format string".to_owned())),
            },
        }
    }

    /// S.format_map(mapping) -> str
    ///
    /// Return a formatted version of S, using substitutions from mapping.
    /// The substitutions are identified by braces ('{' and '}').
    #[pymethod]
    fn format_map(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        if args.args.len() != 2 {
            return Err(vm.new_type_error(format!(
                "format_map() takes exactly one argument ({} given)",
                args.args.len() - 1
            )));
        }

        let zelf = &args.args[0];
        let format_string_text = borrow_value(zelf);
        match FormatString::from_str(format_string_text) {
            Ok(format_string) => perform_format_map(vm, &format_string, &args.args[1]),
            Err(err) => match err {
                FormatParseError::UnmatchedBracket => {
                    Err(vm.new_value_error("expected '}' before end of string".to_owned()))
                }
                _ => Err(vm.new_value_error("Unexpected error parsing format string".to_owned())),
            },
        }
    }

    #[pymethod(name = "__format__")]
    fn format_str(&self, spec: PyStringRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_string(&self.value))
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
    fn replace(&self, old: PyStringRef, new: PyStringRef, count: OptionalArg<isize>) -> String {
        match count {
            OptionalArg::Present(maxcount) if maxcount >= 0 => {
                if maxcount == 0 || self.value.is_empty() {
                    // nothing to do; return the original bytes
                    return self.value.clone();
                }
                self.value
                    .replacen(&old.value, &new.value, maxcount as usize)
            }
            _ => self.value.replace(&old.value, &new.value),
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

    // cpython's isspace ignores whitespace, including \t and \n, etc, unless the whole string is empty
    // which is why isspace is using is_ascii_whitespace. Same for isupper & islower
    #[pymethod]
    fn isspace(&self) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii_whitespace())
    }

    // Return true if all cased characters in the string are lowercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn islower(&self) -> bool {
        // CPython unicode_islower_impl
        let mut cased = false;
        for c in self.value.chars() {
            if c.is_uppercase() {
                return false;
            } else if !cased && c.is_lowercase() {
                cased = true
            }
        }
        cased
    }

    // Return true if all cased characters in the string are uppercase and there is at least one cased character, false otherwise.
    #[pymethod]
    fn isupper(&self) -> bool {
        // CPython unicode_isupper_impl
        let mut cased = false;
        for c in self.value.chars() {
            if c.is_lowercase() {
                return false;
            } else if !cased && c.is_uppercase() {
                cased = true
            }
        }
        cased
    }

    #[pymethod]
    fn isascii(&self) -> bool {
        self.value.chars().all(|c| c.is_ascii())
    }

    #[pymethod]
    fn splitlines(&self, args: pystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        let mut elements = vec![];
        let mut curr = "".to_owned();
        let mut chars = self.value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\n' || ch == '\r' {
                if args.keepends {
                    curr.push(ch);
                }
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    continue;
                }
                elements.push(vm.ctx.new_str(curr.clone()));
                curr.clear();
            } else {
                curr.push(ch);
            }
        }
        if !curr.is_empty() {
            elements.push(vm.ctx.new_str(curr));
        }
        vm.ctx.new_list(elements)
    }

    #[pymethod]
    fn join(&self, iterable: PyIterable<PyStringRef>, vm: &VirtualMachine) -> PyResult<String> {
        let mut joined = String::new();

        for (idx, elem) in iterable.iter(vm)?.enumerate() {
            let elem = elem?;
            if idx != 0 {
                joined.push_str(&self.value);
            }
            joined.push_str(&elem.value)
        }

        Ok(joined)
    }

    #[inline]
    fn _find<F>(&self, args: FindArgs, find: F) -> Option<usize>
    where
        F: Fn(&str, &str) -> Option<usize>,
    {
        let (sub, range) = args.get_value(self.len());
        self.value.py_find(&sub.value, range, find)
    }

    #[pymethod]
    fn find(&self, args: FindArgs) -> isize {
        self._find(args, |r, s| r.find(s))
            .map_or(-1, |v| v as isize)
    }

    #[pymethod]
    fn rfind(&self, args: FindArgs) -> isize {
        self._find(args, |r, s| r.rfind(s))
            .map_or(-1, |v| v as isize)
    }

    #[pymethod]
    fn index(&self, args: FindArgs, vm: &VirtualMachine) -> PyResult<usize> {
        self._find(args, |r, s| r.find(s))
            .ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn rindex(&self, args: FindArgs, vm: &VirtualMachine) -> PyResult<usize> {
        self._find(args, |r, s| r.rfind(s))
            .ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn partition(&self, sub: PyStringRef, vm: &VirtualMachine) -> PyResult {
        if sub.value.is_empty() {
            return Err(vm.new_value_error("empty separator".to_owned()));
        }
        let mut sp = self.value.splitn(2, &sub.value);
        let front = sp.next().unwrap();
        let elems = if let Some(back) = sp.next() {
            [front, &sub.value, back]
        } else {
            [front, "", ""]
        };
        Ok(vm.ctx.new_tuple(
            elems
                .iter()
                .map(|&s| vm.ctx.new_str(s.to_owned()))
                .collect(),
        ))
    }

    #[pymethod]
    fn rpartition(&self, sub: PyStringRef, vm: &VirtualMachine) -> PyResult {
        if sub.value.is_empty() {
            return Err(vm.new_value_error("empty separator".to_owned()));
        }
        let mut sp = self.value.rsplitn(2, &sub.value);
        let back = sp.next().unwrap();
        let elems = if let Some(front) = sp.next() {
            [front, &sub.value, back]
        } else {
            ["", "", back]
        };
        Ok(vm.ctx.new_tuple(
            elems
                .iter()
                .map(|&s| vm.ctx.new_str(s.to_owned()))
                .collect(),
        ))
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
            .py_count(&needle.value, range, |h, n| h.matches(n).count())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> String {
        use super::objbyteinner::bytes_zfill;
        unsafe {
            String::from_utf8_unchecked(bytes_zfill(
                self.value.as_bytes(),
                width.to_usize().unwrap_or(0),
            ))
        }
    }

    fn get_fill_char(fillchar: OptionalArg<PyStringRef>, vm: &VirtualMachine) -> PyResult<char> {
        match fillchar {
            OptionalArg::Present(ref s) => {
                let mut chars = s.value.chars();
                let first = chars.next();
                let second = chars.next();
                if first.is_some() && second.is_none() {
                    // safeness is guaranteed by upper first.is_some()
                    Ok(first.unwrap_or_else(|| unsafe { std::hint::unreachable_unchecked() }))
                } else {
                    Err(vm.new_type_error(
                        "The fill character must be exactly one character long".to_owned(),
                    ))
                }
            }
            OptionalArg::Missing => Ok(' '),
        }
    }

    #[inline]
    fn pad(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStringRef>,
        pad: fn(&str, usize, char) -> String,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let fillchar = Self::get_fill_char(fillchar, vm)?;
        Ok(if self.len() as isize >= width {
            String::from(&self.value)
        } else {
            pad(&self.value, width as usize, fillchar)
        })
    }

    #[pymethod]
    fn center(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.pad(width, fillchar, PyCommonString::<char>::py_center, vm)
    }

    #[pymethod]
    fn ljust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.pad(width, fillchar, PyCommonString::<char>::py_ljust, vm)
    }

    #[pymethod]
    fn rjust(
        &self,
        width: isize,
        fillchar: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.pad(width, fillchar, PyCommonString::<char>::py_rjust, vm)
    }

    #[pymethod]
    fn expandtabs(&self, args: pystr::ExpandTabsArgs) -> String {
        let tab_stop = args.tabsize();
        let mut expanded_str = String::with_capacity(self.value.len());
        let mut tab_size = tab_stop;
        let mut col_count = 0 as usize;
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
            format!("'{}' object is not subscriptable", table.class().name)
        })?;

        let mut translated = String::new();
        for c in self.value.chars() {
            match table.get_item(&(c as u32).into_pyobject(vm)?, vm) {
                Ok(value) => {
                    if let Some(text) = value.payload::<PyString>() {
                        translated.push_str(&text.value);
                    } else if let Some(bigint) = value.payload::<PyInt>() {
                        match bigint.as_bigint().to_u32().and_then(std::char::from_u32) {
                            Some(ch) => translated.push(ch as char),
                            None => {
                                return Err(vm.new_value_error(
                                    "character mapping must be in range(0x110000)".to_owned(),
                                ));
                            }
                        }
                    } else if value.payload::<PyNone>().is_some() {
                        // Do Nothing
                    } else {
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
        to_str: OptionalArg<PyStringRef>,
        none_str: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let new_dict = vm.context().new_dict();
        if let OptionalArg::Present(to_str) = to_str {
            match dict_or_str.downcast::<PyString>() {
                Ok(from_str) => {
                    if to_str.len() == from_str.len() {
                        for (c1, c2) in from_str.value.chars().zip(to_str.value.chars()) {
                            new_dict.set_item(&vm.new_int(c1 as u32), vm.new_int(c2 as u32), vm)?;
                        }
                        if let OptionalArg::Present(none_str) = none_str {
                            for c in none_str.value.chars() {
                                new_dict.set_item(&vm.new_int(c as u32), vm.get_none(), vm)?;
                            }
                        }
                        new_dict.into_pyobject(vm)
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
                                &num.as_bigint().to_i32().into_pyobject(vm)?,
                                val,
                                vm,
                            )?;
                        } else if let Some(string) = key.payload::<PyString>() {
                            if string.len() == 1 {
                                let num_value = string.value.chars().next().unwrap() as u32;
                                new_dict.set_item(&num_value.into_pyobject(vm)?, val, vm)?;
                            } else {
                                return Err(vm.new_value_error(
                                    "string keys in translate table must be of length 1".to_owned(),
                                ));
                            }
                        }
                    }
                    new_dict.into_pyobject(vm)
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
    fn iter(zelf: PyRef<Self>) -> PyStringIterator {
        PyStringIterator {
            position: AtomicCell::new(0),
            string: zelf,
        }
    }

    #[pymethod(magic)]
    fn reversed(zelf: PyRef<Self>) -> PyStringReverseIterator {
        let begin = zelf.value.chars().count();

        PyStringReverseIterator {
            position: AtomicCell::new(begin as isize),
            string: zelf,
        }
    }
}

#[derive(FromArgs)]
struct EncodeArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    encoding: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    errors: Option<PyStringRef>,
}

pub(crate) fn encode_string(
    s: PyStringRef,
    encoding: Option<PyStringRef>,
    errors: Option<PyStringRef>,
    vm: &VirtualMachine,
) -> PyResult<PyBytesRef> {
    vm.encode(s.into_object(), encoding.clone(), errors)?
        .downcast::<PyBytes>()
        .map_err(|obj| {
            vm.new_type_error(format!(
                "'{}' encoder returned '{}' instead of 'bytes'; use codecs.encode() to \
                 encode arbitrary types",
                encoding.as_ref().map_or("utf-8", |s| s.as_str()),
                obj.class().name,
            ))
        })
}

impl PyValue for PyString {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.str_type()
    }
}

impl IntoPyObject for String {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self))
    }
}

impl IntoPyObject for &str {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.to_owned()))
    }
}

impl IntoPyObject for &String {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.clone()))
    }
}

impl TryFromObject for std::ffi::CString {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let s = PyStringRef::try_from_object(vm, obj)?;
        Self::new(s.as_str().to_owned())
            .map_err(|_| vm.new_value_error("embedded null character".to_owned()))
    }
}

type SplitArgs = pystr::SplitArgs<PyStringRef, str, char>;

#[derive(FromArgs)]
pub struct FindArgs {
    #[pyarg(positional_only, optional = false)]
    sub: PyStringRef,
    #[pyarg(positional_only, default = "None")]
    start: Option<PyIntRef>,
    #[pyarg(positional_only, default = "None")]
    end: Option<PyIntRef>,
}

impl FindArgs {
    fn get_value(self, len: usize) -> (PyStringRef, std::ops::Range<usize>) {
        let range = adjust_indices(self.start, self.end, len);
        (self.sub, range)
    }
}

pub fn init(ctx: &PyContext) {
    PyString::extend_class(ctx, &ctx.types.str_type);

    PyStringIterator::extend_class(ctx, &ctx.types.striterator_type);
    PyStringReverseIterator::extend_class(ctx, &ctx.types.strreverseiterator_type);
}

pub fn clone_value(obj: &PyObjectRef) -> String {
    obj.payload::<PyString>().unwrap().value.clone()
}

pub fn borrow_value(obj: &PyObjectRef) -> &str {
    &obj.payload::<PyString>().unwrap().value
}

fn call_getitem(vm: &VirtualMachine, container: &PyObjectRef, key: &PyObjectRef) -> PyResult {
    vm.call_method(container, "__getitem__", vec![key.clone()])
}

fn call_object_format(vm: &VirtualMachine, argument: PyObjectRef, format_spec: &str) -> PyResult {
    let (preconversor, new_format_spec) = FormatPreconversor::parse_and_consume(format_spec);
    let argument = match preconversor {
        Some(FormatPreconversor::Str) => vm.call_method(&argument, "__str__", vec![])?,
        Some(FormatPreconversor::Repr) => vm.call_method(&argument, "__repr__", vec![])?,
        Some(FormatPreconversor::Ascii) => vm.call_method(&argument, "__repr__", vec![])?,
        Some(FormatPreconversor::Bytes) => vm.call_method(&argument, "decode", vec![])?,
        None => argument,
    };
    let returned_type = vm.ctx.new_str(new_format_spec.to_owned());

    let result = vm.call_method(&argument, "__format__", vec![returned_type])?;
    if !objtype::isinstance(&result, &vm.ctx.str_type()) {
        let result_type = result.class();
        let actual_type = vm.to_pystr(&result_type)?;
        return Err(vm.new_type_error(format!("__format__ must return a str, not {}", actual_type)));
    }
    Ok(result)
}

fn do_cformat_specifier(
    vm: &VirtualMachine,
    format_spec: &mut CFormatSpec,
    obj: PyObjectRef,
) -> PyResult<String> {
    use CNumberType::*;
    // do the formatting by type
    let format_type = &format_spec.format_type;

    match format_type {
        CFormatType::String(preconversor) => {
            let result = match preconversor {
                CFormatPreconversor::Str => vm.call_method(&obj.clone(), "__str__", vec![])?,
                CFormatPreconversor::Repr => vm.call_method(&obj.clone(), "__repr__", vec![])?,
                CFormatPreconversor::Ascii => vm.call_method(&obj.clone(), "__repr__", vec![])?,
                CFormatPreconversor::Bytes => vm.call_method(&obj.clone(), "decode", vec![])?,
            };
            Ok(format_spec.format_string(clone_value(&result)))
        }
        CFormatType::Number(_) => {
            if !objtype::isinstance(&obj, &vm.ctx.int_type()) {
                let required_type_string = match format_type {
                    CFormatType::Number(Decimal) => "a number",
                    CFormatType::Number(_) => "an integer",
                    _ => unreachable!(),
                };
                return Err(vm.new_type_error(format!(
                    "%{} format: {} is required, not {}",
                    format_spec.format_char,
                    required_type_string,
                    obj.class()
                )));
            }
            Ok(format_spec.format_number(objint::get_value(&obj)))
        }
        CFormatType::Float(_) => if objtype::isinstance(&obj, &vm.ctx.float_type()) {
            format_spec.format_float(objfloat::get_value(&obj))
        } else if objtype::isinstance(&obj, &vm.ctx.int_type()) {
            format_spec.format_float(objint::get_value(&obj).to_f64().unwrap())
        } else {
            let required_type_string = "an floating point or integer";
            return Err(vm.new_type_error(format!(
                "%{} format: {} is required, not {}",
                format_spec.format_char,
                required_type_string,
                obj.class()
            )));
        }
        .map_err(|e| vm.new_not_implemented_error(e)),
        CFormatType::Character => {
            let char_string = {
                if objtype::isinstance(&obj, &vm.ctx.int_type()) {
                    // BigInt truncation is fine in this case because only the unicode range is relevant
                    match objint::get_value(&obj).to_u32().and_then(char::from_u32) {
                        Some(value) => Ok(value.to_string()),
                        None => {
                            Err(vm.new_overflow_error("%c arg not in range(0x110000)".to_owned()))
                        }
                    }
                } else if objtype::isinstance(&obj, &vm.ctx.str_type()) {
                    let s = borrow_value(&obj);
                    let num_chars = s.chars().count();
                    if num_chars != 1 {
                        Err(vm.new_type_error("%c requires int or char".to_owned()))
                    } else {
                        Ok(s.chars().next().unwrap().to_string())
                    }
                } else {
                    // TODO re-arrange this block so this error is only created once
                    Err(vm.new_type_error("%c requires int or char".to_owned()))
                }
            }?;
            format_spec.precision = Some(CFormatQuantity::Amount(1));
            Ok(format_spec.format_string(char_string))
        }
    }
}

fn try_update_quantity_from_tuple(
    vm: &VirtualMachine,
    elements: &mut dyn Iterator<Item = PyObjectRef>,
    q: &mut Option<CFormatQuantity>,
    mut tuple_index: usize,
) -> PyResult<usize> {
    match q {
        Some(CFormatQuantity::FromValuesTuple) => {
            match elements.next() {
                Some(width_obj) => {
                    tuple_index += 1;
                    if !objtype::isinstance(&width_obj, &vm.ctx.int_type()) {
                        Err(vm.new_type_error("* wants int".to_owned()))
                    } else {
                        // TODO: handle errors when truncating BigInt to usize
                        *q = Some(CFormatQuantity::Amount(
                            objint::get_value(&width_obj).to_usize().unwrap(),
                        ));
                        Ok(tuple_index)
                    }
                }
                None => Err(vm.new_type_error("not enough arguments for format string".to_owned())),
            }
        }
        _ => Ok(tuple_index),
    }
}

pub fn do_cformat_string(
    vm: &VirtualMachine,
    mut format_string: CFormatString,
    values_obj: PyObjectRef,
) -> PyResult<String> {
    let mut final_string = String::new();
    let num_specifiers = format_string
        .format_parts
        .iter()
        .filter(|(_, part)| CFormatPart::is_specifier(part))
        .count();
    let mapping_required = format_string
        .format_parts
        .iter()
        .any(|(_, part)| CFormatPart::has_key(part))
        && format_string
            .format_parts
            .iter()
            .filter(|(_, part)| CFormatPart::is_specifier(part))
            .all(|(_, part)| CFormatPart::has_key(part));

    let values = if mapping_required {
        if !objtype::isinstance(&values_obj, &vm.ctx.dict_type()) {
            return Err(vm.new_type_error("format requires a mapping".to_owned()));
        }
        values_obj.clone()
    } else {
        // check for only literal parts, in which case only dict or empty tuple is allowed
        if num_specifiers == 0
            && !(objtype::isinstance(&values_obj, &vm.ctx.types.tuple_type)
                && objtuple::get_value(&values_obj).is_empty())
            && !objtype::isinstance(&values_obj, &vm.ctx.types.dict_type)
        {
            return Err(vm.new_type_error(
                "not all arguments converted during string formatting".to_owned(),
            ));
        }

        // convert `values_obj` to a new tuple if it's not a tuple
        if !objtype::isinstance(&values_obj, &vm.ctx.tuple_type()) {
            vm.ctx.new_tuple(vec![values_obj.clone()])
        } else {
            values_obj.clone()
        }
    };

    let mut tuple_index: usize = 0;
    for (_, part) in &mut format_string.format_parts {
        let result_string: String = match part {
            CFormatPart::Spec(format_spec) => {
                // try to get the object
                let obj: PyObjectRef = match &format_spec.mapping_key {
                    Some(key) => {
                        // TODO: change the KeyError message to match the one in cpython
                        call_getitem(vm, &values, &vm.ctx.new_str(key.to_owned()))?
                    }
                    None => {
                        let mut elements = objtuple::get_value(&values)
                            .to_vec()
                            .into_iter()
                            .skip(tuple_index);

                        tuple_index = try_update_quantity_from_tuple(
                            vm,
                            &mut elements,
                            &mut format_spec.min_field_width,
                            tuple_index,
                        )?;
                        tuple_index = try_update_quantity_from_tuple(
                            vm,
                            &mut elements,
                            &mut format_spec.precision,
                            tuple_index,
                        )?;

                        let obj = match elements.next() {
                            Some(obj) => Ok(obj),
                            None => Err(vm.new_type_error(
                                "not enough arguments for format string".to_owned(),
                            )),
                        }?;
                        tuple_index += 1;

                        obj
                    }
                };
                do_cformat_specifier(vm, format_spec, obj)
            }
            CFormatPart::Literal(literal) => Ok(literal.clone()),
        }?;
        final_string.push_str(&result_string);
    }

    // check that all arguments were converted
    if (!mapping_required && objtuple::get_value(&values).get(tuple_index).is_some())
        && !objtype::isinstance(&values_obj, &vm.ctx.types.dict_type)
    {
        return Err(
            vm.new_type_error("not all arguments converted during string formatting".to_owned())
        );
    }
    Ok(final_string)
}

fn do_cformat(
    vm: &VirtualMachine,
    format_string: CFormatString,
    values_obj: PyObjectRef,
) -> PyResult {
    Ok(vm
        .ctx
        .new_str(do_cformat_string(vm, format_string, values_obj)?))
}

fn perform_format(
    vm: &VirtualMachine,
    format_string: &FormatString,
    arguments: &PyFuncArgs,
) -> PyResult {
    let mut final_string = String::new();
    if format_string.format_parts.iter().any(FormatPart::is_auto)
        && format_string.format_parts.iter().any(FormatPart::is_index)
    {
        return Err(vm.new_value_error(
            "cannot switch from automatic field numbering to manual field specification".to_owned(),
        ));
    }
    let mut auto_argument_index: usize = 1;
    for part in &format_string.format_parts {
        let result_string: String = match part {
            FormatPart::AutoSpec(format_spec) => {
                let result = match arguments.args.get(auto_argument_index) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_index_error("tuple index out of range".to_owned()));
                    }
                };
                auto_argument_index += 1;
                clone_value(&result)
            }
            FormatPart::IndexSpec(index, format_spec) => {
                let result = match arguments.args.get(*index + 1) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_index_error("tuple index out of range".to_owned()));
                    }
                };
                clone_value(&result)
            }
            FormatPart::KeywordSpec(keyword, format_spec) => {
                let result = match arguments.get_optional_kwarg(&keyword) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_key_error(vm.new_str(keyword.to_owned())));
                    }
                };
                clone_value(&result)
            }
            FormatPart::Literal(literal) => literal.clone(),
        };
        final_string.push_str(&result_string);
    }
    Ok(vm.ctx.new_str(final_string))
}

fn perform_format_map(
    vm: &VirtualMachine,
    format_string: &FormatString,
    dict: &PyObjectRef,
) -> PyResult {
    let mut final_string = String::new();
    for part in &format_string.format_parts {
        let result_string: String = match part {
            FormatPart::AutoSpec(_) | FormatPart::IndexSpec(_, _) => {
                return Err(
                    vm.new_value_error("Format string contains positional fields".to_owned())
                );
            }
            FormatPart::KeywordSpec(keyword, format_spec) => {
                let argument = dict.get_item(keyword, &vm)?;
                let result = call_object_format(vm, argument.clone(), &format_spec)?;
                clone_value(&result)
            }
            FormatPart::Literal(literal) => literal.clone(),
        };
        final_string.push_str(&result_string);
    }
    Ok(vm.ctx.new_str(final_string))
}

impl PySliceableSequence for String {
    type Sliced = String;

    fn do_slice(&self, range: Range<usize>) -> Self::Sliced {
        self.chars()
            .skip(range.start)
            .take(range.end - range.start)
            .collect()
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self::Sliced {
        let count = self.chars().count();

        self.chars()
            .rev()
            .skip(count - range.end)
            .take(range.end - range.start)
            .collect()
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        self.chars()
            .skip(range.start)
            .take(range.end - range.start)
            .step_by(step)
            .collect()
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self::Sliced {
        let count = self.chars().count();

        self.chars()
            .rev()
            .skip(count - range.end)
            .take(range.end - range.start)
            .step_by(step)
            .collect()
    }

    fn empty() -> Self::Sliced {
        String::default()
    }

    fn len(&self) -> usize {
        self.chars().count()
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
            assert_eq!(PyString::from(input).title().as_str(), title);
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
            assert!(PyString::from(s).istitle());
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
            assert!(!PyString::from(s).istitle());
        }
    }

    #[test]
    fn str_maketrans_and_translate() {
        let vm: VirtualMachine = Default::default();

        let table = vm.context().new_dict();
        table
            .set_item("a", vm.new_str("🎅".to_owned()), &vm)
            .unwrap();
        table.set_item("b", vm.get_none(), &vm).unwrap();
        table
            .set_item("c", vm.new_str("xda".to_owned()), &vm)
            .unwrap();
        let translated = PyString::maketrans(
            table.into_object(),
            OptionalArg::Missing,
            OptionalArg::Missing,
            &vm,
        )
        .unwrap();
        let text = PyString::from("abc");
        let translated = text.translate(translated, &vm).unwrap();
        assert_eq!(translated, "🎅xda".to_owned());
        let translated = text.translate(vm.new_int(3), &vm);
        assert_eq!(translated.unwrap_err().class().name, "TypeError".to_owned());
    }
}

impl PyCommonStringWrapper<str> for PyStringRef {
    fn as_ref(&self) -> &str {
        self.value.as_str()
    }
}

impl PyCommonString<char> for str {
    type Container = String;

    fn with_capacity(capacity: usize) -> Self::Container {
        String::with_capacity(capacity)
    }

    fn get_bytes<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self {
        &self[range]
    }

    fn get_chars<'a>(&'a self, range: std::ops::Range<usize>) -> &'a Self {
        let mut chars = self.chars();
        for _ in 0..range.start {
            let _ = chars.next();
        }
        let start = chars.as_str();
        for _ in range {
            let _ = chars.next();
        }
        let end = chars.as_str();
        &start[..start.len() - end.len()]
    }

    fn is_empty(&self) -> bool {
        Self::is_empty(self)
    }

    fn bytes_len(&self) -> usize {
        Self::len(self)
    }

    fn chars_len(&self) -> usize {
        self.chars().count()
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

    fn py_pad(&self, left: usize, right: usize, fill: char) -> Self::Container {
        let mut u = String::with_capacity((left + right) * fill.len_utf8() + self.len());
        u.extend(std::iter::repeat(fill).take(left));
        u.push_str(self);
        u.extend(std::iter::repeat(fill).take(right));
        u
    }
}

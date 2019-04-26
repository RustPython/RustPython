use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::str::FromStr;
use std::string::ToString;

use num_traits::ToPrimitive;
use unicode_casing::CharExt;
use unicode_segmentation::UnicodeSegmentation;

use crate::format::{FormatParseError, FormatPart, FormatString};
use crate::function::{OptionalArg, PyFuncArgs};
use crate::pyobject::{
    IdProtocol, IntoPyObject, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TryIntoRef, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objint;
use super::objsequence::PySliceableSequence;
use super::objslice::PySlice;
use super::objtype::{self, PyClassRef};

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
#[derive(Clone, Debug)]
pub struct PyString {
    // TODO: shouldn't be public
    pub value: String,
}

impl PyString {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl From<&str> for PyString {
    fn from(s: &str) -> PyString {
        PyString {
            value: s.to_string(),
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
        Ok(PyString { value: self }.into_ref(vm))
    }
}

impl TryIntoRef<PyString> for &str {
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<PyString>> {
        Ok(PyString {
            value: self.to_string(),
        }
        .into_ref(vm))
    }
}

#[pyimpl]
impl PyString {
    // TODO: should with following format
    // class str(object='')
    // class str(object=b'', encoding='utf-8', errors='strict')
    #[pymethod(name = "__new__")]
    fn new(
        cls: PyClassRef,
        object: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStringRef> {
        let string = match object {
            OptionalArg::Present(ref input) => vm.to_str(input)?.into_object(),
            OptionalArg::Missing => vm.new_str("".to_string()),
        };
        if string.class().is(&cls) {
            TryFromObject::try_from_object(vm, string)
        } else {
            let payload = string.payload::<PyString>().unwrap();
            payload.clone().into_ref_with_type(vm, cls)
        }
    }
    #[pymethod(name = "__add__")]
    fn add(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(format!("{}{}", self.value, get_value(&rhs)))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> bool {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            self.value == get_value(&rhs)
        } else {
            false
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(&self, needle: PyStringRef, _vm: &VirtualMachine) -> bool {
        self.value.contains(&needle.value)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        subscript(vm, &self.value, needle)
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value > get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value >= get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value < get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value <= get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self, _vm: &VirtualMachine) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish() as usize
    }

    #[pymethod(name = "__len__")]
    fn len(&self, _vm: &VirtualMachine) -> usize {
        self.value.chars().count()
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, val: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        if objtype::isinstance(&val, &vm.ctx.int_type()) {
            let value = &self.value;
            let multiplier = objint::get_value(&val).to_i32().unwrap();
            let mut result = String::new();
            for _x in 0..multiplier {
                result.push_str(value.as_str());
            }
            Ok(result)
        } else {
            Err(vm.new_type_error(format!("Cannot multiply {} and {}", self, val)))
        }
    }

    #[pymethod(name = "__str__")]
    fn str(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyStringRef {
        zelf
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, _vm: &VirtualMachine) -> String {
        let value = &self.value;
        let quote_char = if count_char(value, '\'') > count_char(value, '"') {
            '"'
        } else {
            '\''
        };
        let mut formatted = String::new();
        formatted.push(quote_char);
        for c in value.chars() {
            if c == quote_char || c == '\\' {
                formatted.push('\\');
                formatted.push(c);
            } else if c == '\n' {
                formatted.push('\\');
                formatted.push('n');
            } else if c == '\t' {
                formatted.push('\\');
                formatted.push('t');
            } else if c == '\r' {
                formatted.push('\\');
                formatted.push('r');
            } else {
                formatted.push(c);
            }
        }
        formatted.push(quote_char);
        formatted
    }

    #[pymethod]
    fn lower(&self, _vm: &VirtualMachine) -> String {
        self.value.to_lowercase()
    }

    // casefold is much more aggressive than lower
    #[pymethod]
    fn casefold(&self, _vm: &VirtualMachine) -> String {
        caseless::default_case_fold_str(&self.value)
    }

    #[pymethod]
    fn upper(&self, _vm: &VirtualMachine) -> String {
        self.value.to_uppercase()
    }

    #[pymethod]
    fn capitalize(&self, _vm: &VirtualMachine) -> String {
        let (first_part, lower_str) = self.value.split_at(1);
        format!("{}{}", first_part.to_uppercase(), lower_str)
    }

    #[pymethod]
    fn split(
        &self,
        pattern: OptionalArg<PyStringRef>,
        num: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        let value = &self.value;
        let pattern = match pattern {
            OptionalArg::Present(ref s) => &s.value,
            OptionalArg::Missing => " ",
        };
        let num_splits = num
            .into_option()
            .unwrap_or_else(|| value.split(pattern).count());
        let elements = value
            .splitn(num_splits + 1, pattern)
            .map(|o| vm.ctx.new_str(o.to_string()))
            .collect();
        vm.ctx.new_list(elements)
    }

    #[pymethod]
    fn rsplit(
        &self,
        pattern: OptionalArg<PyStringRef>,
        num: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        let value = &self.value;
        let pattern = match pattern {
            OptionalArg::Present(ref s) => &s.value,
            OptionalArg::Missing => " ",
        };
        let num_splits = num
            .into_option()
            .unwrap_or_else(|| value.split(pattern).count());
        let elements = value
            .rsplitn(num_splits + 1, pattern)
            .map(|o| vm.ctx.new_str(o.to_string()))
            .collect();
        vm.ctx.new_list(elements)
    }

    #[pymethod]
    fn strip(&self, chars: OptionalArg<PyStringRef>, _vm: &VirtualMachine) -> String {
        let chars = match chars {
            OptionalArg::Present(ref chars) => &chars.value,
            OptionalArg::Missing => return self.value.trim().to_string(),
        };
        self.value.trim_matches(|c| chars.contains(c)).to_string()
    }

    #[pymethod]
    fn lstrip(&self, chars: OptionalArg<PyStringRef>, _vm: &VirtualMachine) -> String {
        let chars = match chars {
            OptionalArg::Present(ref chars) => &chars.value,
            OptionalArg::Missing => return self.value.trim_start().to_string(),
        };
        self.value
            .trim_start_matches(|c| chars.contains(c))
            .to_string()
    }

    #[pymethod]
    fn rstrip(&self, chars: OptionalArg<PyStringRef>, _vm: &VirtualMachine) -> String {
        let chars = match chars {
            OptionalArg::Present(ref chars) => &chars.value,
            OptionalArg::Missing => return self.value.trim_end().to_string(),
        };
        self.value
            .trim_end_matches(|c| chars.contains(c))
            .to_string()
    }

    #[pymethod]
    fn endswith(
        &self,
        suffix: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &VirtualMachine,
    ) -> bool {
        if let Some((start, end)) = adjust_indices(start, end, self.value.len()) {
            self.value[start..end].ends_with(&suffix.value)
        } else {
            false
        }
    }

    #[pymethod]
    fn startswith(
        &self,
        prefix: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &VirtualMachine,
    ) -> bool {
        if let Some((start, end)) = adjust_indices(start, end, self.value.len()) {
            self.value[start..end].starts_with(&prefix.value)
        } else {
            false
        }
    }

    #[pymethod]
    fn isalnum(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_alphanumeric)
    }

    #[pymethod]
    fn isnumeric(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_numeric)
    }

    #[pymethod]
    fn isdigit(&self, _vm: &VirtualMachine) -> bool {
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
    fn isdecimal(&self, _vm: &VirtualMachine) -> bool {
        if self.value.is_empty() {
            false
        } else {
            self.value.chars().all(|c| c.is_ascii_digit())
        }
    }

    #[pymethod]
    fn format(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
        if args.args.is_empty() {
            return Err(vm.new_type_error(
                "descriptor 'format' of 'str' object needs an argument".to_string(),
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
        let format_string_text = get_value(zelf);
        match FormatString::from_str(format_string_text.as_str()) {
            Ok(format_string) => perform_format(vm, &format_string, &args),
            Err(err) => match err {
                FormatParseError::UnmatchedBracket => {
                    Err(vm.new_value_error("expected '}' before end of string".to_string()))
                }
                _ => Err(vm.new_value_error("Unexpected error parsing format string".to_string())),
            },
        }
    }

    /// Return a titlecased version of the string where words start with an
    /// uppercase character and the remaining characters are lowercase.
    #[pymethod]
    fn title(&self, _vm: &VirtualMachine) -> String {
        let mut title = String::new();
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
    fn swapcase(&self, _vm: &VirtualMachine) -> String {
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
    fn isalpha(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_alphanumeric)
    }

    #[pymethod]
    fn replace(
        &self,
        old: PyStringRef,
        new: PyStringRef,
        num: OptionalArg<usize>,
        _vm: &VirtualMachine,
    ) -> String {
        match num.into_option() {
            Some(num) => self.value.replacen(&old.value, &new.value, num),
            None => self.value.replace(&old.value, &new.value),
        }
    }

    // cpython's isspace ignores whitespace, including \t and \n, etc, unless the whole string is empty
    // which is why isspace is using is_ascii_whitespace. Same for isupper & islower
    #[pymethod]
    fn isspace(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii_whitespace())
    }

    #[pymethod]
    fn isupper(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty()
            && self
                .value
                .chars()
                .filter(|x| !x.is_ascii_whitespace())
                .all(char::is_uppercase)
    }

    #[pymethod]
    fn islower(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty()
            && self
                .value
                .chars()
                .filter(|x| !x.is_ascii_whitespace())
                .all(char::is_lowercase)
    }

    #[pymethod]
    fn isascii(&self, _vm: &VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii())
    }

    // doesn't implement keep new line delimiter just yet
    #[pymethod]
    fn splitlines(&self, vm: &VirtualMachine) -> PyObjectRef {
        let elements = self
            .value
            .split('\n')
            .map(|e| vm.ctx.new_str(e.to_string()))
            .collect();
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

    #[pymethod]
    fn find(
        &self,
        sub: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &VirtualMachine,
    ) -> isize {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            match value[start..end].find(&sub.value) {
                Some(num) => (start + num) as isize,
                None => -1 as isize,
            }
        } else {
            -1 as isize
        }
    }

    #[pymethod]
    fn rfind(
        &self,
        sub: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &VirtualMachine,
    ) -> isize {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            match value[start..end].rfind(&sub.value) {
                Some(num) => (start + num) as isize,
                None => -1 as isize,
            }
        } else {
            -1 as isize
        }
    }

    #[pymethod]
    fn index(
        &self,
        sub: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            match value[start..end].find(&sub.value) {
                Some(num) => Ok(start + num),
                None => Err(vm.new_value_error("substring not found".to_string())),
            }
        } else {
            Err(vm.new_value_error("substring not found".to_string()))
        }
    }

    #[pymethod]
    fn rindex(
        &self,
        sub: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            match value[start..end].rfind(&sub.value) {
                Some(num) => Ok(start + num),
                None => Err(vm.new_value_error("substring not found".to_string())),
            }
        } else {
            Err(vm.new_value_error("substring not found".to_string()))
        }
    }

    #[pymethod]
    fn partition(&self, sub: PyStringRef, vm: &VirtualMachine) -> PyObjectRef {
        let value = &self.value;
        let sub = &sub.value;
        let mut new_tup = Vec::new();
        if value.contains(sub) {
            new_tup = value
                .splitn(2, sub)
                .map(|s| vm.ctx.new_str(s.to_string()))
                .collect();
            new_tup.insert(1, vm.ctx.new_str(sub.clone()));
        } else {
            new_tup.push(vm.ctx.new_str(value.clone()));
            new_tup.push(vm.ctx.new_str("".to_string()));
            new_tup.push(vm.ctx.new_str("".to_string()));
        }
        vm.ctx.new_tuple(new_tup)
    }

    #[pymethod]
    fn rpartition(&self, sub: PyStringRef, vm: &VirtualMachine) -> PyObjectRef {
        let value = &self.value;
        let sub = &sub.value;
        let mut new_tup = Vec::new();
        if value.contains(sub) {
            new_tup = value
                .rsplitn(2, sub)
                .map(|s| vm.ctx.new_str(s.to_string()))
                .collect();
            new_tup.swap(0, 1); // so it's in the right order
            new_tup.insert(1, vm.ctx.new_str(sub.clone()));
        } else {
            new_tup.push(vm.ctx.new_str("".to_string()));
            new_tup.push(vm.ctx.new_str("".to_string()));
            new_tup.push(vm.ctx.new_str(value.clone()));
        }
        vm.ctx.new_tuple(new_tup)
    }

    /// Return `true` if the sequence is ASCII titlecase and the sequence is not
    /// empty, `false` otherwise.
    #[pymethod]
    fn istitle(&self, _vm: &VirtualMachine) -> bool {
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
    fn count(
        &self,
        sub: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &VirtualMachine,
    ) -> usize {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            self.value[start..end].matches(&sub.value).count()
        } else {
            0
        }
    }

    #[pymethod]
    fn zfill(&self, len: usize, _vm: &VirtualMachine) -> String {
        let value = &self.value;
        if len <= value.len() {
            value.to_string()
        } else {
            format!("{}{}", "0".repeat(len - value.len()), value)
        }
    }

    fn get_fill_char<'a>(
        rep: &'a OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<&'a str> {
        let rep_str = match rep {
            OptionalArg::Present(ref st) => &st.value,
            OptionalArg::Missing => " ",
        };
        if rep_str.len() == 1 {
            Ok(rep_str)
        } else {
            Err(vm.new_type_error(
                "The fill character must be exactly one character long".to_string(),
            ))
        }
    }

    #[pymethod]
    fn ljust(
        &self,
        len: usize,
        rep: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = Self::get_fill_char(&rep, vm)?;
        if len <= value.len() {
            Ok(value.to_string())
        } else {
            Ok(format!("{}{}", value, rep_char.repeat(len - value.len())))
        }
    }

    #[pymethod]
    fn rjust(
        &self,
        len: usize,
        rep: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = Self::get_fill_char(&rep, vm)?;
        if len <= value.len() {
            Ok(value.to_string())
        } else {
            Ok(format!("{}{}", rep_char.repeat(len - value.len()), value))
        }
    }

    #[pymethod]
    fn center(
        &self,
        len: usize,
        rep: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = Self::get_fill_char(&rep, vm)?;
        let value_len = self.value.chars().count();

        if len <= value_len {
            return Ok(value.to_string());
        }
        let diff: usize = len - value_len;
        let mut left_buff: usize = diff / 2;
        let mut right_buff: usize = left_buff;

        if diff % 2 != 0 && value_len % 2 == 0 {
            left_buff += 1
        }

        if diff % 2 != 0 && value_len % 2 != 0 {
            right_buff += 1
        }
        Ok(format!(
            "{}{}{}",
            rep_char.repeat(left_buff),
            value,
            rep_char.repeat(right_buff)
        ))
    }

    #[pymethod]
    fn expandtabs(&self, tab_stop: OptionalArg<usize>, _vm: &VirtualMachine) -> String {
        let tab_stop = tab_stop.into_option().unwrap_or(8 as usize);
        let mut expanded_str = String::new();
        let mut tab_size = tab_stop;
        let mut col_count = 0 as usize;
        for ch in self.value.chars() {
            // 0x0009 is tab
            if ch == 0x0009 as char {
                let num_spaces = tab_size - col_count;
                col_count += num_spaces;
                let expand = " ".repeat(num_spaces);
                expanded_str.push_str(&expand);
            } else {
                expanded_str.push(ch);
                col_count += 1;
            }
            if col_count >= tab_size {
                tab_size += tab_stop;
            }
        }
        expanded_str
    }

    #[pymethod]
    fn isidentifier(&self, _vm: &VirtualMachine) -> bool {
        let value = &self.value;
        // a string is not an identifier if it has whitespace or starts with a number
        if !value.chars().any(|c| c.is_ascii_whitespace())
            && !value.chars().nth(0).unwrap().is_digit(10)
        {
            for c in value.chars() {
                if c != "_".chars().nth(0).unwrap() && !c.is_digit(10) && !c.is_alphabetic() {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
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
        Ok(vm.ctx.new_str(self.to_string()))
    }
}

impl IntoPyObject for &String {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.clone()))
    }
}

pub fn init(ctx: &PyContext) {
    PyString::extend_class(ctx, &ctx.str_type);
}

pub fn get_value(obj: &PyObjectRef) -> String {
    obj.payload::<PyString>().unwrap().value.clone()
}

pub fn borrow_value(obj: &PyObjectRef) -> &str {
    &obj.payload::<PyString>().unwrap().value
}

fn count_char(s: &str, c: char) -> usize {
    s.chars().filter(|x| *x == c).count()
}

fn call_object_format(vm: &VirtualMachine, argument: PyObjectRef, format_spec: &str) -> PyResult {
    let returned_type = vm.ctx.new_str(format_spec.to_string());
    let result = vm.call_method(&argument, "__format__", vec![returned_type])?;
    if !objtype::isinstance(&result, &vm.ctx.str_type()) {
        let result_type = result.class();
        let actual_type = vm.to_pystr(&result_type)?;
        return Err(vm.new_type_error(format!("__format__ must return a str, not {}", actual_type)));
    }
    Ok(result)
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
            "cannot switch from automatic field numbering to manual field specification"
                .to_string(),
        ));
    }
    let mut auto_argument_index: usize = 1;
    for part in &format_string.format_parts {
        let result_string: String = match part {
            FormatPart::AutoSpec(format_spec) => {
                let result = match arguments.args.get(auto_argument_index) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_index_error("tuple index out of range".to_string()));
                    }
                };
                auto_argument_index += 1;
                get_value(&result)
            }
            FormatPart::IndexSpec(index, format_spec) => {
                let result = match arguments.args.get(*index + 1) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_index_error("tuple index out of range".to_string()));
                    }
                };
                get_value(&result)
            }
            FormatPart::KeywordSpec(keyword, format_spec) => {
                let result = match arguments.get_optional_kwarg(&keyword) {
                    Some(argument) => call_object_format(vm, argument.clone(), &format_spec)?,
                    None => {
                        return Err(vm.new_key_error(format!("'{}'", keyword)));
                    }
                };
                get_value(&result)
            }
            FormatPart::Literal(literal) => literal.clone(),
        };
        final_string.push_str(&result_string);
    }
    Ok(vm.ctx.new_str(final_string))
}

impl PySliceableSequence for String {
    fn do_slice(&self, range: Range<usize>) -> Self {
        to_graphemes(self)
            .get(range)
            .map_or(String::default(), |c| c.join(""))
    }

    fn do_slice_reverse(&self, range: Range<usize>) -> Self {
        to_graphemes(self)
            .get_mut(range)
            .map_or(String::default(), |slice| {
                slice.reverse();
                slice.join("")
            })
    }

    fn do_stepped_slice(&self, range: Range<usize>, step: usize) -> Self {
        if let Some(s) = to_graphemes(self).get(range) {
            return s
                .iter()
                .cloned()
                .step_by(step)
                .collect::<Vec<String>>()
                .join("");
        }
        String::default()
    }

    fn do_stepped_slice_reverse(&self, range: Range<usize>, step: usize) -> Self {
        if let Some(s) = to_graphemes(self).get(range) {
            return s
                .iter()
                .rev()
                .cloned()
                .step_by(step)
                .collect::<Vec<String>>()
                .join("");
        }
        String::default()
    }

    fn empty() -> Self {
        String::default()
    }

    fn len(&self) -> usize {
        to_graphemes(self).len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

/// Convert a string-able `value` to a vec of graphemes
/// represents the string according to user perceived characters
fn to_graphemes<S: AsRef<str>>(value: S) -> Vec<String> {
    UnicodeSegmentation::graphemes(value.as_ref(), true)
        .map(String::from)
        .collect()
}

pub fn subscript(vm: &VirtualMachine, value: &str, b: PyObjectRef) -> PyResult {
    if objtype::isinstance(&b, &vm.ctx.int_type()) {
        match objint::get_value(&b).to_i32() {
            Some(pos) => {
                let graphemes = to_graphemes(value);
                if let Some(idx) = graphemes.get_pos(pos) {
                    Ok(vm.new_str(graphemes[idx].to_string()))
                } else {
                    Err(vm.new_index_error("string index out of range".to_string()))
                }
            }
            None => {
                Err(vm.new_index_error("cannot fit 'int' into an index-sized integer".to_string()))
            }
        }
    } else if b.payload::<PySlice>().is_some() {
        let string = value.to_string().get_slice_items(vm, &b)?;
        Ok(vm.new_str(string))
    } else {
        panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            value, b
        )
    }
}

// help get optional string indices
fn adjust_indices(
    start: OptionalArg<isize>,
    end: OptionalArg<isize>,
    len: usize,
) -> Option<(usize, usize)> {
    let mut start = start.into_option().unwrap_or(0);
    let mut end = end.into_option().unwrap_or(len as isize);
    if end > len as isize {
        end = len as isize;
    } else if end < 0 {
        end += len as isize;
        if end < 0 {
            end = 0;
        }
    }
    if start < 0 {
        start += len as isize;
        if start < 0 {
            start = 0;
        }
    }
    if start > end {
        None
    } else {
        Some((start as usize, end as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_title() {
        let vm = VirtualMachine::new();

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
            assert_eq!(PyString::from(input).title(&vm).as_str(), title);
        }
    }

    #[test]
    fn str_istitle() {
        let vm = VirtualMachine::new();

        let pos = vec![
            "A",
            "A Titlecased Line",
            "A\nTitlecased Line",
            "A Titlecased, Line",
            "Greek Ωppercases ...",
            "Greek ῼitlecases ...",
        ];

        for s in pos {
            assert!(PyString::from(s).istitle(&vm));
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
            assert!(!PyString::from(s).istitle(&vm));
        }
    }
}

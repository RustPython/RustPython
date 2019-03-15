use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::str::FromStr;

use num_traits::ToPrimitive;
use unicode_segmentation::UnicodeSegmentation;

use crate::format::{FormatParseError, FormatPart, FormatString};
use crate::pyobject::{
    IdProtocol, IntoPyObject, OptionalArg, PyContext, PyFuncArgs, PyIterable, PyObjectRef, PyRef,
    PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

use super::objint;
use super::objsequence::PySliceableSequence;
use super::objslice::PySlice;
use super::objtype::{self, PyClassRef};

#[derive(Clone, Debug)]
pub struct PyString {
    // TODO: shouldn't be public
    pub value: String,
}

pub type PyStringRef = PyRef<PyString>;

impl PyStringRef {
    fn add(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<String> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(format!("{}{}", self.value, get_value(&rhs)))
        } else {
            Err(vm.new_type_error(format!("Cannot add {} and {}", self, rhs)))
        }
    }

    fn eq(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> bool {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            self.value == get_value(&rhs)
        } else {
            false
        }
    }

    fn contains(self, needle: PyStringRef, _vm: &mut VirtualMachine) -> bool {
        self.value.contains(&needle.value)
    }

    fn getitem(self, needle: PyObjectRef, vm: &mut VirtualMachine) -> PyResult {
        subscript(vm, &self.value, needle)
    }

    fn gt(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value > get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    fn ge(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value >= get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    fn lt(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value < get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    fn le(self, rhs: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<bool> {
        if objtype::isinstance(&rhs, &vm.ctx.str_type()) {
            Ok(self.value <= get_value(&rhs))
        } else {
            Err(vm.new_type_error(format!("Cannot compare {} and {}", self, rhs)))
        }
    }

    fn hash(self, _vm: &mut VirtualMachine) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.value.hash(&mut hasher);
        hasher.finish() as usize
    }

    fn len(self, _vm: &mut VirtualMachine) -> usize {
        self.value.chars().count()
    }

    fn mul(self, val: PyObjectRef, vm: &mut VirtualMachine) -> PyResult<String> {
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

    fn str(self, _vm: &mut VirtualMachine) -> PyStringRef {
        self
    }

    fn repr(self, _vm: &mut VirtualMachine) -> String {
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

    fn lower(self, _vm: &mut VirtualMachine) -> String {
        self.value.to_lowercase()
    }

    // casefold is much more aggressive than lower
    fn casefold(self, _vm: &mut VirtualMachine) -> String {
        caseless::default_case_fold_str(&self.value)
    }

    fn upper(self, _vm: &mut VirtualMachine) -> String {
        self.value.to_uppercase()
    }

    fn capitalize(self, _vm: &mut VirtualMachine) -> String {
        let (first_part, lower_str) = self.value.split_at(1);
        format!("{}{}", first_part.to_uppercase(), lower_str)
    }

    fn split(
        self,
        pattern: OptionalArg<Self>,
        num: OptionalArg<usize>,
        vm: &mut VirtualMachine,
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

    fn rsplit(
        self,
        pattern: OptionalArg<Self>,
        num: OptionalArg<usize>,
        vm: &mut VirtualMachine,
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

    fn strip(self, _vm: &mut VirtualMachine) -> String {
        self.value.trim().to_string()
    }

    fn lstrip(self, _vm: &mut VirtualMachine) -> String {
        self.value.trim_start().to_string()
    }

    fn rstrip(self, _vm: &mut VirtualMachine) -> String {
        self.value.trim_end().to_string()
    }

    fn endswith(
        self,
        suffix: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &mut VirtualMachine,
    ) -> bool {
        if let Some((start, end)) = adjust_indices(start, end, self.value.len()) {
            self.value[start..end].ends_with(&suffix.value)
        } else {
            false
        }
    }

    fn startswith(
        self,
        prefix: PyStringRef,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &mut VirtualMachine,
    ) -> bool {
        if let Some((start, end)) = adjust_indices(start, end, self.value.len()) {
            self.value[start..end].starts_with(&prefix.value)
        } else {
            false
        }
    }

    fn isalnum(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_alphanumeric)
    }

    fn isnumeric(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_numeric)
    }

    fn isdigit(self, _vm: &mut VirtualMachine) -> bool {
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

    fn isdecimal(self, _vm: &mut VirtualMachine) -> bool {
        if self.value.is_empty() {
            false
        } else {
            self.value.chars().all(|c| c.is_ascii_digit())
        }
    }

    fn title(self, _vm: &mut VirtualMachine) -> String {
        make_title(&self.value)
    }

    fn swapcase(self, _vm: &mut VirtualMachine) -> String {
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

    fn isalpha(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(char::is_alphanumeric)
    }

    fn replace(
        self,
        old: Self,
        new: Self,
        num: OptionalArg<usize>,
        _vm: &mut VirtualMachine,
    ) -> String {
        match num.into_option() {
            Some(num) => self.value.replacen(&old.value, &new.value, num),
            None => self.value.replace(&old.value, &new.value),
        }
    }

    // cpython's isspace ignores whitespace, including \t and \n, etc, unless the whole string is empty
    // which is why isspace is using is_ascii_whitespace. Same for isupper & islower
    fn isspace(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii_whitespace())
    }

    fn isupper(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty()
            && self
                .value
                .chars()
                .filter(|x| !x.is_ascii_whitespace())
                .all(char::is_uppercase)
    }

    fn islower(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty()
            && self
                .value
                .chars()
                .filter(|x| !x.is_ascii_whitespace())
                .all(char::is_lowercase)
    }

    fn isascii(self, _vm: &mut VirtualMachine) -> bool {
        !self.value.is_empty() && self.value.chars().all(|c| c.is_ascii())
    }

    // doesn't implement keep new line delimiter just yet
    fn splitlines(self, vm: &mut VirtualMachine) -> PyObjectRef {
        let elements = self
            .value
            .split('\n')
            .map(|e| vm.ctx.new_str(e.to_string()))
            .collect();
        vm.ctx.new_list(elements)
    }

    fn join(self, iterable: PyIterable<PyStringRef>, vm: &mut VirtualMachine) -> PyResult<String> {
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

    fn find(
        self,
        sub: Self,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &mut VirtualMachine,
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

    fn rfind(
        self,
        sub: Self,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &mut VirtualMachine,
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

    fn index(
        self,
        sub: Self,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        vm: &mut VirtualMachine,
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

    fn rindex(
        self,
        sub: Self,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        vm: &mut VirtualMachine,
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

    fn partition(self, sub: PyStringRef, vm: &mut VirtualMachine) -> PyObjectRef {
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

    fn rpartition(self, sub: PyStringRef, vm: &mut VirtualMachine) -> PyObjectRef {
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
            new_tup.push(vm.ctx.new_str(value.clone()));
            new_tup.push(vm.ctx.new_str("".to_string()));
            new_tup.push(vm.ctx.new_str("".to_string()));
        }
        vm.ctx.new_tuple(new_tup)
    }

    fn istitle(self, _vm: &mut VirtualMachine) -> bool {
        if self.value.is_empty() {
            false
        } else {
            self.value.split(' ').all(|word| word == make_title(word))
        }
    }

    fn count(
        self,
        sub: Self,
        start: OptionalArg<isize>,
        end: OptionalArg<isize>,
        _vm: &mut VirtualMachine,
    ) -> usize {
        let value = &self.value;
        if let Some((start, end)) = adjust_indices(start, end, value.len()) {
            self.value[start..end].matches(&sub.value).count()
        } else {
            0
        }
    }

    fn zfill(self, len: usize, _vm: &mut VirtualMachine) -> String {
        let value = &self.value;
        if len <= value.len() {
            value.to_string()
        } else {
            format!("{}{}", "0".repeat(len - value.len()), value)
        }
    }

    fn get_fill_char<'a>(rep: &'a OptionalArg<Self>, vm: &mut VirtualMachine) -> PyResult<&'a str> {
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

    fn ljust(
        self,
        len: usize,
        rep: OptionalArg<Self>,
        vm: &mut VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = PyStringRef::get_fill_char(&rep, vm)?;
        Ok(format!("{}{}", value, rep_char.repeat(len)))
    }

    fn rjust(
        self,
        len: usize,
        rep: OptionalArg<Self>,
        vm: &mut VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = PyStringRef::get_fill_char(&rep, vm)?;
        Ok(format!("{}{}", rep_char.repeat(len), value))
    }

    fn center(
        self,
        len: usize,
        rep: OptionalArg<Self>,
        vm: &mut VirtualMachine,
    ) -> PyResult<String> {
        let value = &self.value;
        let rep_char = PyStringRef::get_fill_char(&rep, vm)?;
        let left_buff: usize = (len - value.len()) / 2;
        let right_buff = len - value.len() - left_buff;
        Ok(format!(
            "{}{}{}",
            rep_char.repeat(left_buff),
            value,
            rep_char.repeat(right_buff)
        ))
    }

    fn expandtabs(self, tab_stop: OptionalArg<usize>, _vm: &mut VirtualMachine) -> String {
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

    fn isidentifier(self, _vm: &mut VirtualMachine) -> bool {
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
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.str_type()
    }
}

impl IntoPyObject for String {
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self))
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    let str_type = &context.str_type;
    let str_doc = "str(object='') -> str\n\
                   str(bytes_or_buffer[, encoding[, errors]]) -> str\n\
                   \n\
                   Create a new string object from the given object. If encoding or\n\
                   errors is specified, then the object must expose a data buffer\n\
                   that will be decoded using the given encoding and error handler.\n\
                   Otherwise, returns the result of object.__str__() (if defined)\n\
                   or repr(object).\n\
                   encoding defaults to sys.getdefaultencoding().\n\
                   errors defaults to 'strict'.";
    context.set_attr(&str_type, "__add__", context.new_rustfunc(PyStringRef::add));
    context.set_attr(&str_type, "__eq__", context.new_rustfunc(PyStringRef::eq));
    context.set_attr(&str_type, "__contains__", context.new_rustfunc(PyStringRef::contains));
    context.set_attr(&str_type, "__getitem__", context.new_rustfunc(PyStringRef::getitem));
    context.set_attr(&str_type, "__gt__", context.new_rustfunc(PyStringRef::gt));
    context.set_attr(&str_type, "__ge__", context.new_rustfunc(PyStringRef::ge));
    context.set_attr(&str_type, "__lt__", context.new_rustfunc(PyStringRef::lt));
    context.set_attr(&str_type, "__le__", context.new_rustfunc(PyStringRef::le));
    context.set_attr(&str_type, "__hash__", context.new_rustfunc(PyStringRef::hash));
    context.set_attr(&str_type, "__len__", context.new_rustfunc(PyStringRef::len));
    context.set_attr(&str_type, "__mul__", context.new_rustfunc(PyStringRef::mul));
    context.set_attr(&str_type, "__new__", context.new_rustfunc(str_new));
    context.set_attr(&str_type, "__str__", context.new_rustfunc(PyStringRef::str));
    context.set_attr(&str_type, "__repr__", context.new_rustfunc(PyStringRef::repr));
    context.set_attr(&str_type, "format", context.new_rustfunc(str_format));
    context.set_attr(&str_type, "lower", context.new_rustfunc(PyStringRef::lower));
    context.set_attr(&str_type, "casefold", context.new_rustfunc(PyStringRef::casefold));
    context.set_attr(&str_type, "upper", context.new_rustfunc(PyStringRef::upper));
    context.set_attr(&str_type, "capitalize", context.new_rustfunc(PyStringRef::capitalize));
    context.set_attr(&str_type, "split", context.new_rustfunc(PyStringRef::split));
    context.set_attr(&str_type, "rsplit", context.new_rustfunc(PyStringRef::rsplit));
    context.set_attr(&str_type, "strip", context.new_rustfunc(PyStringRef::strip));
    context.set_attr(&str_type, "lstrip", context.new_rustfunc(PyStringRef::lstrip));
    context.set_attr(&str_type, "rstrip", context.new_rustfunc(PyStringRef::rstrip));
    context.set_attr(&str_type, "endswith", context.new_rustfunc(PyStringRef::endswith));
    context.set_attr(&str_type, "startswith", context.new_rustfunc(PyStringRef::startswith));
    context.set_attr(&str_type, "isalnum", context.new_rustfunc(PyStringRef::isalnum));
    context.set_attr(&str_type, "isnumeric", context.new_rustfunc(PyStringRef::isnumeric));
    context.set_attr(&str_type, "isdigit", context.new_rustfunc(PyStringRef::isdigit));
    context.set_attr(&str_type, "isdecimal", context.new_rustfunc(PyStringRef::isdecimal));
    context.set_attr(&str_type, "title", context.new_rustfunc(PyStringRef::title));
    context.set_attr(&str_type, "swapcase", context.new_rustfunc(PyStringRef::swapcase));
    context.set_attr(&str_type, "isalpha", context.new_rustfunc(PyStringRef::isalpha));
    context.set_attr(&str_type, "replace", context.new_rustfunc(PyStringRef::replace));
    context.set_attr(&str_type, "isspace", context.new_rustfunc(PyStringRef::isspace));
    context.set_attr(&str_type, "isupper", context.new_rustfunc(PyStringRef::isupper));
    context.set_attr(&str_type, "islower", context.new_rustfunc(PyStringRef::islower));
    context.set_attr(&str_type, "isascii", context.new_rustfunc(PyStringRef::isascii));
    context.set_attr(&str_type, "splitlines", context.new_rustfunc(PyStringRef::splitlines));
    context.set_attr(&str_type, "join", context.new_rustfunc(PyStringRef::join));
    context.set_attr(&str_type, "find", context.new_rustfunc(PyStringRef::find));
    context.set_attr(&str_type, "rfind", context.new_rustfunc(PyStringRef::rfind));
    context.set_attr(&str_type, "index", context.new_rustfunc(PyStringRef::index));
    context.set_attr(&str_type, "rindex", context.new_rustfunc(PyStringRef::rindex));
    context.set_attr(&str_type, "partition", context.new_rustfunc(PyStringRef::partition));
    context.set_attr(&str_type, "rpartition", context.new_rustfunc(PyStringRef::rpartition));
    context.set_attr(&str_type, "istitle", context.new_rustfunc(PyStringRef::istitle));
    context.set_attr(&str_type, "count", context.new_rustfunc(PyStringRef::count));
    context.set_attr(&str_type, "zfill", context.new_rustfunc(PyStringRef::zfill));
    context.set_attr(&str_type, "ljust", context.new_rustfunc(PyStringRef::ljust));
    context.set_attr(&str_type, "rjust", context.new_rustfunc(PyStringRef::rjust));
    context.set_attr(&str_type, "center", context.new_rustfunc(PyStringRef::center));
    context.set_attr(&str_type, "expandtabs", context.new_rustfunc(PyStringRef::expandtabs));
    context.set_attr(&str_type, "isidentifier", context.new_rustfunc(PyStringRef::isidentifier));
    context.set_attr(&str_type, "__doc__", context.new_str(str_doc.to_string()));
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

fn str_format(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        return Err(
            vm.new_type_error("descriptor 'format' of 'str' object needs an argument".to_string())
        );
    }

    let zelf = &args.args[0];
    if !objtype::isinstance(&zelf, &vm.ctx.str_type()) {
        let zelf_typ = zelf.typ();
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

fn call_object_format(
    vm: &mut VirtualMachine,
    argument: PyObjectRef,
    format_spec: &str,
) -> PyResult {
    let returned_type = vm.ctx.new_str(format_spec.to_string());
    let result = vm.call_method(&argument, "__format__", vec![returned_type])?;
    if !objtype::isinstance(&result, &vm.ctx.str_type()) {
        let result_type = result.typ();
        let actual_type = vm.to_pystr(&result_type)?;
        return Err(vm.new_type_error(format!("__format__ must return a str, not {}", actual_type)));
    }
    Ok(result)
}

fn perform_format(
    vm: &mut VirtualMachine,
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

// TODO: should with following format
// class str(object='')
// class str(object=b'', encoding='utf-8', errors='strict')
fn str_new(
    cls: PyClassRef,
    object: OptionalArg<PyObjectRef>,
    vm: &mut VirtualMachine,
) -> PyResult<PyStringRef> {
    let string = match object {
        OptionalArg::Present(ref input) => vm.to_str(input)?.into_object(),
        OptionalArg::Missing => vm.new_str("".to_string()),
    };
    if string.typ().is(&cls) {
        TryFromObject::try_from_object(vm, string)
    } else {
        let payload = string.payload::<PyString>().unwrap();
        PyRef::new_with_type(vm, payload.clone(), cls)
    }
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

pub fn subscript(vm: &mut VirtualMachine, value: &str, b: PyObjectRef) -> PyResult {
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

// helper function to title strings
fn make_title(s: &str) -> String {
    let mut titled_str = String::new();
    let mut capitalize_char: bool = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if !capitalize_char {
                titled_str.push(c);
            } else if capitalize_char {
                titled_str.push(c.to_ascii_uppercase());
                capitalize_char = false;
            }
        } else {
            titled_str.push(c);
            capitalize_char = true;
        }
    }
    titled_str
}

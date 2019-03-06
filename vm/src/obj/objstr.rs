use super::objint;
use super::objsequence::PySliceableSequence;
use super::objtype;
use crate::format::{FormatParseError, FormatPart, FormatString};
use crate::function::PyRef;
use crate::pyobject::{
    IntoPyObject, OptArg, PyContext, PyFuncArgs, PyIterable, PyObjectPayload, PyObjectPayload2,
    PyObjectRef, PyResult, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_traits::ToPrimitive;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::str::FromStr;
// rust's builtin to_lowercase isn't sufficient for casefold
extern crate caseless;
extern crate unicode_segmentation;

use self::unicode_segmentation::UnicodeSegmentation;

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
        start: OptArg<usize>,
        end: OptArg<usize>,
        _vm: &mut VirtualMachine,
    ) -> bool {
        let start = start.unwrap_or(0);
        let end = end.unwrap_or(self.value.len());
        self.value[start..end].ends_with(&suffix.value)
    }

    fn startswith(
        self,
        prefix: PyStringRef,
        start: OptArg<usize>,
        end: OptArg<usize>,
        _vm: &mut VirtualMachine,
    ) -> bool {
        let start = start.unwrap_or(0);
        let end = end.unwrap_or(self.value.len());
        self.value[start..end].starts_with(&prefix.value)
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

impl PyObjectPayload2 for PyString {
    fn required_type(ctx: &PyContext) -> PyObjectRef {
        ctx.str_type()
    }
}

impl IntoPyObject for String {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(ctx.new_str(self))
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    let str_type = &context.str_type;
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
    context.set_attr(&str_type, "__mul__", context.new_rustfunc(str_mul));
    context.set_attr(&str_type, "__new__", context.new_rustfunc(str_new));
    context.set_attr(&str_type, "__str__", context.new_rustfunc(PyStringRef::str));
    context.set_attr(&str_type, "__repr__", context.new_rustfunc(PyStringRef::repr));
    context.set_attr(&str_type, "format", context.new_rustfunc(str_format));
    context.set_attr(&str_type, "lower", context.new_rustfunc(PyStringRef::lower));
    context.set_attr(&str_type, "casefold", context.new_rustfunc(PyStringRef::casefold));
    context.set_attr(&str_type, "upper", context.new_rustfunc(PyStringRef::upper));
    context.set_attr(&str_type, "capitalize", context.new_rustfunc(PyStringRef::capitalize));
    context.set_attr(&str_type, "split", context.new_rustfunc(str_split));
    context.set_attr(&str_type, "rsplit", context.new_rustfunc(str_rsplit));
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
    context.set_attr(&str_type, "replace", context.new_rustfunc(str_replace));
    context.set_attr(&str_type, "center", context.new_rustfunc(str_center));
    context.set_attr(&str_type, "isspace", context.new_rustfunc(PyStringRef::isspace));
    context.set_attr(&str_type, "isupper", context.new_rustfunc(PyStringRef::isupper));
    context.set_attr(&str_type, "islower", context.new_rustfunc(PyStringRef::islower));
    context.set_attr(&str_type, "isascii", context.new_rustfunc(PyStringRef::isascii));
    context.set_attr(&str_type, "splitlines", context.new_rustfunc(PyStringRef::splitlines));
    context.set_attr(&str_type, "join", context.new_rustfunc(PyStringRef::join));
    context.set_attr(&str_type, "find", context.new_rustfunc(str_find));
    context.set_attr(&str_type, "rfind", context.new_rustfunc(str_rfind));
    context.set_attr(&str_type, "index", context.new_rustfunc(str_index));
    context.set_attr(&str_type, "rindex", context.new_rustfunc(str_rindex));
    context.set_attr(&str_type, "partition", context.new_rustfunc(PyStringRef::partition));
    context.set_attr(&str_type, "rpartition", context.new_rustfunc(PyStringRef::rpartition));
    context.set_attr(&str_type, "istitle", context.new_rustfunc(str_istitle));
    context.set_attr(&str_type, "count", context.new_rustfunc(str_count));
    context.set_attr(&str_type, "zfill", context.new_rustfunc(str_zfill));
    context.set_attr(&str_type, "ljust", context.new_rustfunc(str_ljust));
    context.set_attr(&str_type, "rjust", context.new_rustfunc(str_rjust));
    context.set_attr(&str_type, "expandtabs", context.new_rustfunc(str_expandtabs));
    context.set_attr(&str_type, "isidentifier", context.new_rustfunc(PyStringRef::isidentifier));
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

fn str_mul(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (s2, None)]
    );
    if objtype::isinstance(s2, &vm.ctx.int_type()) {
        let value1 = get_value(&s);
        let value2 = objint::get_value(s2).to_i32().unwrap();
        let mut result = String::new();
        for _x in 0..value2 {
            result.push_str(value1.as_str());
        }
        Ok(vm.ctx.new_str(result))
    } else {
        Err(vm.new_type_error(format!("Cannot multiply {} and {}", s, s2)))
    }
}

fn str_rsplit(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type()))],
        optional = [
            (pat, Some(vm.ctx.str_type())),
            (num, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let pat = match pat {
        Some(s) => get_value(&s),
        None => " ".to_string(),
    };
    let num_splits = match num {
        Some(n) => objint::get_value(&n).to_usize().unwrap(),
        None => value.split(&pat).count(),
    };
    let elements = value
        .rsplitn(num_splits + 1, &pat)
        .map(|o| vm.ctx.new_str(o.to_string()))
        .collect();
    Ok(vm.ctx.new_list(elements))
}

fn str_split(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type()))],
        optional = [
            (pat, Some(vm.ctx.str_type())),
            (num, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let pat = match pat {
        Some(s) => get_value(&s),
        None => " ".to_string(),
    };
    let num_splits = match num {
        Some(n) => objint::get_value(&n).to_usize().unwrap(),
        None => value.split(&pat).count(),
    };
    let elements = value
        .splitn(num_splits + 1, &pat)
        .map(|o| vm.ctx.new_str(o.to_string()))
        .collect();
    Ok(vm.ctx.new_list(elements))
}

fn str_zfill(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (len, Some(vm.ctx.int_type()))]
    );
    let value = get_value(&s);
    let len = objint::get_value(&len).to_usize().unwrap();
    let new_str = if len <= value.len() {
        value
    } else {
        format!("{}{}", "0".repeat(len - value.len()), value)
    };
    Ok(vm.ctx.new_str(new_str))
}

fn str_count(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (sub, Some(vm.ctx.str_type()))],
        optional = [
            (start, Some(vm.ctx.int_type())),
            (end, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let sub = get_value(&sub);
    let (start, end) = match get_slice(start, end, value.len()) {
        Ok((start, end)) => (start, end),
        Err(e) => return Err(vm.new_index_error(e)),
    };
    let num_occur: usize = value[start..end].matches(&sub).count();
    Ok(vm.ctx.new_int(num_occur))
}

fn str_index(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (sub, Some(vm.ctx.str_type()))],
        optional = [
            (start, Some(vm.ctx.int_type())),
            (end, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let sub = get_value(&sub);
    let (start, end) = match get_slice(start, end, value.len()) {
        Ok((start, end)) => (start, end),
        Err(e) => return Err(vm.new_index_error(e)),
    };
    let ind: usize = match value[start..=end].find(&sub) {
        Some(num) => num,
        None => {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
    };
    Ok(vm.ctx.new_int(ind))
}

fn str_find(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (sub, Some(vm.ctx.str_type()))],
        optional = [
            (start, Some(vm.ctx.int_type())),
            (end, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let sub = get_value(&sub);
    let (start, end) = match get_slice(start, end, value.len()) {
        Ok((start, end)) => (start, end),
        Err(e) => return Err(vm.new_index_error(e)),
    };
    let ind: i128 = match value[start..=end].find(&sub) {
        Some(num) => num as i128,
        None => -1 as i128,
    };
    Ok(vm.ctx.new_int(ind))
}

fn str_replace(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (s, Some(vm.ctx.str_type())),
            (old, Some(vm.ctx.str_type())),
            (rep, Some(vm.ctx.str_type()))
        ],
        optional = [(n, Some(vm.ctx.int_type()))]
    );
    let s = get_value(&s);
    let old_str = get_value(&old);
    let rep_str = get_value(&rep);
    let num_rep: usize = match n {
        Some(num) => objint::get_value(&num).to_usize().unwrap(),
        None => 1,
    };
    let new_str = s.replacen(&old_str, &rep_str, num_rep);
    Ok(vm.ctx.new_str(new_str))
}

fn str_expandtabs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type()))],
        optional = [(size, Some(vm.ctx.int_type()))]
    );
    let value = get_value(&s);
    let tab_stop = match size {
        Some(num) => objint::get_value(&num).to_usize().unwrap(),
        None => 8 as usize,
    };
    let mut expanded_str = String::new();
    let mut tab_size = tab_stop;
    let mut col_count = 0 as usize;
    for ch in value.chars() {
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
    Ok(vm.ctx.new_str(expanded_str))
}

fn str_rjust(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (num, Some(vm.ctx.int_type()))],
        optional = [(rep, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    let num = objint::get_value(&num).to_usize().unwrap();
    let rep = match rep {
        Some(st) => {
            let rep_str = get_value(&st);
            if rep_str.len() == 1 {
                rep_str
            } else {
                return Err(vm.new_type_error(
                    "The fill character must be exactly one character long".to_string(),
                ));
            }
        }
        None => " ".to_string(),
    };
    let new_str = format!("{}{}", rep.repeat(num), value);
    Ok(vm.ctx.new_str(new_str))
}

fn str_ljust(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (num, Some(vm.ctx.int_type()))],
        optional = [(rep, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    let num = objint::get_value(&num).to_usize().unwrap();
    let rep = match rep {
        Some(st) => {
            let rep_str = get_value(&st);
            if rep_str.len() == 1 {
                rep_str
            } else {
                return Err(vm.new_type_error(
                    "The fill character must be exactly one character long".to_string(),
                ));
            }
        }
        None => " ".to_string(),
    };
    let new_str = format!("{}{}", value, rep.repeat(num));
    Ok(vm.ctx.new_str(new_str))
}

fn str_istitle(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s);

    let is_titled = if value.is_empty() {
        false
    } else {
        value.split(' ').all(|word| word == make_title(word))
    };

    Ok(vm.ctx.new_bool(is_titled))
}

fn str_center(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (len, Some(vm.ctx.int_type()))],
        optional = [(chars, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    let len = objint::get_value(&len).to_usize().unwrap();
    let rep_char = match chars {
        Some(c) => get_value(&c),
        None => " ".to_string(),
    };
    let left_buff: usize = (len - value.len()) / 2;
    let right_buff = len - value.len() - left_buff;
    let new_str = format!(
        "{}{}{}",
        rep_char.repeat(left_buff),
        value,
        rep_char.repeat(right_buff)
    );
    Ok(vm.ctx.new_str(new_str))
}

fn str_rindex(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (sub, Some(vm.ctx.str_type()))],
        optional = [
            (start, Some(vm.ctx.int_type())),
            (end, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let sub = get_value(&sub);
    let (start, end) = match get_slice(start, end, value.len()) {
        Ok((start, end)) => (start, end),
        Err(e) => return Err(vm.new_index_error(e)),
    };
    let ind: i64 = match value[start..=end].rfind(&sub) {
        Some(num) => num as i64,
        None => {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
    };
    Ok(vm.ctx.new_int(ind))
}

fn str_rfind(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (sub, Some(vm.ctx.str_type()))],
        optional = [
            (start, Some(vm.ctx.int_type())),
            (end, Some(vm.ctx.int_type()))
        ]
    );
    let value = get_value(&s);
    let sub = get_value(&sub);
    let (start, end) = match get_slice(start, end, value.len()) {
        Ok((start, end)) => (start, end),
        Err(e) => return Err(vm.new_index_error(e)),
    };
    let ind = match value[start..=end].rfind(&sub) {
        Some(num) => num as i128,
        None => -1 as i128,
    };
    Ok(vm.ctx.new_int(ind))
}

// TODO: should with following format
// class str(object='')
// class str(object=b'', encoding='utf-8', errors='strict')
fn str_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() == 1 {
        return Ok(vm.new_str("".to_string()));
    }

    if args.args.len() > 2 {
        panic!("str expects exactly one parameter");
    };

    vm.to_str(&args.args[1])
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
    } else {
        match b.payload {
            PyObjectPayload::Slice { .. } => {
                let string = value.to_string().get_slice_items(vm, &b)?;
                Ok(vm.new_str(string))
            }
            _ => panic!(
                "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
                value, b
            ),
        }
    }
}

// help get optional string indices
fn get_slice(
    start: Option<&PyObjectRef>,
    end: Option<&PyObjectRef>,
    len: usize,
) -> Result<(usize, usize), String> {
    let start_idx = match start {
        Some(int) => objint::get_value(&int).to_usize().unwrap(),
        None => 0 as usize,
    };
    let end_idx = match end {
        Some(int) => objint::get_value(&int).to_usize().unwrap(),
        None => len - 1,
    };
    if start_idx >= usize::min_value() && start_idx < end_idx && end_idx < len {
        Ok((start_idx, end_idx))
    } else {
        Err("provided index is not valid".to_string())
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

use super::super::format::{FormatParseError, FormatPart, FormatString};
use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objsequence::PySliceableSequence;
use super::objtype;
use num_bigint::ToBigInt;
use num_traits::ToPrimitive;
use std::hash::{Hash, Hasher};

pub fn init(context: &PyContext) {
    let ref str_type = context.str_type;
    context.set_attr(&str_type, "__add__", context.new_rustfunc(str_add));
    context.set_attr(&str_type, "__eq__", context.new_rustfunc(str_eq));
    context.set_attr(
        &str_type,
        "__contains__",
        context.new_rustfunc(str_contains),
    );
    context.set_attr(&str_type, "__getitem__", context.new_rustfunc(str_getitem));
    context.set_attr(&str_type, "__gt__", context.new_rustfunc(str_gt));
    context.set_attr(&str_type, "__hash__", context.new_rustfunc(str_hash));
    context.set_attr(&str_type, "__len__", context.new_rustfunc(str_len));
    context.set_attr(&str_type, "__mul__", context.new_rustfunc(str_mul));
    context.set_attr(&str_type, "__new__", context.new_rustfunc(str_new));
    context.set_attr(&str_type, "__str__", context.new_rustfunc(str_str));
    context.set_attr(&str_type, "__repr__", context.new_rustfunc(str_repr));
    context.set_attr(&str_type, "format", context.new_rustfunc(str_format));
    context.set_attr(&str_type, "lower", context.new_rustfunc(str_lower));
    context.set_attr(&str_type, "upper", context.new_rustfunc(str_upper));
    context.set_attr(
        &str_type,
        "capitalize",
        context.new_rustfunc(str_capitalize),
    );
    context.set_attr(&str_type, "split", context.new_rustfunc(str_split));
    context.set_attr(&str_type, "strip", context.new_rustfunc(str_strip));
    context.set_attr(&str_type, "lstrip", context.new_rustfunc(str_lstrip));
    context.set_attr(&str_type, "rstrip", context.new_rustfunc(str_rstrip));
    context.set_attr(&str_type, "endswith", context.new_rustfunc(str_endswith));
    context.set_attr(
        &str_type,
        "startswith",
        context.new_rustfunc(str_startswith),
    );
    context.set_attr(&str_type, "isalnum", context.new_rustfunc(str_isalnum));
    context.set_attr(&str_type, "isnumeric", context.new_rustfunc(str_isnumeric));
    context.set_attr(&str_type, "isdigit", context.new_rustfunc(str_isdigit));
    context.set_attr(&str_type, "title", context.new_rustfunc(str_title));
    context.set_attr(&str_type, "swapcase", context.new_rustfunc(str_swapcase));
    context.set_attr(&str_type, "isalpha", context.new_rustfunc(str_isalpha));
    context.set_attr(&str_type, "replace", context.new_rustfunc(str_replace));
    context.set_attr(&str_type, "center", context.new_rustfunc(str_center));
}

pub fn get_value(obj: &PyObjectRef) -> String {
    if let PyObjectKind::String { value } = &obj.borrow().kind {
        value.to_string()
    } else {
        panic!("Inner error getting str");
    }
}

fn str_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(a, Some(vm.ctx.str_type())), (b, None)]
    );

    let result = if objtype::isinstance(b, &vm.ctx.str_type()) {
        get_value(a) == get_value(b)
    } else {
        false
    };
    Ok(vm.ctx.new_bool(result))
}

fn str_gt(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (zelf, Some(vm.ctx.str_type())),
            (other, Some(vm.ctx.str_type()))
        ]
    );
    let zelf = get_value(zelf);
    let other = get_value(other);
    let result = zelf > other;
    Ok(vm.ctx.new_bool(result))
}

fn str_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    Ok(s.clone())
}

fn count_char(s: &str, c: char) -> usize {
    s.chars().filter(|x| *x == c).count()
}

fn str_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(s);
    let quote_char = if count_char(&value, '\'') > count_char(&value, '"') {
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
    Ok(vm.ctx.new_str(formatted))
}

fn str_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (s2, None)]
    );
    if objtype::isinstance(s2, &vm.ctx.str_type()) {
        Ok(vm
            .ctx
            .new_str(format!("{}{}", get_value(&s), get_value(&s2))))
    } else {
        Err(vm.new_type_error(format!("Cannot add {:?} and {:?}", s, s2)))
    }
}

fn str_format(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() == 0 {
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
    format_spec: &String,
) -> PyResult {
    let returned_type = vm.ctx.new_str(format_spec.clone());
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

fn str_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.str_type()))]);
    let value = get_value(zelf);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(vm.ctx.new_int(hash.to_bigint().unwrap()))
}

fn str_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let sv = get_value(s);
    Ok(vm.ctx.new_int(sv.len().to_bigint().unwrap()))
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
        Err(vm.new_type_error(format!("Cannot multiply {:?} and {:?}", s, s2)))
    }
}

fn str_upper(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s).to_uppercase();
    Ok(vm.ctx.new_str(value))
}

fn str_lower(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s).to_lowercase();
    Ok(vm.ctx.new_str(value))
}

fn str_capitalize(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s);
    let (first_part, lower_str) = value.split_at(1);
    let capitalized = format!("{}{}", first_part.to_uppercase().to_string(), lower_str);
    Ok(vm.ctx.new_str(capitalized))
}

fn str_split(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (pat, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    // if some
    let pat = get_value(&pat);
    let str_pat = pat.as_str();
    let elements = value
        .split(str_pat)
        .map(|o| vm.ctx.new_str(o.to_string()))
        .collect();
    Ok(vm.ctx.new_list(elements))
}

fn str_strip(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s).trim().to_string();
    Ok(vm.ctx.new_str(value))
}

fn str_lstrip(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s).trim_left().to_string();
    Ok(vm.ctx.new_str(value))
}

fn str_rstrip(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s).trim_right().to_string();
    Ok(vm.ctx.new_str(value))
}

fn str_endswith(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (pat, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    let pat = get_value(&pat);
    Ok(vm.ctx.new_bool(value.ends_with(pat.as_str())))
}

fn str_swapcase(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s);
    let mut swapped_str = String::with_capacity(value.len());
    for c in value.chars() {
        // to_uppercase returns an iterator, to_ascii_uppercase returns the char
        if c.is_lowercase() {
            swapped_str.push(c.to_ascii_uppercase());
        } else if c.is_uppercase() {
            swapped_str.push(c.to_ascii_lowercase());
        } else {
            swapped_str.push(c);
        }
    }
    Ok(vm.ctx.new_str(swapped_str))
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
        optional = [(n, None)]
    );
    let s = get_value(&s);
    let old_str = get_value(&old);
    let rep_str = get_value(&rep);
    let num_rep: usize = match n {
        Some(num) => objint::to_int(vm, num, 10)?.to_usize().unwrap(),
        None => 1,
    };
    let new_str = s.replacen(&old_str, &rep_str, num_rep);
    Ok(vm.ctx.new_str(new_str))
}

fn str_title(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s);
    let titled_str = value
        .split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f
                    .to_uppercase()
                    .chain(c.flat_map(|t| t.to_lowercase()))
                    .collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok(vm.ctx.new_str(titled_str))
}

// TODO: add ability to specify fill character, can't pass it to format!()
fn str_center(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (len, Some(vm.ctx.int_type()))] // optional = [(chars, None)]
    );
    let value = get_value(&s);
    let len = objint::get_value(&len).to_usize().unwrap();
    // let rep_char = match chars {
    //     Some(c) => get_value(&c),
    //     None => " ".to_string(),
    // };
    let new_str = format!("{:^1$}", value, len);
    Ok(vm.ctx.new_str(new_str))
}

fn str_startswith(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (pat, Some(vm.ctx.str_type()))]
    );
    let value = get_value(&s);
    let pat = get_value(&pat);
    Ok(vm.ctx.new_bool(value.starts_with(pat.as_str())))
}

fn str_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (s, Some(vm.ctx.str_type())),
            (needle, Some(vm.ctx.str_type()))
        ]
    );
    let value = get_value(&s);
    let needle = get_value(&needle);
    Ok(vm.ctx.new_bool(value.contains(needle.as_str())))
}

fn str_isalnum(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let is_alnum = get_value(&s).chars().all(|c| c.is_alphanumeric());
    Ok(vm.ctx.new_bool(is_alnum))
}

fn str_isnumeric(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let is_numeric = get_value(&s).chars().all(|c| c.is_numeric());
    Ok(vm.ctx.new_bool(is_numeric))
}

fn str_isalpha(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let is_alpha = get_value(&s).chars().all(|c| c.is_alphanumeric());
    Ok(vm.ctx.new_bool(is_alpha))
}

fn str_isdigit(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let value = get_value(&s);
    // python's isdigit also checks if exponents are digits, these are the unicodes for exponents
    let valid_unicodes: [u16; 10] = [
        0x2070, 0x00B9, 0x00B2, 0x00B3, 0x2074, 0x2075, 0x2076, 0x2077, 0x2078, 0x2079,
    ];
    let mut is_digit: bool = true;
    for c in value.chars() {
        if !c.is_digit(10) {
            // checking if char is exponent
            let char_as_uni: u16 = c as u16;
            if valid_unicodes.contains(&char_as_uni) {
                continue;
            } else {
                is_digit = false;
                break;
            }
        }
    }
    Ok(vm.ctx.new_bool(is_digit))
}

fn str_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(s, Some(vm.ctx.str_type())), (needle, None)]
    );
    let value = get_value(&s);
    subscript(vm, &value, needle.clone())
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
    fn do_slice(&self, start: usize, stop: usize) -> Self {
        self[start..stop].to_string()
    }
    fn do_stepped_slice(&self, start: usize, stop: usize, step: usize) -> Self {
        self[start..stop].chars().step_by(step).collect()
    }
    fn len(&self) -> usize {
        self.len()
    }
}

pub fn subscript(vm: &mut VirtualMachine, value: &str, b: PyObjectRef) -> PyResult {
    // let value = a
    if objtype::isinstance(&b, &vm.ctx.int_type()) {
        let pos = objint::get_value(&b).to_i32().unwrap();
        let idx = value.to_string().get_pos(pos);
        Ok(vm.new_str(value[idx..idx + 1].to_string()))
    } else {
        match &(*b.borrow()).kind {
            &PyObjectKind::Slice {
                start: _,
                stop: _,
                step: _,
            } => Ok(vm.new_str(value.to_string().get_slice_items(&b).to_string())),
            _ => panic!(
                "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
                value, b
            ),
        }
    }
}

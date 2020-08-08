/// Implementation of Printf-Style string formatting
/// [https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting]
use crate::format::get_num_digits;
use crate::obj::{objfloat, objint, objstr, objtuple, objtype};
use crate::pyobject::{
    BorrowValue, ItemProtocol, PyObjectRef, PyResult, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::{BigInt, Sign};
use num_traits::cast::ToPrimitive;
use num_traits::Signed;
use std::cmp;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, PartialEq)]
enum CFormatErrorType {
    UnmatchedKeyParentheses,
    MissingModuloSign,
    UnescapedModuloSignInLiteral,
    UnsupportedFormatChar(char),
    IncompleteFormat,
    // Unimplemented,
}

// also contains how many chars the parsing function consumed
type ParsingError = (CFormatErrorType, usize);

#[derive(Debug, PartialEq)]
pub(crate) struct CFormatError {
    typ: CFormatErrorType,
    index: usize,
}

impl fmt::Display for CFormatError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use CFormatErrorType::*;
        match self.typ {
            UnmatchedKeyParentheses => write!(f, "incomplete format key"),
            CFormatErrorType::IncompleteFormat => write!(f, "incomplete format"),
            UnsupportedFormatChar(c) => write!(
                f,
                "unsupported format character '{}' ({:#x}) at index {}",
                c, c as u32, self.index
            ),
            _ => write!(f, "unexpected error parsing format string"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum CFormatPreconversor {
    Repr,
    Str,
    Ascii,
    Bytes,
}

#[derive(Debug, PartialEq)]
enum CFormatCase {
    Lowercase,
    Uppercase,
}

#[derive(Debug, PartialEq)]
enum CNumberType {
    Decimal,
    Octal,
    Hex(CFormatCase),
}

#[derive(Debug, PartialEq)]
enum CFloatType {
    Exponent(CFormatCase),
    PointDecimal,
    General(CFormatCase),
}

#[derive(Debug, PartialEq)]
enum CFormatType {
    Number(CNumberType),
    Float(CFloatType),
    Character,
    String(CFormatPreconversor),
}

bitflags! {
    struct CConversionFlags: u32 {
        const ALTERNATE_FORM = 0b0000_0001;
        const ZERO_PAD = 0b0000_0010;
        const LEFT_ADJUST = 0b0000_0100;
        const BLANK_SIGN = 0b0000_1000;
        const SIGN_CHAR = 0b0001_0000;
    }
}

impl CConversionFlags {
    fn sign_string(&self) -> &'static str {
        if self.contains(CConversionFlags::SIGN_CHAR) {
            "+"
        } else if self.contains(CConversionFlags::BLANK_SIGN) {
            " "
        } else {
            ""
        }
    }
}

#[derive(Debug, PartialEq)]
enum CFormatQuantity {
    Amount(usize),
    FromValuesTuple,
}

#[derive(Debug, PartialEq)]
struct CFormatSpec {
    mapping_key: Option<String>,
    flags: CConversionFlags,
    min_field_width: Option<CFormatQuantity>,
    precision: Option<CFormatQuantity>,
    format_type: CFormatType,
    format_char: char,
    chars_consumed: usize,
}

impl CFormatSpec {
    fn compute_fill_string(fill_char: char, fill_chars_needed: usize) -> String {
        (0..fill_chars_needed)
            .map(|_| fill_char)
            .collect::<String>()
    }

    fn fill_string(
        &self,
        string: String,
        fill_char: char,
        num_prefix_chars: Option<usize>,
    ) -> String {
        let mut num_chars = string.chars().count();
        if let Some(num_prefix_chars) = num_prefix_chars {
            num_chars += num_prefix_chars;
        }
        let num_chars = num_chars;

        let width = match self.min_field_width {
            Some(CFormatQuantity::Amount(width)) => cmp::max(width, num_chars),
            _ => num_chars,
        };
        let fill_chars_needed = width - num_chars;
        let fill_string = CFormatSpec::compute_fill_string(fill_char, fill_chars_needed);

        if !fill_string.is_empty() {
            if self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                format!("{}{}", string, fill_string)
            } else {
                format!("{}{}", fill_string, string)
            }
        } else {
            string
        }
    }

    fn format_string_with_precision(
        &self,
        string: String,
        precision: Option<&CFormatQuantity>,
    ) -> String {
        // truncate if needed
        let string = match precision {
            Some(CFormatQuantity::Amount(precision)) if string.chars().count() > *precision => {
                string.chars().take(*precision).collect::<String>()
            }
            _ => string,
        };
        self.fill_string(string, ' ', None)
    }

    pub(crate) fn format_string(&self, string: String) -> String {
        self.format_string_with_precision(string, self.precision.as_ref())
    }

    fn format_char(&self, ch: char) -> String {
        self.format_string_with_precision(ch.to_string(), Some(&CFormatQuantity::Amount(1)))
    }

    fn format_number(&self, num: &BigInt) -> String {
        use CFormatCase::{Lowercase, Uppercase};
        use CNumberType::*;
        let magnitude = num.abs();
        let prefix = if self.flags.contains(CConversionFlags::ALTERNATE_FORM) {
            match self.format_type {
                CFormatType::Number(Octal) => "0o",
                CFormatType::Number(Hex(Lowercase)) => "0x",
                CFormatType::Number(Hex(Uppercase)) => "0X",
                _ => "",
            }
        } else {
            ""
        };

        let magnitude_string: String = match self.format_type {
            CFormatType::Number(Decimal) => magnitude.to_str_radix(10),
            CFormatType::Number(Octal) => magnitude.to_str_radix(8),
            CFormatType::Number(Hex(Lowercase)) => magnitude.to_str_radix(16),
            CFormatType::Number(Hex(Uppercase)) => {
                let mut result = magnitude.to_str_radix(16);
                result.make_ascii_uppercase();
                result
            }
            _ => unreachable!(), // Should not happen because caller has to make sure that this is a number
        };

        let sign_string = match num.sign() {
            Sign::Minus => "-",
            _ => self.flags.sign_string(),
        };

        if self.flags.contains(CConversionFlags::ZERO_PAD) {
            let fill_char = if !self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                '0'
            } else {
                ' ' // '-' overrides the '0' conversion if both are given
            };
            let signed_prefix = format!("{}{}", sign_string, prefix);
            format!(
                "{}{}",
                signed_prefix,
                self.fill_string(
                    magnitude_string,
                    fill_char,
                    Some(signed_prefix.chars().count())
                )
            )
        } else {
            self.fill_string(
                format!("{}{}{}", sign_string, prefix, magnitude_string),
                ' ',
                None,
            )
        }
    }

    pub(crate) fn format_float(&self, num: f64) -> Result<String, String> {
        let sign_string = if num.is_sign_positive() {
            self.flags.sign_string()
        } else {
            "-"
        };

        let magnitude_string = match self.format_type {
            CFormatType::Float(CFloatType::PointDecimal) => {
                let precision = match self.precision {
                    Some(CFormatQuantity::Amount(p)) => p,
                    _ => 6,
                };
                let magnitude = num.abs();
                format!("{:.*}", precision, magnitude)
            }
            CFormatType::Float(CFloatType::Exponent(_)) => {
                return Err("Not yet implemented for %e and %E".to_owned())
            }
            CFormatType::Float(CFloatType::General(_)) => {
                return Err("Not yet implemented for %g and %G".to_owned())
            }
            _ => unreachable!(),
        };

        if self.flags.contains(CConversionFlags::ZERO_PAD) {
            let fill_char = if !self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                '0'
            } else {
                ' '
            };
            Ok(format!(
                "{}{}",
                sign_string,
                self.fill_string(
                    magnitude_string,
                    fill_char,
                    Some(sign_string.chars().count())
                )
            ))
        } else {
            Ok(self.fill_string(format!("{}{}", sign_string, magnitude_string), ' ', None))
        }
    }

    fn format(&self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<String> {
        // do the formatting by type
        match &self.format_type {
            CFormatType::String(preconversor) => {
                let result = match preconversor {
                    CFormatPreconversor::Str => vm.to_str(&obj)?,
                    CFormatPreconversor::Repr | CFormatPreconversor::Ascii => vm.to_repr(&obj)?,
                    CFormatPreconversor::Bytes => TryFromObject::try_from_object(
                        vm,
                        vm.call_method(&obj.clone(), "decode", vec![])?,
                    )?,
                };
                Ok(self.format_string(result.borrow_value().to_owned()))
            }
            CFormatType::Number(number_type) => {
                if !objtype::isinstance(&obj, &vm.ctx.types.int_type) {
                    let required_type_string = match number_type {
                        CNumberType::Decimal => "a number",
                        _ => "an integer",
                    };
                    return Err(vm.new_type_error(format!(
                        "%{} format: {} is required, not {}",
                        self.format_char,
                        required_type_string,
                        obj.lease_class()
                    )));
                }
                Ok(self.format_number(objint::get_value(&obj)))
            }
            CFormatType::Float(_) => if let Some(value) = objfloat::try_float(&obj, vm)? {
                self.format_float(value)
            } else {
                let required_type_string = "an floating point or integer";
                return Err(vm.new_type_error(format!(
                    "%{} format: {} is required, not {}",
                    self.format_char,
                    required_type_string,
                    obj.lease_class()
                )));
            }
            .map_err(|e| vm.new_not_implemented_error(e)),
            CFormatType::Character => {
                let ch = {
                    if objtype::isinstance(&obj, &vm.ctx.types.int_type) {
                        // BigInt truncation is fine in this case because only the unicode range is relevant
                        objint::get_value(&obj)
                            .to_u32()
                            .and_then(std::char::from_u32)
                            .ok_or_else(|| {
                                vm.new_overflow_error("%c arg not in range(0x110000)".to_owned())
                            })
                    } else if objtype::isinstance(&obj, &vm.ctx.types.str_type) {
                        let s = objstr::borrow_value(&obj);
                        let num_chars = s.chars().count();
                        if num_chars != 1 {
                            Err(vm.new_type_error("%c requires int or char".to_owned()))
                        } else {
                            Ok(s.chars().next().unwrap())
                        }
                    } else {
                        // TODO re-arrange this block so this error is only created once
                        Err(vm.new_type_error("%c requires int or char".to_owned()))
                    }
                }?;
                Ok(self.format_char(ch))
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum CFormatPart {
    Literal(String),
    Spec(CFormatSpec),
}

impl CFormatPart {
    fn is_specifier(&self) -> bool {
        match self {
            CFormatPart::Spec(_) => true,
            _ => false,
        }
    }

    fn has_key(&self) -> bool {
        match self {
            CFormatPart::Spec(s) => s.mapping_key.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct CFormatString {
    parts: Vec<(usize, CFormatPart)>,
}

impl FromStr for CFormatString {
    type Err = CFormatError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut cur_text: &str = text;
        let mut index = 0;
        let mut parts: Vec<(usize, CFormatPart)> = Vec::new();
        while !cur_text.is_empty() {
            cur_text = parse_literal(cur_text)
                .or_else(|_| parse_specifier(cur_text))
                .map(|(format_part, new_text, consumed)| {
                    parts.push((index, format_part));
                    index += consumed;
                    new_text
                })
                .map_err(|(e, consumed)| CFormatError {
                    typ: e,
                    index: index + consumed,
                })?;
        }

        Ok(CFormatString { parts })
    }
}

impl CFormatString {
    pub(crate) fn format(
        &mut self,
        vm: &VirtualMachine,
        values_obj: PyObjectRef,
    ) -> PyResult<String> {
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
                            if !objtype::isinstance(&width_obj, &vm.ctx.types.int_type) {
                                Err(vm.new_type_error("* wants int".to_owned()))
                            } else {
                                // TODO: handle errors when truncating BigInt to usize
                                *q = Some(CFormatQuantity::Amount(
                                    objint::get_value(&width_obj).to_usize().unwrap(),
                                ));
                                Ok(tuple_index)
                            }
                        }
                        None => Err(
                            vm.new_type_error("not enough arguments for format string".to_owned())
                        ),
                    }
                }
                _ => Ok(tuple_index),
            }
        }

        let mut final_string = String::new();
        let num_specifiers = self
            .parts
            .iter()
            .filter(|(_, part)| CFormatPart::is_specifier(part))
            .count();
        let mapping_required = self
            .parts
            .iter()
            .any(|(_, part)| CFormatPart::has_key(part))
            && self
                .parts
                .iter()
                .filter(|(_, part)| CFormatPart::is_specifier(part))
                .all(|(_, part)| CFormatPart::has_key(part));

        let values = if mapping_required {
            if !objtype::isinstance(&values_obj, &vm.ctx.types.dict_type) {
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
            if !objtype::isinstance(&values_obj, &vm.ctx.types.tuple_type) {
                vm.ctx.new_tuple(vec![values_obj.clone()])
            } else {
                values_obj.clone()
            }
        };

        let mut tuple_index: usize = 0;
        for (_, part) in &mut self.parts {
            let result_string: String = match part {
                CFormatPart::Spec(format_spec) => {
                    // try to get the object
                    let obj: PyObjectRef = match &format_spec.mapping_key {
                        Some(key) => {
                            // TODO: change the KeyError message to match the one in cpython
                            values.get_item(key, vm)?
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
                    format_spec.format(vm, obj)
                }
                CFormatPart::Literal(literal) => Ok(literal.clone()),
            }?;
            final_string.push_str(&result_string);
        }

        // check that all arguments were converted
        if (!mapping_required && objtuple::get_value(&values).get(tuple_index).is_some())
            && !objtype::isinstance(&values_obj, &vm.ctx.types.dict_type)
        {
            return Err(vm.new_type_error(
                "not all arguments converted during string formatting".to_owned(),
            ));
        }
        Ok(final_string)
    }
}

fn parse_quantity(text: &str) -> (Option<CFormatQuantity>, &str) {
    let num_digits: usize = get_num_digits(text);
    if num_digits == 0 {
        let mut chars = text.chars();
        return match chars.next() {
            Some('*') => (Some(CFormatQuantity::FromValuesTuple), chars.as_str()),
            _ => (None, text),
        };
    }
    // This should never fail
    (
        Some(CFormatQuantity::Amount(
            text[..num_digits].parse::<usize>().unwrap(),
        )),
        &text[num_digits..],
    )
}

fn parse_precision(text: &str) -> (Option<CFormatQuantity>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('.') => parse_quantity(&chars.as_str()),
        _ => (None, text),
    }
}

fn parse_literal_single(text: &str) -> Result<(char, &str), CFormatErrorType> {
    let mut chars = text.chars();
    // TODO get rid of the unwrap
    let first_char = chars.next().unwrap();
    if first_char == '%' {
        // if we see a %, it has to be escaped
        match chars.next() {
            Some(next_char) => {
                if next_char != first_char {
                    Err(CFormatErrorType::UnescapedModuloSignInLiteral)
                } else {
                    Ok((first_char, chars.as_str()))
                }
            }
            None => Err(CFormatErrorType::IncompleteFormat),
        }
    } else {
        Ok((first_char, chars.as_str()))
    }
}

fn parse_literal(text: &str) -> Result<(CFormatPart, &str, usize), ParsingError> {
    let mut cur_text = text;
    let mut result_string = String::new();
    let mut consumed = 0;
    while !cur_text.is_empty() {
        match parse_literal_single(cur_text) {
            Ok((next_char, remaining)) => {
                result_string.push(next_char);
                consumed += 1;
                cur_text = remaining;
            }
            Err(err) => {
                if !result_string.is_empty() {
                    return Ok((CFormatPart::Literal(result_string), cur_text, consumed));
                } else {
                    return Err((err, consumed));
                }
            }
        }
    }
    Ok((
        CFormatPart::Literal(result_string),
        "",
        text.chars().count(),
    ))
}

fn parse_text_inside_parentheses(text: &str) -> Option<(String, &str)> {
    let mut counter = 1;
    let mut chars = text.chars();
    let mut contained_text = String::new();
    while counter > 0 {
        let c = chars.next();

        match c {
            Some('(') => {
                counter += 1;
            }
            Some(')') => {
                counter -= 1;
            }
            None => {
                return None;
            }
            _ => (),
        }

        if counter > 0 {
            contained_text.push(c.unwrap());
        }
    }

    Some((contained_text, chars.as_str()))
}

fn parse_spec_mapping_key(text: &str) -> Result<(Option<String>, &str), CFormatErrorType> {
    let mut chars = text.chars();

    let next_char = chars.next();
    if next_char == Some('(') {
        match parse_text_inside_parentheses(chars.as_str()) {
            Some((key, remaining_text)) => Ok((Some(key), remaining_text)),
            None => Err(CFormatErrorType::UnmatchedKeyParentheses),
        }
    } else {
        Ok((None, text))
    }
}

fn parse_flag_single(text: &str) -> (Option<CConversionFlags>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('#') => (Some(CConversionFlags::ALTERNATE_FORM), chars.as_str()),
        Some('0') => (Some(CConversionFlags::ZERO_PAD), chars.as_str()),
        Some('-') => (Some(CConversionFlags::LEFT_ADJUST), chars.as_str()),
        Some(' ') => (Some(CConversionFlags::BLANK_SIGN), chars.as_str()),
        Some('+') => (Some(CConversionFlags::SIGN_CHAR), chars.as_str()),
        _ => (None, text),
    }
}

fn parse_flags(text: &str) -> (CConversionFlags, &str) {
    let mut flags = CConversionFlags::empty();
    let mut cur_text = text;
    while !cur_text.is_empty() {
        match parse_flag_single(cur_text) {
            (Some(flag), text) => {
                flags |= flag;
                cur_text = text;
            }

            (None, text) => {
                return (flags, text);
            }
        }
    }

    (flags, "")
}

fn consume_length(text: &str) -> &str {
    let mut chars = text.chars();
    match chars.next() {
        Some('h') | Some('l') | Some('L') => chars.as_str(),
        _ => text,
    }
}

fn parse_format_type(text: &str) -> Result<(CFormatType, &str, char), CFormatErrorType> {
    use CFloatType::*;
    use CFormatCase::{Lowercase, Uppercase};
    use CNumberType::*;
    let mut chars = text.chars();
    let next_char = chars.next();
    match next_char {
        Some('d') | Some('i') | Some('u') => Ok((
            CFormatType::Number(Decimal),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('o') => Ok((
            CFormatType::Number(Octal),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('x') => Ok((
            CFormatType::Number(Hex(Lowercase)),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('X') => Ok((
            CFormatType::Number(Hex(Uppercase)),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('e') => Ok((
            CFormatType::Float(Exponent(Lowercase)),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('E') => Ok((
            CFormatType::Float(Exponent(Uppercase)),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('f') => Ok((
            CFormatType::Float(PointDecimal),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('F') => Ok((
            CFormatType::Float(PointDecimal),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('g') => Ok((
            CFormatType::Float(General(Lowercase)),
            text,
            next_char.unwrap(),
        )),
        Some('G') => Ok((
            CFormatType::Float(General(Uppercase)),
            text,
            next_char.unwrap(),
        )),
        Some('c') => Ok((CFormatType::Character, chars.as_str(), next_char.unwrap())),
        Some('r') => Ok((
            CFormatType::String(CFormatPreconversor::Repr),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('s') => Ok((
            CFormatType::String(CFormatPreconversor::Str),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('b') => Ok((
            CFormatType::String(CFormatPreconversor::Bytes),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some('a') => Ok((
            CFormatType::String(CFormatPreconversor::Ascii),
            chars.as_str(),
            next_char.unwrap(),
        )),
        Some(c) => Err(CFormatErrorType::UnsupportedFormatChar(c)),
        None => Err(CFormatErrorType::IncompleteFormat), // should not happen because it is handled earlier in the parsing
    }
}

fn calc_consumed(a: &str, b: &str) -> usize {
    a.chars().count() - b.chars().count()
}

impl FromStr for CFormatSpec {
    type Err = ParsingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut chars = text.chars();
        if chars.next() != Some('%') {
            return Err((CFormatErrorType::MissingModuloSign, 1));
        }

        let after_modulo_sign = chars.as_str();
        let (mapping_key, after_mapping_key) = parse_spec_mapping_key(after_modulo_sign)
            .map_err(|err| (err, calc_consumed(text, after_modulo_sign)))?;
        let (flags, after_flags) = parse_flags(after_mapping_key);
        let (width, after_width) = parse_quantity(after_flags);
        let (precision, after_precision) = parse_precision(after_width);
        // A length modifier (h, l, or L) may be present,
        // but is ignored as it is not necessary for Python – so e.g. %ld is identical to %d.
        let after_length = consume_length(after_precision);
        let (format_type, remaining_text, format_char) = parse_format_type(after_length)
            .map_err(|err| (err, calc_consumed(text, after_length)))?;

        // apply default precision for float types
        let precision = match precision {
            Some(precision) => Some(precision),
            None => match format_type {
                CFormatType::Float(_) => Some(CFormatQuantity::Amount(6)),
                _ => None,
            },
        };

        Ok(CFormatSpec {
            mapping_key,
            flags,
            min_field_width: width,
            precision,
            format_type,
            format_char,
            chars_consumed: calc_consumed(text, remaining_text),
        })
    }
}

fn parse_specifier(text: &str) -> Result<(CFormatPart, &str, usize), ParsingError> {
    let spec = text.parse::<CFormatSpec>()?;
    let chars_consumed = spec.chars_consumed;
    Ok((
        CFormatPart::Spec(spec),
        &text[chars_consumed..],
        chars_consumed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill_and_align() {
        assert_eq!(
            "%10s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("test".to_owned()),
            "      test".to_owned()
        );
        assert_eq!(
            "%-10s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("test".to_owned()),
            "test      ".to_owned()
        );
        assert_eq!(
            "%#10x"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(0x1337)),
            "    0x1337".to_owned()
        );
        assert_eq!(
            "%-#10x"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(0x1337)),
            "0x1337    ".to_owned()
        );
    }

    #[test]
    fn test_parse_key() {
        let expected = Ok(CFormatSpec {
            mapping_key: Some("amount".to_owned()),
            format_type: CFormatType::Number(CNumberType::Decimal),
            format_char: 'd',
            chars_consumed: 10,
            min_field_width: None,
            precision: None,
            flags: CConversionFlags::empty(),
        });
        assert_eq!("%(amount)d".parse::<CFormatSpec>(), expected);

        let expected = Ok(CFormatSpec {
            mapping_key: Some("m((u(((l((((ti))))p)))l))e".to_owned()),
            format_type: CFormatType::Number(CNumberType::Decimal),
            format_char: 'd',
            chars_consumed: 30,
            min_field_width: None,
            precision: None,
            flags: CConversionFlags::empty(),
        });
        assert_eq!(
            "%(m((u(((l((((ti))))p)))l))e)d".parse::<CFormatSpec>(),
            expected
        );
    }

    #[test]
    fn test_format_parse_key_fail() {
        assert_eq!(
            "%(aged".parse::<CFormatString>(),
            Err(CFormatError {
                typ: CFormatErrorType::UnmatchedKeyParentheses,
                index: 1
            })
        );
    }

    #[test]
    fn test_format_parse_type_fail() {
        assert_eq!(
            "Hello %n".parse::<CFormatString>(),
            Err(CFormatError {
                typ: CFormatErrorType::UnsupportedFormatChar('n'),
                index: 7
            })
        );
    }

    #[test]
    fn test_incomplete_format_fail() {
        assert_eq!(
            "Hello %".parse::<CFormatString>(),
            Err(CFormatError {
                typ: CFormatErrorType::IncompleteFormat,
                index: 7
            })
        );
    }

    #[test]
    fn test_parse_flags() {
        let expected = Ok(CFormatSpec {
            format_type: CFormatType::Number(CNumberType::Decimal),
            format_char: 'd',
            chars_consumed: 17,
            min_field_width: Some(CFormatQuantity::Amount(10)),
            precision: None,
            mapping_key: None,
            flags: CConversionFlags::all(),
        });
        let parsed = "%  0   -+++###10d".parse::<CFormatSpec>();
        assert_eq!(parsed, expected);
        assert_eq!(
            parsed.unwrap().format_number(&BigInt::from(12)),
            "+12       ".to_owned()
        );
    }

    #[test]
    fn test_parse_and_format_string() {
        assert_eq!(
            "%5.4s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("Hello, World!".to_owned()),
            " Hell".to_owned()
        );
        assert_eq!(
            "%-5.4s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("Hello, World!".to_owned()),
            "Hell ".to_owned()
        );
    }

    #[test]
    fn test_parse_and_format_unicode_string() {
        assert_eq!(
            "%.2s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("❤❤❤❤❤❤❤❤".to_owned()),
            "❤❤".to_owned()
        );
    }

    #[test]
    fn test_parse_and_format_number() {
        assert_eq!(
            "%05d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(27)),
            "00027".to_owned()
        );
        assert_eq!(
            "%+05d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(27)),
            "+0027".to_owned()
        );
        assert_eq!(
            "%-d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(-27)),
            "-27".to_owned()
        );
        assert_eq!(
            "% d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(27)),
            " 27".to_owned()
        );
        assert_eq!(
            "% d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(-27)),
            "-27".to_owned()
        );
        assert_eq!(
            "%08x"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(0x1337)),
            "00001337".to_owned()
        );
        assert_eq!(
            "%#010x"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(0x1337)),
            "0x00001337".to_owned()
        );
        assert_eq!(
            "%-#010x"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(0x1337)),
            "0x1337    ".to_owned()
        );
    }

    #[test]
    fn test_parse_and_format_float() {
        assert_eq!(
            "%f".parse::<CFormatSpec>()
                .unwrap()
                .format_float(f64::from(1.2345))
                .ok(),
            Some("1.234500".to_owned())
        );
        assert_eq!(
            "%+f"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_float(f64::from(1.2345))
                .ok(),
            Some("+1.234500".to_owned())
        );
        assert_eq!(
            "% f"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_float(f64::from(1.2345))
                .ok(),
            Some(" 1.234500".to_owned())
        );
        assert_eq!(
            "%f".parse::<CFormatSpec>()
                .unwrap()
                .format_float(f64::from(-1.2345))
                .ok(),
            Some("-1.234500".to_owned())
        );
        assert_eq!(
            "%f".parse::<CFormatSpec>()
                .unwrap()
                .format_float(f64::from(1.2345678901))
                .ok(),
            Some("1.234568".to_owned())
        );
    }

    #[test]
    fn test_format_parse() {
        let fmt = "Hello, my name is %s and I'm %d years old";
        let expected = Ok(CFormatString {
            parts: vec![
                (0, CFormatPart::Literal("Hello, my name is ".to_owned())),
                (
                    18,
                    CFormatPart::Spec(CFormatSpec {
                        format_type: CFormatType::String(CFormatPreconversor::Str),
                        format_char: 's',
                        chars_consumed: 2,
                        mapping_key: None,
                        min_field_width: None,
                        precision: None,
                        flags: CConversionFlags::empty(),
                    }),
                ),
                (20, CFormatPart::Literal(" and I'm ".to_owned())),
                (
                    29,
                    CFormatPart::Spec(CFormatSpec {
                        format_type: CFormatType::Number(CNumberType::Decimal),
                        format_char: 'd',
                        chars_consumed: 2,
                        mapping_key: None,
                        min_field_width: None,
                        precision: None,
                        flags: CConversionFlags::empty(),
                    }),
                ),
                (31, CFormatPart::Literal(" years old".to_owned())),
            ],
        });
        let result = fmt.parse::<CFormatString>();
        assert_eq!(
            result, expected,
            "left = {:#?} \n\n\n right = {:#?}",
            result, expected
        );
    }
}

use crate::builtins::{self, pystr};
use crate::common::float_ops;
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::FuncArgs;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;
use itertools::{Itertools, PeekingNext};
use num_bigint::{BigInt, Sign};
use num_traits::cast::ToPrimitive;
use num_traits::Signed;
use std::cmp;
use std::str::FromStr;

#[derive(Debug, Copy, Clone, PartialEq)]
enum FormatPreconversor {
    Str,
    Repr,
    Ascii,
    Bytes,
}

impl FormatPreconversor {
    fn from_char(c: char) -> Option<FormatPreconversor> {
        match c {
            's' => Some(FormatPreconversor::Str),
            'r' => Some(FormatPreconversor::Repr),
            'a' => Some(FormatPreconversor::Ascii),
            'b' => Some(FormatPreconversor::Bytes),
            _ => None,
        }
    }

    fn from_string(text: &str) -> Option<FormatPreconversor> {
        let mut chars = text.chars();
        if chars.next() != Some('!') {
            return None;
        }

        FormatPreconversor::from_char(chars.next()?)
    }

    fn parse_and_consume(text: &str) -> (Option<FormatPreconversor>, &str) {
        let preconversor = FormatPreconversor::from_string(text);
        match preconversor {
            None => (None, text),
            Some(_) => {
                let mut chars = text.chars();
                chars.next(); // Consume the bang
                chars.next(); // Consume one r,s,a char
                (preconversor, chars.as_str())
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum FormatAlign {
    Left,
    Right,
    AfterSign,
    Center,
}

impl FormatAlign {
    fn from_char(c: char) -> Option<FormatAlign> {
        match c {
            '<' => Some(FormatAlign::Left),
            '>' => Some(FormatAlign::Right),
            '=' => Some(FormatAlign::AfterSign),
            '^' => Some(FormatAlign::Center),
            _ => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
enum FormatSign {
    Plus,
    Minus,
    MinusOrSpace,
}

#[derive(Debug, PartialEq)]
enum FormatGrouping {
    Comma,
    Underscore,
}

#[derive(Debug, PartialEq)]
enum FormatType {
    String,
    Binary,
    Character,
    Decimal,
    Octal,
    HexLower,
    HexUpper,
    Number,
    ExponentLower,
    ExponentUpper,
    GeneralFormatLower,
    GeneralFormatUpper,
    FixedPointLower,
    FixedPointUpper,
    Percentage,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FormatSpec {
    preconversor: Option<FormatPreconversor>,
    fill: Option<char>,
    align: Option<FormatAlign>,
    sign: Option<FormatSign>,
    alternate_form: bool,
    width: Option<usize>,
    grouping_option: Option<FormatGrouping>,
    precision: Option<usize>,
    format_type: Option<FormatType>,
}

pub(crate) fn get_num_digits(text: &str) -> usize {
    for (index, character) in text.char_indices() {
        if !character.is_digit(10) {
            return index;
        }
    }
    text.len()
}

fn parse_preconversor(text: &str) -> (Option<FormatPreconversor>, &str) {
    FormatPreconversor::parse_and_consume(text)
}

fn parse_align(text: &str) -> (Option<FormatAlign>, &str) {
    let mut chars = text.chars();
    let maybe_align = chars.next().and_then(FormatAlign::from_char);
    if maybe_align.is_some() {
        (maybe_align, &chars.as_str())
    } else {
        (None, text)
    }
}

fn parse_fill_and_align(text: &str) -> (Option<char>, Option<FormatAlign>, &str) {
    let char_indices: Vec<(usize, char)> = text.char_indices().take(3).collect();
    if char_indices.is_empty() {
        (None, None, text)
    } else if char_indices.len() == 1 {
        let (maybe_align, remaining) = parse_align(text);
        (None, maybe_align, remaining)
    } else {
        let (maybe_align, remaining) = parse_align(&text[char_indices[1].0..]);
        if maybe_align.is_some() {
            (Some(char_indices[0].1), maybe_align, remaining)
        } else {
            let (only_align, only_align_remaining) = parse_align(text);
            (None, only_align, only_align_remaining)
        }
    }
}

fn parse_number(text: &str) -> Result<(Option<usize>, &str), &'static str> {
    let num_digits: usize = get_num_digits(text);
    if num_digits == 0 {
        return Ok((None, text));
    }
    if let Ok(num) = text[..num_digits].parse::<usize>() {
        Ok((Some(num), &text[num_digits..]))
    } else {
        // NOTE: this condition is different from CPython
        Err("Too many decimal digits in format string")
    }
}

fn parse_sign(text: &str) -> (Option<FormatSign>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('-') => (Some(FormatSign::Minus), chars.as_str()),
        Some('+') => (Some(FormatSign::Plus), chars.as_str()),
        Some(' ') => (Some(FormatSign::MinusOrSpace), chars.as_str()),
        _ => (None, text),
    }
}

fn parse_alternate_form(text: &str) -> (bool, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('#') => (true, chars.as_str()),
        _ => (false, text),
    }
}

fn parse_zero(text: &str) -> (bool, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('0') => (true, chars.as_str()),
        _ => (false, text),
    }
}

fn parse_precision(text: &str) -> Result<(Option<usize>, &str), &'static str> {
    let mut chars = text.chars();
    Ok(match chars.next() {
        Some('.') => {
            let (size, remaining) = parse_number(&chars.as_str())?;
            if size.is_some() {
                (size, remaining)
            } else {
                (None, text)
            }
        }
        _ => (None, text),
    })
}

fn parse_grouping_option(text: &str) -> (Option<FormatGrouping>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('_') => (Some(FormatGrouping::Underscore), chars.as_str()),
        Some(',') => (Some(FormatGrouping::Comma), chars.as_str()),
        _ => (None, text),
    }
}

fn parse_format_type(text: &str) -> (Option<FormatType>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some('s') => (Some(FormatType::String), chars.as_str()),
        Some('b') => (Some(FormatType::Binary), chars.as_str()),
        Some('c') => (Some(FormatType::Character), chars.as_str()),
        Some('d') => (Some(FormatType::Decimal), chars.as_str()),
        Some('o') => (Some(FormatType::Octal), chars.as_str()),
        Some('x') => (Some(FormatType::HexLower), chars.as_str()),
        Some('X') => (Some(FormatType::HexUpper), chars.as_str()),
        Some('e') => (Some(FormatType::ExponentLower), chars.as_str()),
        Some('E') => (Some(FormatType::ExponentUpper), chars.as_str()),
        Some('f') => (Some(FormatType::FixedPointLower), chars.as_str()),
        Some('F') => (Some(FormatType::FixedPointUpper), chars.as_str()),
        Some('g') => (Some(FormatType::GeneralFormatLower), chars.as_str()),
        Some('G') => (Some(FormatType::GeneralFormatUpper), chars.as_str()),
        Some('n') => (Some(FormatType::Number), chars.as_str()),
        Some('%') => (Some(FormatType::Percentage), chars.as_str()),
        _ => (None, text),
    }
}

fn parse_format_spec(text: &str) -> Result<FormatSpec, &'static str> {
    // get_integer in CPython
    let (preconversor, after_preconversor) = parse_preconversor(text);
    let (mut fill, mut align, after_align) = parse_fill_and_align(after_preconversor);
    let (sign, after_sign) = parse_sign(after_align);
    let (alternate_form, after_alternate_form) = parse_alternate_form(after_sign);
    let (zero, after_zero) = parse_zero(after_alternate_form);
    let (width, after_width) = parse_number(after_zero)?;
    let (grouping_option, after_grouping_option) = parse_grouping_option(after_width);
    let (precision, after_precision) = parse_precision(after_grouping_option)?;
    let (format_type, after_format_type) = parse_format_type(after_precision);
    if !after_format_type.is_empty() {
        return Err("Invalid format specifier");
    }

    if zero && fill.is_none() {
        fill.replace('0');
        align = align.or(Some(FormatAlign::AfterSign));
    }

    Ok(FormatSpec {
        preconversor,
        fill,
        align,
        sign,
        alternate_form,
        width,
        grouping_option,
        precision,
        format_type,
    })
}

impl FormatSpec {
    pub(crate) fn parse(text: &str) -> Result<FormatSpec, &'static str> {
        parse_format_spec(text)
    }

    fn compute_fill_string(fill_char: char, fill_chars_needed: i32) -> String {
        (0..fill_chars_needed)
            .map(|_| fill_char)
            .collect::<String>()
    }

    fn add_magnitude_separators_for_char(
        magnitude_string: String,
        interval: usize,
        separator: char,
    ) -> String {
        let mut result = String::new();

        // Don't add separators to the floating decimal point of numbers
        let mut parts = magnitude_string.splitn(2, '.');
        let magnitude_integer_string = parts.next().unwrap();
        let mut remaining: usize = magnitude_integer_string.len();
        for c in magnitude_integer_string.chars() {
            result.push(c);
            remaining -= 1;
            if remaining % interval == 0 && remaining > 0 {
                result.push(separator);
            }
        }
        if let Some(part) = parts.next() {
            result.push('.');
            result.push_str(part);
        }
        result
    }

    fn get_separator_interval(&self) -> usize {
        match self.format_type {
            Some(FormatType::Binary) => 4,
            Some(FormatType::Decimal) => 3,
            Some(FormatType::Octal) => 4,
            Some(FormatType::HexLower) => 4,
            Some(FormatType::HexUpper) => 4,
            Some(FormatType::Number) => 3,
            Some(FormatType::FixedPointLower) | Some(FormatType::FixedPointUpper) => 3,
            None => 3,
            _ => panic!("Separators only valid for numbers!"),
        }
    }

    fn add_magnitude_separators(&self, magnitude_string: String) -> String {
        match self.grouping_option {
            Some(FormatGrouping::Comma) => FormatSpec::add_magnitude_separators_for_char(
                magnitude_string,
                self.get_separator_interval(),
                ',',
            ),
            Some(FormatGrouping::Underscore) => FormatSpec::add_magnitude_separators_for_char(
                magnitude_string,
                self.get_separator_interval(),
                '_',
            ),
            None => magnitude_string,
        }
    }

    pub(crate) fn format_float(&self, num: f64) -> Result<String, &'static str> {
        let precision = self.precision.unwrap_or(6);
        let magnitude = num.abs();
        let raw_magnitude_string_result: Result<String, &'static str> = match self.format_type {
            Some(FormatType::FixedPointUpper) => Ok(float_ops::format_fixed(
                precision,
                magnitude,
                float_ops::Case::Upper,
            )),
            Some(FormatType::FixedPointLower) => Ok(float_ops::format_fixed(
                precision,
                magnitude,
                float_ops::Case::Lower,
            )),
            Some(FormatType::Decimal) => Err("Unknown format code 'd' for object of type 'float'"),
            Some(FormatType::Binary) => Err("Unknown format code 'b' for object of type 'float'"),
            Some(FormatType::Octal) => Err("Unknown format code 'o' for object of type 'float'"),
            Some(FormatType::HexLower) => Err("Unknown format code 'x' for object of type 'float'"),
            Some(FormatType::HexUpper) => Err("Unknown format code 'X' for object of type 'float'"),
            Some(FormatType::String) => Err("Unknown format code 's' for object of type 'float'"),
            Some(FormatType::Character) => {
                Err("Unknown format code 'c' for object of type 'float'")
            }
            Some(FormatType::Number) => {
                Err("Format code 'n' for object of type 'float' not implemented yet")
            }
            Some(FormatType::GeneralFormatUpper) => {
                let precision = if precision == 0 { 1 } else { precision };
                Ok(float_ops::format_general(
                    precision,
                    magnitude,
                    float_ops::Case::Upper,
                ))
            }
            Some(FormatType::GeneralFormatLower) => {
                let precision = if precision == 0 { 1 } else { precision };
                Ok(float_ops::format_general(
                    precision,
                    magnitude,
                    float_ops::Case::Lower,
                ))
            }
            Some(FormatType::ExponentUpper) => Ok(float_ops::format_exponent(
                precision,
                magnitude,
                float_ops::Case::Upper,
            )),
            Some(FormatType::ExponentLower) => Ok(float_ops::format_exponent(
                precision,
                magnitude,
                float_ops::Case::Lower,
            )),
            Some(FormatType::Percentage) => match magnitude {
                magnitude if magnitude.is_nan() => Ok("nan%".to_owned()),
                magnitude if magnitude.is_infinite() => Ok("inf%".to_owned()),
                _ => Ok(format!("{:.*}%", precision, magnitude * 100.0)),
            },
            None => match magnitude {
                magnitude if magnitude.is_nan() => Ok("nan".to_owned()),
                magnitude if magnitude.is_infinite() => Ok("inf".to_owned()),
                _ => Ok(float_ops::to_string(magnitude)),
            },
        };

        if raw_magnitude_string_result.is_err() {
            return raw_magnitude_string_result;
        }

        let magnitude_string = self.add_magnitude_separators(raw_magnitude_string_result.unwrap());
        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = if num.is_sign_negative() && !num.is_nan() {
            "-"
        } else {
            match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            }
        };

        self.format_sign_and_align(&magnitude_string, sign_str)
    }

    pub(crate) fn format_int(&self, num: &BigInt) -> Result<String, &'static str> {
        let magnitude = num.abs();
        let prefix = if self.alternate_form {
            match self.format_type {
                Some(FormatType::Binary) => "0b",
                Some(FormatType::Octal) => "0o",
                Some(FormatType::HexLower) => "0x",
                Some(FormatType::HexUpper) => "0x",
                _ => "",
            }
        } else {
            ""
        };
        let raw_magnitude_string_result: Result<String, &'static str> = match self.format_type {
            Some(FormatType::Binary) => Ok(magnitude.to_str_radix(2)),
            Some(FormatType::Decimal) => Ok(magnitude.to_str_radix(10)),
            Some(FormatType::Octal) => Ok(magnitude.to_str_radix(8)),
            Some(FormatType::HexLower) => Ok(magnitude.to_str_radix(16)),
            Some(FormatType::HexUpper) => {
                let mut result = magnitude.to_str_radix(16);
                result.make_ascii_uppercase();
                Ok(result)
            }
            Some(FormatType::Number) => Ok(magnitude.to_str_radix(10)),
            Some(FormatType::String) => Err("Unknown format code 's' for object of type 'int'"),
            Some(FormatType::Character) => Err("Unknown format code 'c' for object of type 'int'"),
            Some(FormatType::GeneralFormatUpper) => {
                Err("Unknown format code 'G' for object of type 'int'")
            }
            Some(FormatType::GeneralFormatLower) => {
                Err("Unknown format code 'g' for object of type 'int'")
            }
            Some(FormatType::FixedPointUpper)
            | Some(FormatType::FixedPointLower)
            | Some(FormatType::ExponentUpper)
            | Some(FormatType::ExponentLower)
            | Some(FormatType::Percentage) => match num.to_f64() {
                Some(float) => return self.format_float(float),
                _ => Err("Unable to convert int to float"),
            },
            None => Ok(magnitude.to_str_radix(10)),
        };
        if raw_magnitude_string_result.is_err() {
            return raw_magnitude_string_result;
        }
        let magnitude_string = format!(
            "{}{}",
            prefix,
            self.add_magnitude_separators(raw_magnitude_string_result.unwrap())
        );

        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = match num.sign() {
            Sign::Minus => "-",
            _ => match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            },
        };

        self.format_sign_and_align(&magnitude_string, sign_str)
    }

    pub(crate) fn format_string(&self, s: &str) -> Result<String, &'static str> {
        match self.format_type {
            Some(FormatType::String) | None => self.format_sign_and_align(s, ""),
            _ => Err("Unknown format code for object of type 'str'"),
        }
    }

    fn format_sign_and_align(
        &self,
        magnitude_string: &str,
        sign_str: &str,
    ) -> Result<String, &'static str> {
        let align = self.align.unwrap_or(FormatAlign::Right);

        // Use the byte length as the string length since we're in ascii
        let num_chars = magnitude_string.len();
        let fill_char = self.fill.unwrap_or(' ');
        let fill_chars_needed: i32 = self.width.map_or(0, |w| {
            cmp::max(0, (w as i32) - (num_chars as i32) - (sign_str.len() as i32))
        });
        Ok(match align {
            FormatAlign::Left => format!(
                "{}{}{}",
                sign_str,
                magnitude_string,
                FormatSpec::compute_fill_string(fill_char, fill_chars_needed)
            ),
            FormatAlign::Right => format!(
                "{}{}{}",
                FormatSpec::compute_fill_string(fill_char, fill_chars_needed),
                sign_str,
                magnitude_string
            ),
            FormatAlign::AfterSign => format!(
                "{}{}{}",
                sign_str,
                FormatSpec::compute_fill_string(fill_char, fill_chars_needed),
                magnitude_string
            ),
            FormatAlign::Center => {
                let left_fill_chars_needed = fill_chars_needed / 2;
                let right_fill_chars_needed = fill_chars_needed - left_fill_chars_needed;
                let left_fill_string =
                    FormatSpec::compute_fill_string(fill_char, left_fill_chars_needed);
                let right_fill_string =
                    FormatSpec::compute_fill_string(fill_char, right_fill_chars_needed);
                format!(
                    "{}{}{}{}",
                    left_fill_string, sign_str, magnitude_string, right_fill_string
                )
            }
        })
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum FormatParseError {
    UnmatchedBracket,
    MissingStartBracket,
    UnescapedStartBracketInLiteral,
    InvalidFormatSpecifier,
    UnknownConversion,
    EmptyAttribute,
    MissingRightBracket,
    InvalidCharacterAfterRightBracket,
}

impl IntoPyException for FormatParseError {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            FormatParseError::UnmatchedBracket => {
                vm.new_value_error("expected '}' before end of string".to_owned())
            }
            _ => vm.new_value_error("Unexpected error parsing format string".to_owned()),
        }
    }
}

impl FromStr for FormatSpec {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FormatSpec::parse(s)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum FieldNamePart {
    Attribute(String),
    Index(usize),
    StringIndex(String),
}

impl FieldNamePart {
    fn parse_part(
        chars: &mut impl PeekingNext<Item = char>,
    ) -> Result<Option<FieldNamePart>, FormatParseError> {
        chars
            .next()
            .map(|ch| match ch {
                '.' => {
                    let mut attribute = String::new();
                    for ch in chars.peeking_take_while(|ch| *ch != '.' && *ch != '[') {
                        attribute.push(ch);
                    }
                    if attribute.is_empty() {
                        Err(FormatParseError::EmptyAttribute)
                    } else {
                        Ok(FieldNamePart::Attribute(attribute))
                    }
                }
                '[' => {
                    let mut index = String::new();
                    for ch in chars {
                        if ch == ']' {
                            return if index.is_empty() {
                                Err(FormatParseError::EmptyAttribute)
                            } else if let Ok(index) = index.parse::<usize>() {
                                Ok(FieldNamePart::Index(index))
                            } else {
                                Ok(FieldNamePart::StringIndex(index))
                            };
                        }
                        index.push(ch);
                    }
                    Err(FormatParseError::MissingRightBracket)
                }
                _ => Err(FormatParseError::InvalidCharacterAfterRightBracket),
            })
            .transpose()
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum FieldType {
    Auto,
    Index(usize),
    Keyword(String),
}

#[derive(Debug, PartialEq)]
pub(crate) struct FieldName {
    pub field_type: FieldType,
    pub parts: Vec<FieldNamePart>,
}

impl FieldName {
    pub(crate) fn parse(text: &str) -> Result<FieldName, FormatParseError> {
        let mut chars = text.chars().peekable();
        let mut first = String::new();
        for ch in chars.peeking_take_while(|ch| *ch != '.' && *ch != '[') {
            first.push(ch);
        }

        let field_type = if first.is_empty() {
            FieldType::Auto
        } else if let Ok(index) = first.parse::<usize>() {
            FieldType::Index(index)
        } else {
            FieldType::Keyword(first)
        };

        let mut parts = Vec::new();
        while let Some(part) = FieldNamePart::parse_part(&mut chars)? {
            parts.push(part)
        }

        Ok(FieldName { field_type, parts })
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum FormatPart {
    Field {
        field_name: String,
        preconversion_spec: Option<char>,
        format_spec: String,
    },
    Literal(String),
}

#[derive(Debug, PartialEq)]
pub(crate) struct FormatString {
    pub format_parts: Vec<FormatPart>,
}

impl FormatString {
    fn parse_literal_single(text: &str) -> Result<(char, &str), FormatParseError> {
        let mut chars = text.chars();
        // This should never be called with an empty str
        let first_char = chars.next().unwrap();
        // isn't this detectable only with bytes operation?
        if first_char == '{' || first_char == '}' {
            let maybe_next_char = chars.next();
            // if we see a bracket, it has to be escaped by doubling up to be in a literal
            return if maybe_next_char.is_none() || maybe_next_char.unwrap() != first_char {
                Err(FormatParseError::UnescapedStartBracketInLiteral)
            } else {
                Ok((first_char, chars.as_str()))
            };
        }
        Ok((first_char, chars.as_str()))
    }

    fn parse_literal(text: &str) -> Result<(FormatPart, &str), FormatParseError> {
        let mut cur_text = text;
        let mut result_string = String::new();
        while !cur_text.is_empty() {
            match FormatString::parse_literal_single(cur_text) {
                Ok((next_char, remaining)) => {
                    result_string.push(next_char);
                    cur_text = remaining;
                }
                Err(err) => {
                    return if !result_string.is_empty() {
                        Ok((FormatPart::Literal(result_string), cur_text))
                    } else {
                        Err(err)
                    };
                }
            }
        }
        Ok((FormatPart::Literal(result_string), ""))
    }

    fn parse_part_in_brackets(text: &str) -> Result<FormatPart, FormatParseError> {
        let parts: Vec<&str> = text.splitn(2, ':').collect();
        // before the comma is a keyword or arg index, after the comma is maybe a spec.
        let arg_part = parts[0];

        let format_spec = if parts.len() > 1 {
            parts[1].to_owned()
        } else {
            String::new()
        };

        // On parts[0] can still be the preconversor (!r, !s, !a)
        let parts: Vec<&str> = arg_part.splitn(2, '!').collect();
        // before the bang is a keyword or arg index, after the comma is maybe a conversor spec.
        let arg_part = parts[0];

        let preconversion_spec = parts
            .get(1)
            .map(|conversion| {
                // conversions are only every one character
                conversion
                    .chars()
                    .exactly_one()
                    .map_err(|_| FormatParseError::UnknownConversion)
            })
            .transpose()?;

        Ok(FormatPart::Field {
            field_name: arg_part.to_owned(),
            preconversion_spec,
            format_spec,
        })
    }

    fn parse_spec(text: &str) -> Result<(FormatPart, &str), FormatParseError> {
        let mut nested = false;
        let mut end_bracket_pos = None;
        let mut left = String::new();

        // There may be one layer nesting brackets in spec
        for (idx, c) in text.chars().enumerate() {
            if idx == 0 {
                if c != '{' {
                    return Err(FormatParseError::MissingStartBracket);
                }
            } else if c == '{' {
                if nested {
                    return Err(FormatParseError::InvalidFormatSpecifier);
                } else {
                    nested = true;
                    left.push(c);
                    continue;
                }
            } else if c == '}' {
                if nested {
                    nested = false;
                    left.push(c);
                    continue;
                } else {
                    end_bracket_pos = Some(idx);
                    break;
                }
            } else {
                left.push(c);
            }
        }
        if let Some(pos) = end_bracket_pos {
            let (_, right) = text.split_at(pos);
            let format_part = FormatString::parse_part_in_brackets(&left)?;
            Ok((format_part, &right[1..]))
        } else {
            Err(FormatParseError::UnmatchedBracket)
        }
    }

    fn format_internal(
        &self,
        vm: &VirtualMachine,
        field_func: &mut impl FnMut(FieldType) -> PyResult,
    ) -> PyResult<String> {
        let mut final_string = String::new();
        for part in &self.format_parts {
            let result_string: String = match part {
                FormatPart::Field {
                    field_name,
                    preconversion_spec,
                    format_spec,
                } => {
                    let FieldName { field_type, parts } = FieldName::parse(field_name.as_str())
                        .map_err(|e| e.into_pyexception(vm))?;

                    let mut argument = field_func(field_type)?;

                    for name_part in parts {
                        match name_part {
                            FieldNamePart::Attribute(attribute) => {
                                argument = vm.get_attribute(argument, attribute.as_str())?;
                            }
                            FieldNamePart::Index(index) => {
                                argument = argument.get_item(index, vm)?;
                            }
                            FieldNamePart::StringIndex(index) => {
                                argument = argument.get_item(&index, vm)?;
                            }
                        }
                    }

                    let nested_format =
                        FormatString::from_str(&format_spec).map_err(|e| e.into_pyexception(vm))?;
                    let format_spec = nested_format.format_internal(vm, field_func)?;

                    let value =
                        call_object_format(vm, argument, *preconversion_spec, &format_spec)?;
                    pystr::clone_value(&value)
                }
                FormatPart::Literal(literal) => literal.clone(),
            };
            final_string.push_str(&result_string);
        }
        Ok(final_string)
    }

    pub(crate) fn format(&self, arguments: &FuncArgs, vm: &VirtualMachine) -> PyResult<String> {
        let mut auto_argument_index: usize = 0;
        let mut seen_index = false;
        self.format_internal(vm, &mut |field_type| match field_type {
            FieldType::Auto => {
                if seen_index {
                    return Err(vm.new_value_error(
                        "cannot switch from manual field specification to automatic field numbering"
                            .to_owned(),
                    ));
                }
                auto_argument_index += 1;
                arguments
                    .args
                    .get(auto_argument_index - 1)
                    .cloned()
                    .ok_or_else(|| vm.new_index_error("tuple index out of range".to_owned()))
            }
            FieldType::Index(index) => {
                if auto_argument_index != 0 {
                    return Err(vm.new_value_error(
                        "cannot switch from automatic field numbering to manual field specification"
                            .to_owned(),
                    ));
                }
                seen_index = true;
                arguments
                    .args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| vm.new_index_error("tuple index out of range".to_owned()))
            }
            FieldType::Keyword(keyword) => arguments
                .get_optional_kwarg(&keyword)
                .ok_or_else(|| vm.new_key_error(vm.ctx.new_str(keyword))),
        })
    }

    pub(crate) fn format_map(&self, dict: &PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        self.format_internal(vm, &mut |field_type| match field_type {
            FieldType::Auto | FieldType::Index(_) => {
                Err(vm.new_value_error("Format string contains positional fields".to_owned()))
            }
            FieldType::Keyword(keyword) => dict.get_item(keyword, &vm),
        })
    }
}

fn call_object_format(
    vm: &VirtualMachine,
    argument: PyObjectRef,
    preconversion_spec: Option<char>,
    format_spec: &str,
) -> PyResult {
    let argument = match preconversion_spec.and_then(FormatPreconversor::from_char) {
        Some(FormatPreconversor::Str) => vm.to_str(&argument)?.into_object(),
        Some(FormatPreconversor::Repr) => vm.to_repr(&argument)?.into_object(),
        Some(FormatPreconversor::Ascii) => vm.ctx.new_str(builtins::ascii(argument, vm)?),
        Some(FormatPreconversor::Bytes) => vm.call_method(&argument, "decode", ())?,
        None => argument,
    };
    let result = vm.call_special_method(argument, "__format__", (format_spec,))?;
    if !result.isinstance(&vm.ctx.types.str_type) {
        return Err(vm.new_type_error(format!(
            "__format__ must return a str, not {}",
            &result.class().name
        )));
    }
    Ok(result)
}

pub(crate) trait FromTemplate<'a>: Sized {
    type Err;
    fn from_str(s: &'a str) -> Result<Self, Self::Err>;
}

impl<'a> FromTemplate<'a> for FormatString {
    type Err = FormatParseError;

    fn from_str(text: &'a str) -> Result<Self, Self::Err> {
        let mut cur_text: &str = text;
        let mut parts: Vec<FormatPart> = Vec::new();
        while !cur_text.is_empty() {
            // Try to parse both literals and bracketed format parts util we
            // run out of text
            cur_text = FormatString::parse_literal(cur_text)
                .or_else(|_| FormatString::parse_spec(cur_text))
                .map(|(part, new_text)| {
                    parts.push(part);
                    new_text
                })?;
        }
        Ok(FormatString {
            format_parts: parts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill_and_align() {
        assert_eq!(
            parse_fill_and_align(" <"),
            (Some(' '), Some(FormatAlign::Left), "")
        );
        assert_eq!(
            parse_fill_and_align(" <22"),
            (Some(' '), Some(FormatAlign::Left), "22")
        );
        assert_eq!(
            parse_fill_and_align("<22"),
            (None, Some(FormatAlign::Left), "22")
        );
        assert_eq!(
            parse_fill_and_align(" ^^"),
            (Some(' '), Some(FormatAlign::Center), "^")
        );
        assert_eq!(
            parse_fill_and_align("==="),
            (Some('='), Some(FormatAlign::AfterSign), "=")
        );
    }

    #[test]
    fn test_width_only() {
        let expected = Ok(FormatSpec {
            preconversor: None,
            fill: None,
            align: None,
            sign: None,
            alternate_form: false,
            width: Some(33),
            grouping_option: None,
            precision: None,
            format_type: None,
        });
        assert_eq!(parse_format_spec("33"), expected);
    }

    #[test]
    fn test_fill_and_width() {
        let expected = Ok(FormatSpec {
            preconversor: None,
            fill: Some('<'),
            align: Some(FormatAlign::Right),
            sign: None,
            alternate_form: false,
            width: Some(33),
            grouping_option: None,
            precision: None,
            format_type: None,
        });
        assert_eq!(parse_format_spec("<>33"), expected);
    }

    #[test]
    fn test_all() {
        let expected = Ok(FormatSpec {
            preconversor: None,
            fill: Some('<'),
            align: Some(FormatAlign::Right),
            sign: Some(FormatSign::Minus),
            alternate_form: true,
            width: Some(23),
            grouping_option: Some(FormatGrouping::Comma),
            precision: Some(11),
            format_type: Some(FormatType::Binary),
        });
        assert_eq!(parse_format_spec("<>-#23,.11b"), expected);
    }

    #[test]
    fn test_format_int() {
        assert_eq!(
            parse_format_spec("d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("16".to_owned())
        );
        assert_eq!(
            parse_format_spec("x")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("10".to_owned())
        );
        assert_eq!(
            parse_format_spec("b")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("10000".to_owned())
        );
        assert_eq!(
            parse_format_spec("o")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("20".to_owned())
        );
        assert_eq!(
            parse_format_spec("+d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("+16".to_owned())
        );
        assert_eq!(
            parse_format_spec("^ 5d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Minus, b"\x10")),
            Ok(" -16 ".to_owned())
        );
        assert_eq!(
            parse_format_spec("0>+#10x")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("00000+0x10".to_owned())
        );
    }

    #[test]
    fn test_format_parse() {
        let expected = Ok(FormatString {
            format_parts: vec![
                FormatPart::Literal("abcd".to_owned()),
                FormatPart::Field {
                    field_name: "1".to_owned(),
                    preconversion_spec: None,
                    format_spec: String::new(),
                },
                FormatPart::Literal(":".to_owned()),
                FormatPart::Field {
                    field_name: "key".to_owned(),
                    preconversion_spec: None,
                    format_spec: String::new(),
                },
            ],
        });

        assert_eq!(FormatString::from_str("abcd{1}:{key}"), expected);
    }

    #[test]
    fn test_format_parse_fail() {
        assert_eq!(
            FormatString::from_str("{s"),
            Err(FormatParseError::UnmatchedBracket)
        );
    }

    #[test]
    fn test_format_parse_escape() {
        let expected = Ok(FormatString {
            format_parts: vec![
                FormatPart::Literal("{".to_owned()),
                FormatPart::Field {
                    field_name: "key".to_owned(),
                    preconversion_spec: None,
                    format_spec: String::new(),
                },
                FormatPart::Literal("}ddfe".to_owned()),
            ],
        });

        assert_eq!(FormatString::from_str("{{{key}}}ddfe"), expected);
    }

    #[test]
    fn test_format_invalid_specification() {
        assert_eq!(parse_format_spec("%3"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec(".2fa"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec("ds"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec("x+"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec("b4"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec("o!"), Err("Invalid format specifier"));
        assert_eq!(parse_format_spec("d "), Err("Invalid format specifier"));
    }

    #[test]
    fn test_parse_field_name() {
        assert_eq!(
            FieldName::parse(""),
            Ok(FieldName {
                field_type: FieldType::Auto,
                parts: Vec::new(),
            })
        );
        assert_eq!(
            FieldName::parse("0"),
            Ok(FieldName {
                field_type: FieldType::Index(0),
                parts: Vec::new(),
            })
        );
        assert_eq!(
            FieldName::parse("key"),
            Ok(FieldName {
                field_type: FieldType::Keyword("key".to_owned()),
                parts: Vec::new(),
            })
        );
        assert_eq!(
            FieldName::parse("key.attr[0][string]"),
            Ok(FieldName {
                field_type: FieldType::Keyword("key".to_owned()),
                parts: vec![
                    FieldNamePart::Attribute("attr".to_owned()),
                    FieldNamePart::Index(0),
                    FieldNamePart::StringIndex("string".to_owned())
                ],
            })
        );
        assert_eq!(
            FieldName::parse("key.."),
            Err(FormatParseError::EmptyAttribute)
        );
        assert_eq!(
            FieldName::parse("key[]"),
            Err(FormatParseError::EmptyAttribute)
        );
        assert_eq!(
            FieldName::parse("key["),
            Err(FormatParseError::MissingRightBracket)
        );
        assert_eq!(
            FieldName::parse("key[0]after"),
            Err(FormatParseError::InvalidCharacterAfterRightBracket)
        );
    }
}

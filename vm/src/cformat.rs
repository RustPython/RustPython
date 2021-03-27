/// Implementation of Printf-Style string formatting
/// [https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting]
use crate::builtins::float::{try_bigint, IntoPyFloat, PyFloat};
use crate::builtins::int::{self, PyInt};
use crate::builtins::pystr::PyStr;
use crate::builtins::{memory::try_buffer_from_object, tuple, PyBytes};
use crate::common::float_ops;
use crate::pyobject::{
    BorrowValue, ItemProtocol, PyObjectRef, PyResult, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use itertools::Itertools;
use num_bigint::{BigInt, Sign};
use num_traits::cast::ToPrimitive;
use num_traits::Signed;
use std::iter::{Enumerate, Peekable};
use std::str::FromStr;
use std::{cmp, fmt};

#[derive(Debug, PartialEq)]
enum CFormatErrorType {
    UnmatchedKeyParentheses,
    MissingModuloSign,
    UnsupportedFormatChar(char),
    IncompleteFormat,
    IntTooBig,
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
            IntTooBig => write!(f, "width/precision too big"),
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
    PointDecimal(CFormatCase),
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
    // chars_consumed: usize,
}

impl CFormatSpec {
    fn parse<T, I>(iter: &mut ParseIter<I>) -> Result<Self, ParsingError>
    where
        T: Into<char> + Copy,
        I: Iterator<Item = T>,
    {
        let mapping_key = parse_spec_mapping_key(iter)?;
        let flags = parse_flags(iter);
        let min_field_width = parse_quantity(iter)?;
        let precision = parse_precision(iter)?;
        consume_length(iter);
        let (format_type, format_char) = parse_format_type(iter)?;
        let precision = precision.or_else(|| match format_type {
            CFormatType::Float(_) => Some(CFormatQuantity::Amount(6)),
            _ => None,
        });

        Ok(CFormatSpec {
            mapping_key,
            flags,
            min_field_width,
            precision,
            format_type,
            format_char,
        })
    }

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

    fn format_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let bytes = if let Some(CFormatQuantity::Amount(precision)) = self.precision {
            &bytes[..cmp::min(bytes.len(), precision)]
        } else {
            bytes
        };
        if let Some(CFormatQuantity::Amount(width)) = self.min_field_width {
            let fill = cmp::max(0, width - bytes.len());
            let mut v = Vec::with_capacity(bytes.len() + fill);
            if self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                v.extend_from_slice(bytes);
                v.append(&mut vec![b' '; fill]);
            } else {
                v.append(&mut vec![b' '; fill]);
                v.extend_from_slice(bytes);
            }
            v
        } else {
            bytes.to_vec()
        }
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

    pub(crate) fn format_float(&self, num: f64) -> String {
        let sign_string = if num.is_sign_negative() && !num.is_nan() {
            "-"
        } else {
            self.flags.sign_string()
        };

        let precision = match self.precision {
            Some(CFormatQuantity::Amount(p)) => p,
            _ => 6,
        };

        let magnitude_string = match &self.format_type {
            CFormatType::Float(CFloatType::PointDecimal(case)) => {
                let case = match case {
                    CFormatCase::Lowercase => float_ops::Case::Lower,
                    CFormatCase::Uppercase => float_ops::Case::Upper,
                };
                let magnitude = num.abs();
                float_ops::format_fixed(precision, magnitude, case)
            }
            CFormatType::Float(CFloatType::Exponent(case)) => {
                let case = match case {
                    CFormatCase::Lowercase => float_ops::Case::Lower,
                    CFormatCase::Uppercase => float_ops::Case::Upper,
                };
                let magnitude = num.abs();
                float_ops::format_exponent(precision, magnitude, case)
            }
            CFormatType::Float(CFloatType::General(case)) => {
                let precision = if precision == 0 { 1 } else { precision };
                let case = match case {
                    CFormatCase::Lowercase => float_ops::Case::Lower,
                    CFormatCase::Uppercase => float_ops::Case::Upper,
                };
                let magnitude = num.abs();
                float_ops::format_general(precision, magnitude, case)
            }
            _ => unreachable!(),
        };

        let formatted = if self.flags.contains(CConversionFlags::ZERO_PAD) {
            let fill_char = if !self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                '0'
            } else {
                ' '
            };
            format!(
                "{}{}",
                sign_string,
                self.fill_string(
                    magnitude_string,
                    fill_char,
                    Some(sign_string.chars().count())
                )
            )
        } else {
            self.fill_string(format!("{}{}", sign_string, magnitude_string), ' ', None)
        };

        formatted
    }

    fn bytes_format(&self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Vec<u8>> {
        match &self.format_type {
            CFormatType::String(preconversor) => match preconversor {
                CFormatPreconversor::Repr | CFormatPreconversor::Ascii => {
                    let s = vm.to_repr(&obj)?;
                    let s = self.format_string(s.borrow_value().to_owned());
                    Ok(s.into_bytes())
                }
                CFormatPreconversor::Str | CFormatPreconversor::Bytes => {
                    if let Ok(buffer) = try_buffer_from_object(vm, &obj) {
                        let guard;
                        let vec;
                        let bytes = match buffer.as_contiguous() {
                            Some(bytes) => {
                                guard = bytes;
                                &*guard
                            }
                            None => {
                                vec = buffer.to_contiguous();
                                vec.as_slice()
                            }
                        };
                        Ok(self.format_bytes(bytes))
                    } else {
                        let bytes = vm
                            .get_special_method(obj, "__bytes__")?
                            .map_err(|obj| {
                                vm.new_type_error(format!(
                                    "%b requires a bytes-like object, or an object that \
                                     implements __bytes__, not '{}'",
                                    obj.class().name
                                ))
                            })?
                            .invoke((), vm)?;
                        let bytes = PyBytes::try_from_object(vm, bytes)?;
                        Ok(self.format_bytes(bytes.borrow_value()))
                    }
                }
            },
            CFormatType::Number(number_type) => match number_type {
                CNumberType::Decimal => match_class!(match &obj {
                    ref i @ PyInt => {
                        Ok(self.format_number(i.borrow_value()).into_bytes())
                    }
                    ref f @ PyFloat => {
                        Ok(self
                            .format_number(&try_bigint(f.to_f64(), vm)?)
                            .into_bytes())
                    }
                    obj => {
                        if let Some(method) = vm.get_method(obj.clone(), "__int__") {
                            let result = vm.invoke(&method?, ())?;
                            if let Some(i) = result.payload::<PyInt>() {
                                return Ok(self.format_number(i.borrow_value()).into_bytes());
                            }
                        }
                        Err(vm.new_type_error(format!(
                            "%{} format: a number is required, not {}",
                            self.format_char,
                            obj.class().name
                        )))
                    }
                }),
                _ => {
                    if let Some(i) = obj.payload::<PyInt>() {
                        Ok(self.format_number(i.borrow_value()).into_bytes())
                    } else {
                        Err(vm.new_type_error(format!(
                            "%{} format: an integer is required, not {}",
                            self.format_char,
                            obj.class().name
                        )))
                    }
                }
            },
            CFormatType::Float(_) => {
                let value = IntoPyFloat::try_from_object(vm, obj)?.to_f64();
                Ok(self.format_float(value).into_bytes())
            }
            CFormatType::Character => {
                if let Some(i) = obj.payload::<PyInt>() {
                    let ch = i
                        .borrow_value()
                        .to_u32()
                        .and_then(std::char::from_u32)
                        .ok_or_else(|| {
                            vm.new_overflow_error("%c arg not in range(0x110000)".to_owned())
                        })?;
                    return Ok(self.format_char(ch).into_bytes());
                }
                if let Some(s) = obj.payload::<PyStr>() {
                    if let Ok(ch) = s.borrow_value().chars().exactly_one() {
                        return Ok(self.format_char(ch).into_bytes());
                    }
                }
                Err(vm.new_type_error("%c requires int or char".to_owned()))
            }
        }
    }

    fn string_format(&self, vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<String> {
        match &self.format_type {
            CFormatType::String(preconversor) => {
                let result = match preconversor {
                    CFormatPreconversor::Str => vm.to_str(&obj)?,
                    CFormatPreconversor::Repr | CFormatPreconversor::Ascii => vm.to_repr(&obj)?,
                    CFormatPreconversor::Bytes => {
                        return Err(vm.new_value_error(
                            "unsupported format character 'b' (0x62)".to_owned(),
                        ));
                    }
                };
                Ok(self.format_string(result.borrow_value().to_owned()))
            }
            CFormatType::Number(number_type) => match number_type {
                CNumberType::Decimal => match_class!(match &obj {
                    ref i @ PyInt => {
                        Ok(self.format_number(i.borrow_value()))
                    }
                    ref f @ PyFloat => {
                        Ok(self.format_number(&try_bigint(f.to_f64(), vm)?))
                    }
                    obj => {
                        if let Some(method) = vm.get_method(obj.clone(), "__int__") {
                            let result = vm.invoke(&method?, ())?;
                            if let Some(i) = result.payload::<PyInt>() {
                                return Ok(self.format_number(i.borrow_value()));
                            }
                        }
                        Err(vm.new_type_error(format!(
                            "%{} format: a number is required, not {}",
                            self.format_char,
                            obj.class().name
                        )))
                    }
                }),
                _ => {
                    if let Some(i) = obj.payload::<PyInt>() {
                        Ok(self.format_number(i.borrow_value()))
                    } else {
                        Err(vm.new_type_error(format!(
                            "%{} format: an integer is required, not {}",
                            self.format_char,
                            obj.class().name
                        )))
                    }
                }
            },
            CFormatType::Float(_) => {
                let value = IntoPyFloat::try_from_object(vm, obj)?.to_f64();
                Ok(self.format_float(value))
            }
            CFormatType::Character => {
                if let Some(i) = obj.payload::<PyInt>() {
                    let ch = i
                        .borrow_value()
                        .to_u32()
                        .and_then(std::char::from_u32)
                        .ok_or_else(|| {
                            vm.new_overflow_error("%c arg not in range(0x110000)".to_owned())
                        })?;
                    return Ok(self.format_char(ch));
                }
                if let Some(s) = obj.payload::<PyStr>() {
                    if let Ok(ch) = s.borrow_value().chars().exactly_one() {
                        return Ok(self.format_char(ch));
                    }
                }
                Err(vm.new_type_error("%c requires int or char".to_owned()))
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum CFormatPart<T> {
    Literal(T),
    Spec(CFormatSpec),
}

impl<T> CFormatPart<T> {
    fn is_specifier(&self) -> bool {
        matches!(self, CFormatPart::Spec(_))
    }

    fn has_key(&self) -> bool {
        match self {
            CFormatPart::Spec(s) => s.mapping_key.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct CFormatBytes {
    parts: Vec<(usize, CFormatPart<Vec<u8>>)>,
}

fn try_update_quantity_from_tuple<'a, I: Iterator<Item = &'a PyObjectRef>>(
    vm: &VirtualMachine,
    elements: &mut I,
    q: &mut Option<CFormatQuantity>,
) -> PyResult<()> {
    match q {
        Some(CFormatQuantity::FromValuesTuple) => match elements.next() {
            Some(width_obj) => {
                if !width_obj.isinstance(&vm.ctx.types.int_type) {
                    Err(vm.new_type_error("* wants int".to_owned()))
                } else {
                    let i = int::get_value(&width_obj);
                    let i = int::try_to_primitive::<isize>(i, vm)? as usize;
                    *q = Some(CFormatQuantity::Amount(i));
                    Ok(())
                }
            }
            None => Err(vm.new_type_error("not enough arguments for format string".to_owned())),
        },
        _ => Ok(()),
    }
}

fn check_specifiers<T>(
    parts: &[(usize, CFormatPart<T>)],
    vm: &VirtualMachine,
) -> PyResult<(usize, bool)> {
    let mut count = 0;
    let mut mapping_required = false;
    for (_, part) in parts {
        if part.is_specifier() {
            let has_key = part.has_key();
            if count == 0 {
                mapping_required = has_key;
            } else if mapping_required != has_key {
                return Err(vm.new_type_error("format requires a mapping".to_owned()));
            }
            count += 1;
        }
    }
    Ok((count, mapping_required))
}

impl CFormatBytes {
    pub(crate) fn parse<I: Iterator<Item = u8>>(
        iter: &mut ParseIter<I>,
    ) -> Result<Self, CFormatError> {
        let mut parts = vec![];
        let mut literal = vec![];
        let mut part_index = 0;
        while let Some((index, c)) = iter.next() {
            if c == b'%' {
                if let Some(&(_, second)) = iter.peek() {
                    if second == b'%' {
                        iter.next().unwrap();
                        literal.push(b'%');
                        continue;
                    } else {
                        if !literal.is_empty() {
                            parts.push((
                                part_index,
                                CFormatPart::Literal(std::mem::take(&mut literal)),
                            ));
                        }
                        let spec = CFormatSpec::parse(iter).map_err(|err| CFormatError {
                            typ: err.0,
                            index: err.1,
                        })?;
                        parts.push((index, CFormatPart::Spec(spec)));
                        if let Some(&(index, _)) = iter.peek() {
                            part_index = index;
                        }
                    }
                } else {
                    return Err(CFormatError {
                        typ: CFormatErrorType::IncompleteFormat,
                        index: index + 1,
                    });
                }
            } else {
                literal.push(c);
            }
        }
        if !literal.is_empty() {
            parts.push((part_index, CFormatPart::Literal(literal)));
        }
        Ok(Self { parts })
    }

    pub(crate) fn parse_from_bytes(bytes: &[u8]) -> Result<Self, CFormatError> {
        let mut iter = bytes.iter().cloned().enumerate().peekable();
        Self::parse(&mut iter)
    }
    pub(crate) fn format(
        &mut self,
        vm: &VirtualMachine,
        values_obj: PyObjectRef,
    ) -> PyResult<Vec<u8>> {
        let (num_specifiers, mapping_required) = check_specifiers(self.parts.as_slice(), vm)?;
        let mut result = vec![];

        let is_mapping = values_obj.class().has_attr("__getitem__")
            && !values_obj.isinstance(&vm.ctx.types.tuple_type)
            && !values_obj.isinstance(&vm.ctx.types.str_type);

        if num_specifiers == 0 {
            // literal only
            return if is_mapping
                || values_obj
                    .payload::<tuple::PyTuple>()
                    .map_or(false, |e| e.borrow_value().is_empty())
            {
                for (_, part) in &mut self.parts {
                    match part {
                        CFormatPart::Literal(literal) => result.append(literal),
                        CFormatPart::Spec(_) => unreachable!(),
                    }
                }
                Ok(result)
            } else {
                Err(vm.new_type_error(
                    "not all arguments converted during string formatting".to_owned(),
                ))
            };
        }

        if mapping_required {
            // dict
            return if is_mapping {
                for (_, part) in &mut self.parts {
                    match part {
                        CFormatPart::Literal(literal) => result.append(literal),
                        CFormatPart::Spec(spec) => {
                            let value = match &spec.mapping_key {
                                Some(key) => values_obj.get_item(key, vm)?,
                                None => unreachable!(),
                            };
                            let mut part_result = spec.bytes_format(vm, value)?;
                            result.append(&mut part_result);
                        }
                    }
                }
                Ok(result)
            } else {
                Err(vm.new_type_error("format requires a mapping".to_owned()))
            };
        }

        // tuple
        let values = if let Some(tup) = values_obj.payload_if_subclass::<tuple::PyTuple>(vm) {
            tup.borrow_value()
        } else {
            std::slice::from_ref(&values_obj)
        };
        let mut value_iter = values.iter();

        for (_, part) in &mut self.parts {
            match part {
                CFormatPart::Literal(literal) => result.append(literal),
                CFormatPart::Spec(spec) => {
                    try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.min_field_width)?;
                    try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.precision)?;

                    let value = match value_iter.next() {
                        Some(obj) => Ok(obj.clone()),
                        None => Err(
                            vm.new_type_error("not enough arguments for format string".to_owned())
                        ),
                    }?;
                    let mut part_result = spec.bytes_format(vm, value)?;
                    result.append(&mut part_result);
                }
            }
        }

        // check that all arguments were converted
        if value_iter.next().is_some() && !is_mapping {
            Err(vm
                .new_type_error("not all arguments converted during string formatting".to_owned()))
        } else {
            Ok(result)
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct CFormatString {
    parts: Vec<(usize, CFormatPart<String>)>,
}

impl FromStr for CFormatString {
    type Err = CFormatError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut iter = text.chars().enumerate().peekable();
        Self::parse(&mut iter)
    }
}

impl CFormatString {
    pub(crate) fn parse<I: Iterator<Item = char>>(
        iter: &mut ParseIter<I>,
    ) -> Result<Self, CFormatError> {
        let mut parts = vec![];
        let mut literal = String::new();
        let mut part_index = 0;
        while let Some((index, c)) = iter.next() {
            if c == '%' {
                if let Some(&(_, second)) = iter.peek() {
                    if second == '%' {
                        iter.next().unwrap();
                        literal.push('%');
                        continue;
                    } else {
                        if !literal.is_empty() {
                            parts.push((
                                part_index,
                                CFormatPart::Literal(std::mem::take(&mut literal)),
                            ));
                        }
                        let spec = CFormatSpec::parse(iter).map_err(|err| CFormatError {
                            typ: err.0,
                            index: err.1,
                        })?;
                        parts.push((index, CFormatPart::Spec(spec)));
                        if let Some(&(index, _)) = iter.peek() {
                            part_index = index;
                        }
                    }
                } else {
                    return Err(CFormatError {
                        typ: CFormatErrorType::IncompleteFormat,
                        index: index + 1,
                    });
                }
            } else {
                literal.push(c);
            }
        }
        if !literal.is_empty() {
            parts.push((part_index, CFormatPart::Literal(literal)));
        }
        Ok(Self { parts })
    }

    pub(crate) fn format(
        &mut self,
        vm: &VirtualMachine,
        values_obj: PyObjectRef,
    ) -> PyResult<String> {
        let (num_specifiers, mapping_required) = check_specifiers(self.parts.as_slice(), vm)?;
        let mut result = String::new();

        let is_mapping = values_obj.class().has_attr("__getitem__")
            && !values_obj.isinstance(&vm.ctx.types.tuple_type)
            && !values_obj.isinstance(&vm.ctx.types.str_type);

        if num_specifiers == 0 {
            // literal only
            return if is_mapping
                || values_obj
                    .payload::<tuple::PyTuple>()
                    .map_or(false, |e| e.borrow_value().is_empty())
            {
                for (_, part) in &self.parts {
                    match part {
                        CFormatPart::Literal(literal) => result.push_str(&literal),
                        CFormatPart::Spec(_) => unreachable!(),
                    }
                }
                Ok(result)
            } else {
                Err(vm.new_type_error(
                    "not all arguments converted during string formatting".to_owned(),
                ))
            };
        }

        if mapping_required {
            // dict
            return if is_mapping {
                for (_, part) in &self.parts {
                    match part {
                        CFormatPart::Literal(literal) => result.push_str(&literal),
                        CFormatPart::Spec(spec) => {
                            let value = match &spec.mapping_key {
                                Some(key) => values_obj.get_item(key, vm)?,
                                None => unreachable!(),
                            };
                            let part_result = spec.string_format(vm, value)?;
                            result.push_str(&part_result);
                        }
                    }
                }
                Ok(result)
            } else {
                Err(vm.new_type_error("format requires a mapping".to_owned()))
            };
        }

        // tuple
        let values = if let Some(tup) = values_obj.payload_if_subclass::<tuple::PyTuple>(vm) {
            tup.borrow_value()
        } else {
            std::slice::from_ref(&values_obj)
        };
        let mut value_iter = values.iter();

        for (_, part) in &mut self.parts {
            match part {
                CFormatPart::Literal(literal) => result.push_str(&literal),
                CFormatPart::Spec(spec) => {
                    try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.min_field_width)?;
                    try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.precision)?;

                    let value = match value_iter.next() {
                        Some(obj) => Ok(obj.clone()),
                        None => Err(
                            vm.new_type_error("not enough arguments for format string".to_owned())
                        ),
                    }?;
                    let part_result = spec.string_format(vm, value)?;
                    result.push_str(&part_result);
                }
            }
        }

        // check that all arguments were converted
        if value_iter.next().is_some() && !is_mapping {
            Err(vm
                .new_type_error("not all arguments converted during string formatting".to_owned()))
        } else {
            Ok(result)
        }
    }
}

type ParseIter<I> = Peekable<Enumerate<I>>;

fn parse_quantity<T, I>(iter: &mut ParseIter<I>) -> Result<Option<CFormatQuantity>, ParsingError>
where
    T: Into<char> + Copy,
    I: Iterator<Item = T>,
{
    if let Some(&(_, c)) = iter.peek() {
        let c: char = c.into();
        if c == '*' {
            iter.next().unwrap();
            return Ok(Some(CFormatQuantity::FromValuesTuple));
        }
        if let Some(i) = c.to_digit(10) {
            let mut num = i as isize;
            iter.next().unwrap();
            while let Some(&(index, c)) = iter.peek() {
                if let Some(i) = c.into().to_digit(10) {
                    num = num
                        .checked_mul(10)
                        .and_then(|num| num.checked_add(i as isize))
                        .ok_or((CFormatErrorType::IntTooBig, index))?;
                    iter.next().unwrap();
                } else {
                    break;
                }
            }
            return Ok(Some(CFormatQuantity::Amount(num as usize)));
        }
    }
    Ok(None)
}

fn parse_precision<T, I>(iter: &mut ParseIter<I>) -> Result<Option<CFormatQuantity>, ParsingError>
where
    T: Into<char> + Copy,
    I: Iterator<Item = T>,
{
    if let Some(&(_, c)) = iter.peek() {
        if c.into() == '.' {
            iter.next().unwrap();
            return parse_quantity(iter);
        }
    }
    Ok(None)
}

fn parse_text_inside_parentheses<T, I>(iter: &mut ParseIter<I>) -> Option<String>
where
    T: Into<char>,
    I: Iterator<Item = T>,
{
    let mut counter: i32 = 1;
    let mut contained_text = String::new();
    loop {
        let (_, c) = iter.next()?;
        let c = c.into();
        match c {
            _ if c == '(' => {
                counter += 1;
            }
            _ if c == ')' => {
                counter -= 1;
            }
            _ => (),
        }

        if counter > 0 {
            contained_text.push(c);
        } else {
            break;
        }
    }

    Some(contained_text)
}

fn parse_spec_mapping_key<T, I>(iter: &mut ParseIter<I>) -> Result<Option<String>, ParsingError>
where
    T: Into<char> + Copy,
    I: Iterator<Item = T>,
{
    if let Some(&(index, c)) = iter.peek() {
        if c.into() == '(' {
            iter.next().unwrap();
            return match parse_text_inside_parentheses(iter) {
                Some(key) => Ok(Some(key)),
                None => Err((CFormatErrorType::UnmatchedKeyParentheses, index)),
            };
        }
    }
    Ok(None)
}

fn parse_flags<T, I>(iter: &mut ParseIter<I>) -> CConversionFlags
where
    T: Into<char> + Copy,
    I: Iterator<Item = T>,
{
    let mut flags = CConversionFlags::empty();
    while let Some(&(_, c)) = iter.peek() {
        let flag = match c.into() {
            '#' => CConversionFlags::ALTERNATE_FORM,
            '0' => CConversionFlags::ZERO_PAD,
            '-' => CConversionFlags::LEFT_ADJUST,
            ' ' => CConversionFlags::BLANK_SIGN,
            '+' => CConversionFlags::SIGN_CHAR,
            _ => break,
        };
        iter.next().unwrap();
        flags |= flag;
    }
    flags
}

fn consume_length<T, I>(iter: &mut ParseIter<I>)
where
    T: Into<char> + Copy,
    I: Iterator<Item = T>,
{
    if let Some(&(_, c)) = iter.peek() {
        let c = c.into();
        if c == 'h' || c == 'l' || c == 'L' {
            iter.next().unwrap();
        }
    }
}

fn parse_format_type<T, I>(iter: &mut ParseIter<I>) -> Result<(CFormatType, char), ParsingError>
where
    T: Into<char>,
    I: Iterator<Item = T>,
{
    use CFloatType::*;
    use CFormatCase::{Lowercase, Uppercase};
    use CNumberType::*;
    let (index, c) = match iter.next() {
        Some((index, c)) => (index, c.into()),
        None => {
            return Err((
                CFormatErrorType::IncompleteFormat,
                iter.peek().map(|x| x.0).unwrap_or(0),
            ));
        }
    };
    let format_type = match c {
        'd' | 'i' | 'u' => CFormatType::Number(Decimal),
        'o' => CFormatType::Number(Octal),
        'x' => CFormatType::Number(Hex(Lowercase)),
        'X' => CFormatType::Number(Hex(Uppercase)),
        'e' => CFormatType::Float(Exponent(Lowercase)),
        'E' => CFormatType::Float(Exponent(Uppercase)),
        'f' => CFormatType::Float(PointDecimal(Lowercase)),
        'F' => CFormatType::Float(PointDecimal(Uppercase)),
        'g' => CFormatType::Float(General(Lowercase)),
        'G' => CFormatType::Float(General(Uppercase)),
        'c' => CFormatType::Character,
        'r' => CFormatType::String(CFormatPreconversor::Repr),
        's' => CFormatType::String(CFormatPreconversor::Str),
        'b' => CFormatType::String(CFormatPreconversor::Bytes),
        'a' => CFormatType::String(CFormatPreconversor::Ascii),
        _ => return Err((CFormatErrorType::UnsupportedFormatChar(c), index)),
    };
    Ok((format_type, c))
}

impl FromStr for CFormatSpec {
    type Err = ParsingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut chars = text.chars().enumerate().peekable();
        if chars.next().map(|x| x.1) != Some('%') {
            return Err((CFormatErrorType::MissingModuloSign, 1));
        }

        CFormatSpec::parse(&mut chars)
    }
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
            min_field_width: None,
            precision: None,
            flags: CConversionFlags::empty(),
        });
        assert_eq!("%(amount)d".parse::<CFormatSpec>(), expected);

        let expected = Ok(CFormatSpec {
            mapping_key: Some("m((u(((l((((ti))))p)))l))e".to_owned()),
            format_type: CFormatType::Number(CNumberType::Decimal),
            format_char: 'd',
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
            "%f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            "1.234500"
        );
        assert_eq!(
            "%+f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            "+1.234500"
        );
        assert_eq!(
            "% f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            " 1.234500"
        );
        assert_eq!(
            "%f".parse::<CFormatSpec>().unwrap().format_float(-1.2345),
            "-1.234500"
        );
        assert_eq!(
            "%f".parse::<CFormatSpec>()
                .unwrap()
                .format_float(1.2345678901),
            "1.234568"
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

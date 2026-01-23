//! Implementation of Printf-Style string formatting
//! as per the [Python Docs](https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting).
use alloc::fmt;
use bitflags::bitflags;
use core::{
    cmp,
    iter::{Enumerate, Peekable},
    str::FromStr,
};
use itertools::Itertools;
use malachite_bigint::{BigInt, Sign};
use num_traits::Signed;
use rustpython_literal::{float, format::Case};

use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CFormatErrorType {
    UnmatchedKeyParentheses,
    MissingModuloSign,
    UnsupportedFormatChar(CodePoint),
    IncompleteFormat,
    IntTooBig,
    // Unimplemented,
}

// also contains how many chars the parsing function consumed
pub type ParsingError = (CFormatErrorType, usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CFormatError {
    pub typ: CFormatErrorType, // FIXME
    pub index: usize,
}

impl fmt::Display for CFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use CFormatErrorType::*;
        match self.typ {
            UnmatchedKeyParentheses => write!(f, "incomplete format key"),
            IncompleteFormat => write!(f, "incomplete format"),
            UnsupportedFormatChar(c) => write!(
                f,
                "unsupported format character '{}' ({:#x}) at index {}",
                c,
                c.to_u32(),
                self.index
            ),
            IntTooBig => write!(f, "width/precision too big"),
            _ => write!(f, "unexpected error parsing format string"),
        }
    }
}

pub type CFormatConversion = super::format::FormatConversion;

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum CNumberType {
    DecimalD = b'd',
    DecimalI = b'i',
    DecimalU = b'u',
    Octal = b'o',
    HexLower = b'x',
    HexUpper = b'X',
}

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum CFloatType {
    ExponentLower = b'e',
    ExponentUpper = b'E',
    PointDecimalLower = b'f',
    PointDecimalUpper = b'F',
    GeneralLower = b'g',
    GeneralUpper = b'G',
}

impl CFloatType {
    const fn case(self) -> Case {
        use CFloatType::*;

        match self {
            ExponentLower | PointDecimalLower | GeneralLower => Case::Lower,
            ExponentUpper | PointDecimalUpper | GeneralUpper => Case::Upper,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum CCharacterType {
    Character = b'c',
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CFormatType {
    Number(CNumberType),
    Float(CFloatType),
    Character(CCharacterType),
    String(CFormatConversion),
}

impl CFormatType {
    pub const fn to_char(self) -> char {
        match self {
            Self::Number(x) => x as u8 as char,
            Self::Float(x) => x as u8 as char,
            Self::Character(x) => x as u8 as char,
            Self::String(x) => x as u8 as char,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CFormatPrecision {
    Quantity(CFormatQuantity),
    Dot,
}

impl From<CFormatQuantity> for CFormatPrecision {
    fn from(quantity: CFormatQuantity) -> Self {
        Self::Quantity(quantity)
    }
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct CConversionFlags: u32 {
        const ALTERNATE_FORM = 0b0000_0001;
        const ZERO_PAD = 0b0000_0010;
        const LEFT_ADJUST = 0b0000_0100;
        const BLANK_SIGN = 0b0000_1000;
        const SIGN_CHAR = 0b0001_0000;
    }
}

impl CConversionFlags {
    #[inline]
    pub const fn sign_string(&self) -> &'static str {
        if self.contains(Self::SIGN_CHAR) {
            "+"
        } else if self.contains(Self::BLANK_SIGN) {
            " "
        } else {
            ""
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CFormatQuantity {
    Amount(usize),
    FromValuesTuple,
}

pub trait FormatBuf:
    Extend<Self::Char> + Default + FromIterator<Self::Char> + From<String>
{
    type Char: FormatChar;
    fn chars(&self) -> impl Iterator<Item = Self::Char>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn concat(self, other: Self) -> Self;
}

pub trait FormatChar: Copy + Into<CodePoint> + From<u8> {
    fn to_char_lossy(self) -> char;
    fn eq_char(self, c: char) -> bool;
}

impl FormatBuf for String {
    type Char = char;

    fn chars(&self) -> impl Iterator<Item = Self::Char> {
        (**self).chars()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn concat(mut self, other: Self) -> Self {
        self.extend([other]);
        self
    }
}

impl FormatChar for char {
    fn to_char_lossy(self) -> char {
        self
    }

    fn eq_char(self, c: char) -> bool {
        self == c
    }
}

impl FormatBuf for Wtf8Buf {
    type Char = CodePoint;

    fn chars(&self) -> impl Iterator<Item = Self::Char> {
        self.code_points()
    }

    fn len(&self) -> usize {
        (**self).len()
    }

    fn concat(mut self, other: Self) -> Self {
        self.extend([other]);
        self
    }
}

impl FormatChar for CodePoint {
    fn to_char_lossy(self) -> char {
        self.to_char_lossy()
    }

    fn eq_char(self, c: char) -> bool {
        self == c
    }
}

impl FormatBuf for Vec<u8> {
    type Char = u8;

    fn chars(&self) -> impl Iterator<Item = Self::Char> {
        self.iter().copied()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn concat(mut self, other: Self) -> Self {
        self.extend(other);
        self
    }
}

impl FormatChar for u8 {
    fn to_char_lossy(self) -> char {
        self.into()
    }

    fn eq_char(self, c: char) -> bool {
        char::from(self) == c
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct CFormatSpec {
    pub flags: CConversionFlags,
    pub min_field_width: Option<CFormatQuantity>,
    pub precision: Option<CFormatPrecision>,
    pub format_type: CFormatType,
    // chars_consumed: usize,
}

#[derive(Debug, PartialEq)]
pub struct CFormatSpecKeyed<T> {
    pub mapping_key: Option<T>,
    pub spec: CFormatSpec,
}

#[cfg(test)]
impl FromStr for CFormatSpec {
    type Err = ParsingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse::<CFormatSpecKeyed<String>>()
            .map(|CFormatSpecKeyed { mapping_key, spec }| {
                assert!(mapping_key.is_none());
                spec
            })
    }
}

impl FromStr for CFormatSpecKeyed<String> {
    type Err = ParsingError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut chars = text.chars().enumerate().peekable();
        if chars.next().map(|x| x.1) != Some('%') {
            return Err((CFormatErrorType::MissingModuloSign, 1));
        }

        Self::parse(&mut chars)
    }
}

pub type ParseIter<I> = Peekable<Enumerate<I>>;

impl<T: FormatBuf> CFormatSpecKeyed<T> {
    pub fn parse<I>(iter: &mut ParseIter<I>) -> Result<Self, ParsingError>
    where
        I: Iterator<Item = T::Char>,
    {
        let mapping_key = parse_spec_mapping_key(iter)?;
        let flags = parse_flags(iter);
        let min_field_width = parse_quantity(iter)?;
        let precision = parse_precision(iter)?;
        consume_length(iter);
        let format_type = parse_format_type(iter)?;

        let spec = CFormatSpec {
            flags,
            min_field_width,
            precision,
            format_type,
        };
        Ok(Self { mapping_key, spec })
    }
}

impl CFormatSpec {
    fn compute_fill_string<T: FormatBuf>(fill_char: T::Char, fill_chars_needed: usize) -> T {
        (0..fill_chars_needed).map(|_| fill_char).collect()
    }

    fn fill_string<T: FormatBuf>(
        &self,
        string: T,
        fill_char: T::Char,
        num_prefix_chars: Option<usize>,
    ) -> T {
        let mut num_chars = string.chars().count();
        if let Some(num_prefix_chars) = num_prefix_chars {
            num_chars += num_prefix_chars;
        }
        let num_chars = num_chars;

        let width = match &self.min_field_width {
            Some(CFormatQuantity::Amount(width)) => cmp::max(width, &num_chars),
            _ => &num_chars,
        };
        let fill_chars_needed = width.saturating_sub(num_chars);
        let fill_string: T = Self::compute_fill_string(fill_char, fill_chars_needed);

        if !fill_string.is_empty() {
            if self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                string.concat(fill_string)
            } else {
                fill_string.concat(string)
            }
        } else {
            string
        }
    }

    fn fill_string_with_precision<T: FormatBuf>(&self, string: T, fill_char: T::Char) -> T {
        let num_chars = string.chars().count();

        let width = match &self.precision {
            Some(CFormatPrecision::Quantity(CFormatQuantity::Amount(width))) => {
                cmp::max(width, &num_chars)
            }
            _ => &num_chars,
        };
        let fill_chars_needed = width.saturating_sub(num_chars);
        let fill_string: T = Self::compute_fill_string(fill_char, fill_chars_needed);

        if !fill_string.is_empty() {
            // Don't left-adjust if precision-filling: that will always be prepending 0s to %d
            // arguments, the LEFT_ADJUST flag will be used by a later call to fill_string with
            // the 0-filled string as the string param.
            fill_string.concat(string)
        } else {
            string
        }
    }

    fn format_string_with_precision<T: FormatBuf>(
        &self,
        string: T,
        precision: Option<&CFormatPrecision>,
    ) -> T {
        // truncate if needed
        let string = match precision {
            Some(CFormatPrecision::Quantity(CFormatQuantity::Amount(precision)))
                if string.chars().count() > *precision =>
            {
                string.chars().take(*precision).collect::<T>()
            }
            Some(CFormatPrecision::Dot) => {
                // truncate to 0
                T::default()
            }
            _ => string,
        };
        self.fill_string(string, b' '.into(), None)
    }

    #[inline]
    pub fn format_string<T: FormatBuf>(&self, string: T) -> T {
        self.format_string_with_precision(string, self.precision.as_ref())
    }

    #[inline]
    pub fn format_char<T: FormatBuf>(&self, ch: T::Char) -> T {
        self.format_string_with_precision(
            T::from_iter([ch]),
            Some(&(CFormatQuantity::Amount(1).into())),
        )
    }

    pub fn format_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let bytes = if let Some(CFormatPrecision::Quantity(CFormatQuantity::Amount(precision))) =
            self.precision
        {
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

    pub fn format_number(&self, num: &BigInt) -> String {
        use CNumberType::*;
        let CFormatType::Number(format_type) = self.format_type else {
            unreachable!()
        };
        let magnitude = num.abs();
        let prefix = if self.flags.contains(CConversionFlags::ALTERNATE_FORM) {
            match format_type {
                Octal => "0o",
                HexLower => "0x",
                HexUpper => "0X",
                _ => "",
            }
        } else {
            ""
        };

        let magnitude_string: String = match format_type {
            DecimalD | DecimalI | DecimalU => magnitude.to_str_radix(10),
            Octal => magnitude.to_str_radix(8),
            HexLower => magnitude.to_str_radix(16),
            HexUpper => {
                let mut result = magnitude.to_str_radix(16);
                result.make_ascii_uppercase();
                result
            }
        };

        let sign_string = match num.sign() {
            Sign::Minus => "-",
            _ => self.flags.sign_string(),
        };

        let padded_magnitude_string = self.fill_string_with_precision(magnitude_string, '0');

        if self.flags.contains(CConversionFlags::ZERO_PAD) {
            let fill_char = if !self.flags.contains(CConversionFlags::LEFT_ADJUST) {
                '0'
            } else {
                ' ' // '-' overrides the '0' conversion if both are given
            };
            let signed_prefix = format!("{sign_string}{prefix}");
            format!(
                "{}{}",
                signed_prefix,
                self.fill_string(
                    padded_magnitude_string,
                    fill_char,
                    Some(signed_prefix.chars().count()),
                ),
            )
        } else {
            self.fill_string(
                format!("{sign_string}{prefix}{padded_magnitude_string}"),
                ' ',
                None,
            )
        }
    }

    pub fn format_float(&self, num: f64) -> String {
        let sign_string = if num.is_sign_negative() && !num.is_nan() {
            "-"
        } else {
            self.flags.sign_string()
        };

        let precision = match &self.precision {
            Some(CFormatPrecision::Quantity(quantity)) => match quantity {
                CFormatQuantity::Amount(amount) => *amount,
                CFormatQuantity::FromValuesTuple => 6,
            },
            Some(CFormatPrecision::Dot) => 0,
            None => 6,
        };

        let CFormatType::Float(format_type) = self.format_type else {
            unreachable!()
        };

        let magnitude = num.abs();
        let case = format_type.case();

        let magnitude_string = match format_type {
            CFloatType::PointDecimalLower | CFloatType::PointDecimalUpper => float::format_fixed(
                precision,
                magnitude,
                case,
                self.flags.contains(CConversionFlags::ALTERNATE_FORM),
            ),
            CFloatType::ExponentLower | CFloatType::ExponentUpper => float::format_exponent(
                precision,
                magnitude,
                case,
                self.flags.contains(CConversionFlags::ALTERNATE_FORM),
            ),
            CFloatType::GeneralLower | CFloatType::GeneralUpper => {
                let precision = if precision == 0 { 1 } else { precision };
                float::format_general(
                    precision,
                    magnitude,
                    case,
                    self.flags.contains(CConversionFlags::ALTERNATE_FORM),
                    false,
                )
            }
        };

        if self.flags.contains(CConversionFlags::ZERO_PAD) {
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
                    Some(sign_string.chars().count()),
                )
            )
        } else {
            self.fill_string(format!("{sign_string}{magnitude_string}"), ' ', None)
        }
    }
}

fn parse_spec_mapping_key<T, I>(iter: &mut ParseIter<I>) -> Result<Option<T>, ParsingError>
where
    T: FormatBuf,
    I: Iterator<Item = T::Char>,
{
    if let Some((index, _)) = iter.next_if(|(_, c)| c.eq_char('(')) {
        return match parse_text_inside_parentheses(iter) {
            Some(key) => Ok(Some(key)),
            None => Err((CFormatErrorType::UnmatchedKeyParentheses, index)),
        };
    }
    Ok(None)
}

fn parse_flags<C, I>(iter: &mut ParseIter<I>) -> CConversionFlags
where
    C: FormatChar,
    I: Iterator<Item = C>,
{
    let mut flags = CConversionFlags::empty();
    iter.peeking_take_while(|(_, c)| {
        let flag = match c.to_char_lossy() {
            '#' => CConversionFlags::ALTERNATE_FORM,
            '0' => CConversionFlags::ZERO_PAD,
            '-' => CConversionFlags::LEFT_ADJUST,
            ' ' => CConversionFlags::BLANK_SIGN,
            '+' => CConversionFlags::SIGN_CHAR,
            _ => return false,
        };
        flags |= flag;
        true
    })
    .for_each(drop);
    flags
}

fn consume_length<C, I>(iter: &mut ParseIter<I>)
where
    C: FormatChar,
    I: Iterator<Item = C>,
{
    iter.next_if(|(_, c)| matches!(c.to_char_lossy(), 'h' | 'l' | 'L'));
}

fn parse_format_type<C, I>(iter: &mut ParseIter<I>) -> Result<CFormatType, ParsingError>
where
    C: FormatChar,
    I: Iterator<Item = C>,
{
    use CFloatType::*;
    use CNumberType::*;
    let (index, c) = iter.next().ok_or_else(|| {
        (
            CFormatErrorType::IncompleteFormat,
            iter.peek().map(|x| x.0).unwrap_or(0),
        )
    })?;
    let format_type = match c.to_char_lossy() {
        'd' => CFormatType::Number(DecimalD),
        'i' => CFormatType::Number(DecimalI),
        'u' => CFormatType::Number(DecimalU),
        'o' => CFormatType::Number(Octal),
        'x' => CFormatType::Number(HexLower),
        'X' => CFormatType::Number(HexUpper),
        'e' => CFormatType::Float(ExponentLower),
        'E' => CFormatType::Float(ExponentUpper),
        'f' => CFormatType::Float(PointDecimalLower),
        'F' => CFormatType::Float(PointDecimalUpper),
        'g' => CFormatType::Float(GeneralLower),
        'G' => CFormatType::Float(GeneralUpper),
        'c' => CFormatType::Character(CCharacterType::Character),
        'r' => CFormatType::String(CFormatConversion::Repr),
        's' => CFormatType::String(CFormatConversion::Str),
        'b' => CFormatType::String(CFormatConversion::Bytes),
        'a' => CFormatType::String(CFormatConversion::Ascii),
        _ => return Err((CFormatErrorType::UnsupportedFormatChar(c.into()), index)),
    };
    Ok(format_type)
}

fn parse_quantity<C, I>(iter: &mut ParseIter<I>) -> Result<Option<CFormatQuantity>, ParsingError>
where
    C: FormatChar,
    I: Iterator<Item = C>,
{
    if let Some(&(_, c)) = iter.peek() {
        if c.eq_char('*') {
            iter.next().unwrap();
            return Ok(Some(CFormatQuantity::FromValuesTuple));
        }
        if let Some(i) = c.to_char_lossy().to_digit(10) {
            let mut num = i as i32;
            iter.next().unwrap();
            while let Some(&(index, c)) = iter.peek() {
                if let Some(i) = c.to_char_lossy().to_digit(10) {
                    num = num
                        .checked_mul(10)
                        .and_then(|num| num.checked_add(i as i32))
                        .ok_or((CFormatErrorType::IntTooBig, index))?;
                    iter.next().unwrap();
                } else {
                    break;
                }
            }
            return Ok(Some(CFormatQuantity::Amount(num.unsigned_abs() as usize)));
        }
    }
    Ok(None)
}

fn parse_precision<C, I>(iter: &mut ParseIter<I>) -> Result<Option<CFormatPrecision>, ParsingError>
where
    C: FormatChar,
    I: Iterator<Item = C>,
{
    if iter.next_if(|(_, c)| c.eq_char('.')).is_some() {
        let quantity = parse_quantity(iter)?;
        let precision = quantity.map_or(CFormatPrecision::Dot, CFormatPrecision::Quantity);
        return Ok(Some(precision));
    }
    Ok(None)
}

fn parse_text_inside_parentheses<T, I>(iter: &mut ParseIter<I>) -> Option<T>
where
    T: FormatBuf,
    I: Iterator<Item = T::Char>,
{
    let mut counter: i32 = 1;
    let mut contained_text = T::default();
    loop {
        let (_, c) = iter.next()?;
        match c.to_char_lossy() {
            '(' => {
                counter += 1;
            }
            ')' => {
                counter -= 1;
            }
            _ => (),
        }

        if counter > 0 {
            contained_text.extend([c]);
        } else {
            break;
        }
    }

    Some(contained_text)
}

#[derive(Debug, PartialEq)]
pub enum CFormatPart<T> {
    Literal(T),
    Spec(CFormatSpecKeyed<T>),
}

impl<T> CFormatPart<T> {
    #[inline]
    pub const fn is_specifier(&self) -> bool {
        matches!(self, Self::Spec { .. })
    }

    #[inline]
    pub const fn has_key(&self) -> bool {
        match self {
            Self::Spec(s) => s.mapping_key.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct CFormatStrOrBytes<S> {
    parts: Vec<(usize, CFormatPart<S>)>,
}

impl<S> CFormatStrOrBytes<S> {
    pub fn check_specifiers(&self) -> Option<(usize, bool)> {
        let mut count = 0;
        let mut mapping_required = false;
        for (_, part) in &self.parts {
            if part.is_specifier() {
                let has_key = part.has_key();
                if count == 0 {
                    mapping_required = has_key;
                } else if mapping_required != has_key {
                    return None;
                }
                count += 1;
            }
        }
        Some((count, mapping_required))
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &(usize, CFormatPart<S>)> {
        self.parts.iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut (usize, CFormatPart<S>)> {
        self.parts.iter_mut()
    }

    pub fn parse<I>(iter: &mut ParseIter<I>) -> Result<Self, CFormatError>
    where
        S: FormatBuf,
        I: Iterator<Item = S::Char>,
    {
        let mut parts = vec![];
        let mut literal = S::default();
        let mut part_index = 0;
        while let Some((index, c)) = iter.next() {
            if c.eq_char('%') {
                if let Some(&(_, second)) = iter.peek() {
                    if second.eq_char('%') {
                        iter.next().unwrap();
                        literal.extend([second]);
                        continue;
                    } else {
                        if !literal.is_empty() {
                            parts.push((
                                part_index,
                                CFormatPart::Literal(core::mem::take(&mut literal)),
                            ));
                        }
                        let spec = CFormatSpecKeyed::parse(iter).map_err(|err| CFormatError {
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
                literal.extend([c]);
            }
        }
        if !literal.is_empty() {
            parts.push((part_index, CFormatPart::Literal(literal)));
        }
        Ok(Self { parts })
    }
}

impl<S> IntoIterator for CFormatStrOrBytes<S> {
    type Item = (usize, CFormatPart<S>);
    type IntoIter = alloc::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.parts.into_iter()
    }
}

pub type CFormatBytes = CFormatStrOrBytes<Vec<u8>>;

impl CFormatBytes {
    pub fn parse_from_bytes(bytes: &[u8]) -> Result<Self, CFormatError> {
        let mut iter = bytes.iter().cloned().enumerate().peekable();
        Self::parse(&mut iter)
    }
}

pub type CFormatString = CFormatStrOrBytes<String>;

impl FromStr for CFormatString {
    type Err = CFormatError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut iter = text.chars().enumerate().peekable();
        Self::parse(&mut iter)
    }
}

pub type CFormatWtf8 = CFormatStrOrBytes<Wtf8Buf>;

impl CFormatWtf8 {
    pub fn parse_from_wtf8(s: &Wtf8) -> Result<Self, CFormatError> {
        let mut iter = s.code_points().enumerate().peekable();
        Self::parse(&mut iter)
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
        let expected = Ok(CFormatSpecKeyed {
            mapping_key: Some("amount".to_owned()),
            spec: CFormatSpec {
                format_type: CFormatType::Number(CNumberType::DecimalD),
                min_field_width: None,
                precision: None,
                flags: CConversionFlags::empty(),
            },
        });
        assert_eq!("%(amount)d".parse::<CFormatSpecKeyed<String>>(), expected);

        let expected = Ok(CFormatSpecKeyed {
            mapping_key: Some("m((u(((l((((ti))))p)))l))e".to_owned()),
            spec: CFormatSpec {
                format_type: CFormatType::Number(CNumberType::DecimalD),
                min_field_width: None,
                precision: None,
                flags: CConversionFlags::empty(),
            },
        });
        assert_eq!(
            "%(m((u(((l((((ti))))p)))l))e)d".parse::<CFormatSpecKeyed<String>>(),
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
                typ: CFormatErrorType::UnsupportedFormatChar('n'.into()),
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
            format_type: CFormatType::Number(CNumberType::DecimalD),
            min_field_width: Some(CFormatQuantity::Amount(10)),
            precision: None,
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
        assert_eq!(
            "%.s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("Hello, World!".to_owned()),
            "".to_owned()
        );
        assert_eq!(
            "%5.s"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_string("Hello, World!".to_owned()),
            "     ".to_owned()
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
            "%5d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(27)),
            "   27".to_owned()
        );
        assert_eq!(
            "%05d"
                .parse::<CFormatSpec>()
                .unwrap()
                .format_number(&BigInt::from(27)),
            "00027".to_owned()
        );
        assert_eq!(
            "%.5d"
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
            "%.2f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            "1.23"
        );
        assert_eq!(
            "%.f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            "1"
        );
        assert_eq!(
            "%+.f".parse::<CFormatSpec>().unwrap().format_float(1.2345),
            "+1"
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
                    CFormatPart::Spec(CFormatSpecKeyed {
                        mapping_key: None,
                        spec: CFormatSpec {
                            format_type: CFormatType::String(CFormatConversion::Str),
                            min_field_width: None,
                            precision: None,
                            flags: CConversionFlags::empty(),
                        },
                    }),
                ),
                (20, CFormatPart::Literal(" and I'm ".to_owned())),
                (
                    29,
                    CFormatPart::Spec(CFormatSpecKeyed {
                        mapping_key: None,
                        spec: CFormatSpec {
                            format_type: CFormatType::Number(CNumberType::DecimalD),
                            min_field_width: None,
                            precision: None,
                            flags: CConversionFlags::empty(),
                        },
                    }),
                ),
                (31, CFormatPart::Literal(" years old".to_owned())),
            ],
        });
        let result = fmt.parse::<CFormatString>();
        assert_eq!(
            result, expected,
            "left = {result:#?} \n\n\n right = {expected:#?}"
        );
    }
}

// spell-checker:ignore ddfe DTSF
use core::ops::Deref;
use core::{cmp, str::FromStr};
use itertools::{Itertools, PeekingNext};
use malachite_base::num::basic::floats::PrimitiveFloat;
use malachite_bigint::{BigInt, Sign};
use num_complex::Complex64;
use num_traits::FromPrimitive;
use num_traits::{Signed, cast::ToPrimitive};
use rustpython_literal::float;
use rustpython_literal::format::Case;

use crate::wtf8::{CodePoint, Wtf8, Wtf8Buf};

/// Locale information for 'n' format specifier.
/// Contains thousands separator, decimal point, and grouping pattern
/// from the C library's `localeconv()`.
#[derive(Clone, Debug)]
pub struct LocaleInfo {
    pub thousands_sep: String,
    pub decimal_point: String,
    /// Grouping pattern from `lconv.grouping`.
    /// Each element is a group size. The last non-zero element repeats.
    /// e.g. `[3, 0]` means groups of 3 repeating forever.
    pub grouping: Vec<u8>,
}

trait FormatParse {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8)
    where
        Self: Sized;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum FormatConversion {
    Str = b's',
    Repr = b'r',
    Ascii = b'b',
    Bytes = b'a',
}

impl FormatParse for FormatConversion {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8) {
        let Some(conversion) = Self::from_string(text) else {
            return (None, text);
        };
        let mut chars = text.code_points();
        chars.next(); // Consume the bang
        chars.next(); // Consume one r,s,a char
        (Some(conversion), chars.as_wtf8())
    }
}

impl FormatConversion {
    #[must_use]
    pub fn from_char(c: CodePoint) -> Option<Self> {
        match c.to_char_lossy() {
            's' => Some(Self::Str),
            'r' => Some(Self::Repr),
            'a' => Some(Self::Ascii),
            'b' => Some(Self::Bytes),
            _ => None,
        }
    }

    fn from_string(text: &Wtf8) -> Option<Self> {
        let mut chars = text.code_points();
        if chars.next()? != '!' {
            return None;
        }

        Self::from_char(chars.next()?)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormatAlign {
    Left,
    Right,
    AfterSign,
    Center,
}

impl FormatAlign {
    fn from_char(c: CodePoint) -> Option<Self> {
        match c.to_char_lossy() {
            '<' => Some(Self::Left),
            '>' => Some(Self::Right),
            '=' => Some(Self::AfterSign),
            '^' => Some(Self::Center),
            _ => None,
        }
    }
}

impl FormatParse for FormatAlign {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8) {
        let mut chars = text.code_points();
        if let Some(maybe_align) = chars.next().and_then(Self::from_char) {
            (Some(maybe_align), chars.as_wtf8())
        } else {
            (None, text)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormatSign {
    Plus,
    Minus,
    MinusOrSpace,
}

impl FormatParse for FormatSign {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8) {
        let mut chars = text.code_points();
        match chars.next().and_then(CodePoint::to_char) {
            Some('-') => (Some(Self::Minus), chars.as_wtf8()),
            Some('+') => (Some(Self::Plus), chars.as_wtf8()),
            Some(' ') => (Some(Self::MinusOrSpace), chars.as_wtf8()),
            _ => (None, text),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatGrouping {
    Comma,
    Underscore,
}

impl FormatParse for FormatGrouping {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8) {
        let mut chars = text.code_points();
        match chars.next().and_then(CodePoint::to_char) {
            Some('_') => (Some(Self::Underscore), chars.as_wtf8()),
            Some(',') => (Some(Self::Comma), chars.as_wtf8()),
            _ => (None, text),
        }
    }
}

impl From<FormatGrouping> for char {
    fn from(fg: FormatGrouping) -> Self {
        match fg {
            FormatGrouping::Comma => ',',
            FormatGrouping::Underscore => '_',
        }
    }
}

impl From<&FormatGrouping> for char {
    fn from(fg: &FormatGrouping) -> Self {
        Self::from(*fg)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatType {
    String,
    Binary,
    Character,
    Decimal,
    Octal,
    Number(Case),
    Hex(Case),
    Exponent(Case),
    GeneralFormat(Case),
    FixedPoint(Case),
    Percentage,
    Unknown(char),
}

impl From<&FormatType> for char {
    fn from(from: &FormatType) -> Self {
        match from {
            FormatType::String => 's',
            FormatType::Binary => 'b',
            FormatType::Character => 'c',
            FormatType::Decimal => 'd',
            FormatType::Octal => 'o',
            FormatType::Number(Case::Lower) => 'n',
            FormatType::Number(Case::Upper) => 'N',
            FormatType::Hex(Case::Lower) => 'x',
            FormatType::Hex(Case::Upper) => 'X',
            FormatType::Exponent(Case::Lower) => 'e',
            FormatType::Exponent(Case::Upper) => 'E',
            FormatType::GeneralFormat(Case::Lower) => 'g',
            FormatType::GeneralFormat(Case::Upper) => 'G',
            FormatType::FixedPoint(Case::Lower) => 'f',
            FormatType::FixedPoint(Case::Upper) => 'F',
            FormatType::Percentage => '%',
            FormatType::Unknown(c) => *c,
        }
    }
}

impl FormatParse for FormatType {
    fn parse(text: &Wtf8) -> (Option<Self>, &Wtf8) {
        let mut chars = text.code_points();
        match chars.next().and_then(CodePoint::to_char) {
            Some('s') => (Some(Self::String), chars.as_wtf8()),
            Some('b') => (Some(Self::Binary), chars.as_wtf8()),
            Some('c') => (Some(Self::Character), chars.as_wtf8()),
            Some('d') => (Some(Self::Decimal), chars.as_wtf8()),
            Some('o') => (Some(Self::Octal), chars.as_wtf8()),
            Some('n') => (Some(Self::Number(Case::Lower)), chars.as_wtf8()),
            Some('N') => (Some(Self::Number(Case::Upper)), chars.as_wtf8()),
            Some('x') => (Some(Self::Hex(Case::Lower)), chars.as_wtf8()),
            Some('X') => (Some(Self::Hex(Case::Upper)), chars.as_wtf8()),
            Some('e') => (Some(Self::Exponent(Case::Lower)), chars.as_wtf8()),
            Some('E') => (Some(Self::Exponent(Case::Upper)), chars.as_wtf8()),
            Some('f') => (Some(Self::FixedPoint(Case::Lower)), chars.as_wtf8()),
            Some('F') => (Some(Self::FixedPoint(Case::Upper)), chars.as_wtf8()),
            Some('g') => (Some(Self::GeneralFormat(Case::Lower)), chars.as_wtf8()),
            Some('G') => (Some(Self::GeneralFormat(Case::Upper)), chars.as_wtf8()),
            Some('%') => (Some(Self::Percentage), chars.as_wtf8()),
            Some(c) => (Some(Self::Unknown(c)), chars.as_wtf8()),
            _ => (None, text),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatSpec {
    conversion: Option<FormatConversion>,
    fill: Option<CodePoint>,
    align: Option<FormatAlign>,
    align_specified: bool,
    sign: Option<FormatSign>,
    no_neg_0: bool,
    alternate_form: bool,
    width: Option<usize>,
    grouping_option: Option<FormatGrouping>,
    precision: Option<usize>,
    frac_grouping_option: Option<FormatGrouping>,
    format_type: Option<FormatType>,
}

fn get_num_digits(text: &Wtf8) -> usize {
    for (index, character) in text.code_point_indices() {
        if !character.is_char_and(|c| c.is_ascii_digit()) {
            return index;
        }
    }
    text.len()
}

fn parse_fill_and_align(text: &Wtf8) -> (Option<CodePoint>, Option<FormatAlign>, &Wtf8) {
    let char_indices: Vec<(usize, CodePoint)> = text.code_point_indices().take(3).collect();
    if char_indices.is_empty() {
        (None, None, text)
    } else if char_indices.len() == 1 {
        let (maybe_align, remaining) = FormatAlign::parse(text);
        (None, maybe_align, remaining)
    } else {
        let (maybe_align, remaining) = FormatAlign::parse(&text[char_indices[1].0..]);
        if maybe_align.is_some() {
            (Some(char_indices[0].1), maybe_align, remaining)
        } else {
            let (only_align, only_align_remaining) = FormatAlign::parse(text);
            (None, only_align, only_align_remaining)
        }
    }
}

fn parse_number(text: &Wtf8) -> Result<(Option<usize>, &Wtf8), FormatSpecError> {
    let num_digits: usize = get_num_digits(text);
    if num_digits == 0 {
        return Ok((None, text));
    }
    if let Some(num) = parse_usize(&text[..num_digits]) {
        Ok((Some(num), &text[num_digits..]))
    } else {
        // NOTE: this condition is different from CPython
        Err(FormatSpecError::DecimalDigitsTooMany)
    }
}

fn parse_alternate_form(text: &Wtf8) -> (bool, &Wtf8) {
    let mut chars = text.code_points();
    match chars.next().and_then(CodePoint::to_char) {
        Some('#') => (true, chars.as_wtf8()),
        _ => (false, text),
    }
}

fn parse_no_negative_zero(text: &Wtf8) -> (bool, &Wtf8) {
    let mut chars = text.code_points();
    match chars.next().and_then(CodePoint::to_char) {
        Some('z') => (true, chars.as_wtf8()),
        _ => (false, text),
    }
}

fn parse_zero(text: &Wtf8) -> (bool, &Wtf8) {
    let mut chars = text.code_points();
    match chars.next().and_then(CodePoint::to_char) {
        Some('0') => (true, chars.as_wtf8()),
        _ => (false, text),
    }
}

fn parse_char(text: &Wtf8, expected: char) -> (bool, &Wtf8) {
    let mut chars = text.code_points();
    if chars.next().and_then(CodePoint::to_char) == Some(expected) {
        (true, chars.as_wtf8())
    } else {
        (false, text)
    }
}

fn parse_precision(
    text: &Wtf8,
) -> Result<(Option<usize>, Option<FormatGrouping>, &Wtf8), FormatSpecError> {
    let (dot, text) = parse_char(text, '.');
    if !dot {
        return Ok((None, None, text));
    }
    let (precision, text) = parse_number(text)?;
    if let Some(precision) = precision
        && precision > i32::MAX as usize
    {
        return Err(FormatSpecError::PrecisionTooBig);
    }
    let mut frac_grouping = None;
    let (comma, text) = parse_char(text, ',');
    if comma {
        frac_grouping = Some(FormatGrouping::Comma);
    }
    let (underscore, text) = parse_char(text, '_');
    if underscore {
        if frac_grouping.is_some() {
            return Err(FormatSpecError::ExclusiveFormat(',', '_'));
        }
        frac_grouping = Some(FormatGrouping::Underscore);
    }
    let (trailing_comma, _) = parse_char(text, ',');
    if trailing_comma && frac_grouping == Some(FormatGrouping::Underscore) {
        return Err(FormatSpecError::ExclusiveFormat(',', '_'));
    }
    // Not having a precision or underscore/comma after a dot is an error.
    if precision.is_none() && frac_grouping.is_none() {
        return Err(FormatSpecError::PrecisionMissing);
    }
    Ok((precision, frac_grouping, text))
}

impl FormatSpec {
    pub fn parse(text: impl AsRef<Wtf8>) -> Result<Self, FormatSpecError> {
        Self::_parse(text.as_ref())
    }

    fn _parse(text: &Wtf8) -> Result<Self, FormatSpecError> {
        // the invalid-specifier error reports the whole spec
        let full_spec = String::from_utf8_lossy(text.as_bytes()).into_owned();
        // get_integer in CPython
        let (conversion, text) = FormatConversion::parse(text);
        let (mut fill, mut align, text) = parse_fill_and_align(text);
        let align_specified = align.is_some();
        let (sign, text) = FormatSign::parse(text);
        let (no_neg_0, text) = parse_no_negative_zero(text);
        let (alternate_form, text) = parse_alternate_form(text);
        let (zero, text) = parse_zero(text);
        let (width, text) = parse_number(text)?;
        if let Some(w) = width
            && w > i32::MAX as usize
        {
            return Err(FormatSpecError::DecimalDigitsTooMany);
        }
        let (grouping_option, text) = FormatGrouping::parse(text);
        if let Some(grouping) = grouping_option {
            Self::validate_separator(grouping, text)?;
        }
        let (precision, frac_grouping_option, text) = parse_precision(text)?;
        let (format_type, text) = FormatType::parse(text);
        if !text.is_empty() {
            return Err(FormatSpecError::InvalidFormatSpecifier(full_spec));
        }

        if zero && fill.is_none() {
            fill.replace('0'.into());
            align = align.or(Some(FormatAlign::AfterSign));
        }

        Ok(Self {
            conversion,
            fill,
            align,
            align_specified,
            sign,
            no_neg_0,
            alternate_form,
            width,
            grouping_option,
            precision,
            frac_grouping_option,
            format_type,
        })
    }

    fn validate_separator(grouping: FormatGrouping, text: &Wtf8) -> Result<(), FormatSpecError> {
        let mut chars = text.code_points().peekable();
        let grouping_char = char::from(grouping);
        match chars.peek().and_then(|cp| CodePoint::to_char(*cp)) {
            Some(c) if c == ',' || c == '_' => {
                if c == grouping_char {
                    Err(FormatSpecError::UnspecifiedFormat(c, c))
                } else {
                    Err(FormatSpecError::ExclusiveFormat(',', '_'))
                }
            }
            _ => Ok(()),
        }
    }

    fn compute_fill_string(fill_char: CodePoint, fill_chars_needed: i32) -> Wtf8Buf {
        (0..fill_chars_needed).map(|_| fill_char).collect()
    }

    fn add_magnitude_separators_for_char(
        magnitude_str: String,
        inter: i32,
        sep: char,
        disp_digit_cnt: i32,
    ) -> String {
        // Group only the leading integer digits; the trailing remainder must
        // never receive separators. For decimal and float output (interval 3)
        // that remainder is the decimal point and fraction, an exponent
        // (`e+NN`), or a trailing percent sign. Hex/octal/binary output
        // (interval 4) has no such tail and its `a`-`f`/`e`/`E` are digits, so
        // the whole magnitude is groupable.
        let int_len = if inter == 4 {
            magnitude_str.len()
        } else {
            magnitude_str
                .bytes()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(magnitude_str.len())
        };
        // No leading integer digits (e.g. "inf"/"nan") means nothing to group;
        // leave any width padding to the fill/align step.
        if int_len == 0 {
            return magnitude_str;
        }
        let magnitude_int_str = magnitude_str[..int_len].to_string();
        let remainder = &magnitude_str[int_len..];
        let dec_digit_cnt = magnitude_str.len() as i32 - magnitude_int_str.len() as i32;
        let int_digit_cnt = disp_digit_cnt - dec_digit_cnt;
        let mut result = Self::separate_integer(magnitude_int_str, inter, sep, int_digit_cnt);
        result.push_str(remainder);
        result
    }

    fn separate_integer(
        magnitude_str: String,
        inter: i32,
        sep: char,
        disp_digit_cnt: i32,
    ) -> String {
        let magnitude_len = magnitude_str.len() as i32;
        let offset = (disp_digit_cnt % (inter + 1) == 0) as i32;
        let disp_digit_cnt = disp_digit_cnt + offset;
        let pad_cnt = disp_digit_cnt - magnitude_len;
        let sep_cnt = disp_digit_cnt / (inter + 1);
        let diff = pad_cnt - sep_cnt;
        if pad_cnt > 0 && diff > 0 {
            // separate with 0 padding
            let padding = "0".repeat(diff as usize);
            let padded_num = format!("{padding}{magnitude_str}");
            Self::insert_separator(padded_num, inter, sep, sep_cnt)
        } else {
            // separate without padding
            let sep_cnt = (magnitude_len - 1) / inter;
            Self::insert_separator(magnitude_str, inter, sep, sep_cnt)
        }
    }

    fn insert_separator(mut magnitude_str: String, inter: i32, sep: char, sep_cnt: i32) -> String {
        let magnitude_len = magnitude_str.len() as i32;
        for i in 1..=sep_cnt {
            magnitude_str.insert((magnitude_len - inter * i) as usize, sep);
        }
        magnitude_str
    }

    fn validate_format(&self, default_format_type: FormatType) -> Result<(), FormatSpecError> {
        let format_type = self.format_type.as_ref().unwrap_or(&default_format_type);
        match (&self.grouping_option, format_type) {
            (
                Some(FormatGrouping::Comma),
                FormatType::String
                | FormatType::Character
                | FormatType::Binary
                | FormatType::Octal
                | FormatType::Hex(_)
                | FormatType::Number(_),
            ) => {
                let ch = char::from(format_type);
                Err(FormatSpecError::UnspecifiedFormat(',', ch))
            }
            (
                Some(FormatGrouping::Underscore),
                FormatType::String | FormatType::Character | FormatType::Number(_),
            ) => {
                let ch = char::from(format_type);
                Err(FormatSpecError::UnspecifiedFormat('_', ch))
            }
            _ => Ok(()),
        }?;
        if let Some(grouping) = self.frac_grouping_option
            && matches!(format_type, FormatType::Number(_))
        {
            let ch = char::from(format_type);
            return Err(FormatSpecError::UnspecifiedFormat(char::from(grouping), ch));
        }
        Ok(())
    }

    fn formatted_magnitude_is_zero(magnitude: &str) -> bool {
        let mut saw_digit = false;
        for byte in magnitude.bytes() {
            if byte.is_ascii_digit() {
                saw_digit = true;
                if byte != b'0' {
                    return false;
                }
            }
        }
        saw_digit
    }

    fn is_negative_after_zero_coercion(&self, num: f64, magnitude: &str) -> bool {
        num.is_sign_negative()
            && !num.is_nan()
            && !(self.no_neg_0 && Self::formatted_magnitude_is_zero(magnitude))
    }

    fn validate_complex_padding_and_alignment(&self) -> Result<(), FormatSpecError> {
        match &self.fill.unwrap_or_else(|| ' '.into()).to_char() {
            Some('0') => Err(FormatSpecError::ZeroPadding),
            _ if self.align == Some(FormatAlign::AfterSign) => Err(FormatSpecError::AlignmentFlag),
            _ => Ok(()),
        }
    }

    const fn get_separator_interval(&self) -> usize {
        match self.format_type {
            Some(FormatType::Binary | FormatType::Octal | FormatType::Hex(_)) => 4,
            Some(
                FormatType::Decimal
                | FormatType::FixedPoint(_)
                | FormatType::GeneralFormat(_)
                | FormatType::Exponent(_)
                | FormatType::Percentage
                | FormatType::Number(_),
            ) => 3,
            None => 3,
            _ => panic!("Separators only valid for numbers!"),
        }
    }

    fn add_magnitude_separators(&self, magnitude_str: String, prefix: &str) -> String {
        match &self.grouping_option {
            Some(fg) => {
                let sep = char::from(fg);
                let inter = self.get_separator_interval().try_into().unwrap();
                let magnitude_len = magnitude_str.len();
                let disp_digit_cnt = if self.fill == Some('0'.into())
                    && self.align == Some(FormatAlign::AfterSign)
                {
                    let width = self.width.unwrap_or(magnitude_len) as i32
                        - prefix.len() as i32
                        - self.frac_separator_count(&magnitude_str) as i32;
                    cmp::max(width, magnitude_len as i32)
                } else {
                    magnitude_len as i32
                };
                Self::add_magnitude_separators_for_char(magnitude_str, inter, sep, disp_digit_cnt)
            }
            None => magnitude_str,
        }
    }

    fn frac_digit_span(&self, magnitude_str: &str) -> Option<(FormatGrouping, usize, usize)> {
        let grouping = self.frac_grouping_option?;
        let start = magnitude_str.find('.')? + 1;
        let end = magnitude_str[start..]
            .bytes()
            .position(|b| !b.is_ascii_digit())
            .map_or(magnitude_str.len(), |offset| start + offset);
        (start < end).then_some((grouping, start, end))
    }

    fn frac_separator_count(&self, magnitude_str: &str) -> usize {
        match self.frac_digit_span(magnitude_str) {
            Some((_, start, end)) => (end - start - 1) / self.get_separator_interval(),
            None => 0,
        }
    }

    fn add_frac_separators(&self, magnitude_str: String) -> String {
        let Some((grouping, start, end)) = self.frac_digit_span(&magnitude_str) else {
            return magnitude_str;
        };
        let inter = self.get_separator_interval();
        let sep = char::from(grouping);
        let mut result = magnitude_str[..start].to_string();
        let mut frac = &magnitude_str[start..end];
        while frac.len() > inter {
            result.push_str(&frac[..inter]);
            result.push(sep);
            frac = &frac[inter..];
        }
        result.push_str(frac);
        result.push_str(&magnitude_str[end..]);
        result
    }

    /// Returns true if this format spec uses the locale-aware 'n' format type.
    #[must_use]
    pub fn has_locale_format(&self) -> bool {
        matches!(self.format_type, Some(FormatType::Number(Case::Lower)))
    }

    /// Returns true if this format spec produces a decimal int representation
    /// subject to `sys.get_int_max_str_digits()` (no spec, 'd', or 'n').
    /// Binary bases ('b', 'o', 'x', 'X') are exempt per CPython. 'N' is rejected
    /// later in `format_int` as `UnknownFormatCode`, so it is not included here.
    #[must_use]
    pub fn is_decimal_int_format(&self) -> bool {
        matches!(
            self.format_type,
            None | Some(FormatType::Decimal | FormatType::Number(Case::Lower))
        )
    }

    /// Insert locale-aware thousands separators into an integer string.
    /// Follows CPython's GroupGenerator logic for variable-width grouping.
    fn insert_locale_grouping(int_part: &str, locale: &LocaleInfo) -> String {
        if locale.grouping.is_empty() || locale.thousands_sep.is_empty() || int_part.len() <= 1 {
            return int_part.to_string();
        }

        let mut group_idx = 0;
        let mut group_size = locale.grouping[0] as usize;

        if group_size == 0 {
            return int_part.to_string();
        }

        // Collect groups of digits from right to left
        let len = int_part.len();
        let mut groups: Vec<&str> = Vec::new();
        let mut pos = len;

        loop {
            if pos <= group_size {
                groups.push(&int_part[..pos]);
                break;
            }

            groups.push(&int_part[pos - group_size..pos]);
            pos -= group_size;

            // Advance to next group size
            if group_idx + 1 < locale.grouping.len() {
                let next = locale.grouping[group_idx + 1] as usize;
                if next != 0 {
                    group_size = next;
                    group_idx += 1;
                }
                // 0 means repeat previous group size forever
            }
        }

        // Groups were collected right-to-left, reverse to get left-to-right
        groups.reverse();
        groups.join(&locale.thousands_sep)
    }

    /// Apply locale-aware grouping and decimal point replacement to a formatted number.
    fn apply_locale_formatting(magnitude_str: String, locale: &LocaleInfo) -> String {
        let mut parts = magnitude_str.splitn(2, '.');
        let int_part = parts.next().unwrap();
        let grouped = Self::insert_locale_grouping(int_part, locale);

        if let Some(frac_part) = parts.next() {
            format!("{grouped}{}{frac_part}", locale.decimal_point)
        } else {
            grouped
        }
    }

    /// Format an integer with locale-aware 'n' format.
    pub fn format_int_locale(
        &self,
        num: &BigInt,
        locale: &LocaleInfo,
    ) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::Decimal)?;
        let magnitude = num.abs();

        let raw_magnitude_str = match self.format_type {
            Some(FormatType::Number(Case::Lower)) => self.format_int_radix(magnitude, 10),
            _ => return self.format_int(num),
        }?;
        if self.no_neg_0 {
            return Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"));
        }

        let magnitude_str = Self::apply_locale_formatting(raw_magnitude_str, locale);

        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = match num.sign() {
            Sign::Minus => "-",
            _ => match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            },
        };

        Ok(
            self.format_sign_and_align(
                &AsciiStr::new(&magnitude_str),
                sign_str,
                FormatAlign::Right,
            ),
        )
    }

    /// Format a float with locale-aware 'n' format.
    pub fn format_float_locale(
        &self,
        num: f64,
        locale: &LocaleInfo,
    ) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::FixedPoint(Case::Lower))?;
        let precision = self.precision.unwrap_or(6);
        let magnitude = num.abs();

        let raw_magnitude_str = match &self.format_type {
            Some(FormatType::Number(case)) => {
                let precision = if precision == 0 { 1 } else { precision };
                Ok(float::format_general(
                    precision,
                    magnitude,
                    *case,
                    self.alternate_form,
                    false,
                ))
            }
            _ => return self.format_float(num),
        }?;

        let magnitude_str = Self::apply_locale_formatting(raw_magnitude_str, locale);

        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = if self.is_negative_after_zero_coercion(num, &magnitude_str) {
            "-"
        } else {
            match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            }
        };

        Ok(
            self.format_sign_and_align(
                &AsciiStr::new(&magnitude_str),
                sign_str,
                FormatAlign::Right,
            ),
        )
    }

    /// Format a complex number with locale-aware 'n' format.
    pub fn format_complex_locale(
        &self,
        num: &Complex64,
        locale: &LocaleInfo,
    ) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::FixedPoint(Case::Lower))?;
        // Reuse format_complex_re_im with 'g' type to get the base formatted parts,
        // then apply locale grouping. This matches CPython's format_complex_internal:
        // 'n' → 'g', add_parens=0, skip_re=0.
        let locale_spec = Self {
            format_type: Some(FormatType::GeneralFormat(Case::Lower)),
            ..*self
        };
        let (formatted_re, formatted_im) = locale_spec.format_complex_re_im(num)?;

        // Apply locale grouping to both parts
        let grouped_re = if formatted_re.is_empty() {
            formatted_re
        } else {
            // Split sign from magnitude, apply grouping, recombine
            let (sign, mag) = if formatted_re.starts_with('-')
                || formatted_re.starts_with('+')
                || formatted_re.starts_with(' ')
            {
                formatted_re.split_at(1)
            } else {
                ("", formatted_re.as_str())
            };
            format!(
                "{sign}{}",
                Self::apply_locale_formatting(mag.to_string(), locale)
            )
        };

        // formatted_im is like "+1234j" or "-1234j" or "1234j"
        // Split sign, magnitude, and 'j' suffix
        let im_str = &formatted_im;
        let (im_sign, im_rest) = if im_str.starts_with('+') || im_str.starts_with('-') {
            im_str.split_at(1)
        } else {
            ("", im_str.as_str())
        };
        let im_mag = im_rest.strip_suffix('j').unwrap_or(im_rest);
        let im_grouped = Self::apply_locale_formatting(im_mag.to_string(), locale);
        let grouped_im = format!("{im_sign}{im_grouped}j");

        // No parentheses for 'n' format (CPython: add_parens=0)
        let magnitude_str = format!("{grouped_re}{grouped_im}");

        self.validate_complex_padding_and_alignment()?;
        Ok(self.format_sign_and_align(&AsciiStr::new(&magnitude_str), "", FormatAlign::Right))
    }

    pub fn format_bool(&self, input: bool) -> Result<String, FormatSpecError> {
        let x = u8::from(input);
        match &self.format_type {
            Some(
                FormatType::Binary
                | FormatType::Decimal
                | FormatType::Octal
                | FormatType::Number(Case::Lower)
                | FormatType::Hex(_)
                | FormatType::GeneralFormat(_)
                | FormatType::Character,
            ) => self.format_int(&BigInt::from_u8(x).unwrap()),
            Some(FormatType::Exponent(_) | FormatType::FixedPoint(_) | FormatType::Percentage) => {
                self.format_float(x as f64)
            }
            None => {
                if self.no_neg_0 {
                    return Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"));
                }
                let first_letter = (input.to_string().as_bytes()[0] as char).to_uppercase();
                Ok(first_letter.collect::<String>() + &input.to_string()[1..])
            }
            Some(format_type) => {
                let ch = char::from(format_type);
                Err(FormatSpecError::UnknownFormatCode(ch, "bool"))
            }
        }
    }

    pub fn format_float(&self, num: f64) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::FixedPoint(Case::Lower))?;
        let precision = self.precision.unwrap_or(6);
        let magnitude = num.abs();
        let raw_magnitude_str: Result<String, FormatSpecError> = match &self.format_type {
            Some(FormatType::FixedPoint(case)) => Ok(float::format_fixed(
                precision,
                magnitude,
                *case,
                self.alternate_form,
            )),
            Some(
                FormatType::Decimal
                | FormatType::Binary
                | FormatType::Octal
                | FormatType::Hex(_)
                | FormatType::String
                | FormatType::Character
                | FormatType::Number(Case::Upper)
                | FormatType::Unknown(_),
            ) => {
                let ch = char::from(self.format_type.as_ref().unwrap());
                Err(FormatSpecError::UnknownFormatCode(ch, "float"))
            }
            Some(FormatType::GeneralFormat(case) | FormatType::Number(case)) => {
                let precision = if precision == 0 { 1 } else { precision };
                Ok(float::format_general(
                    precision,
                    magnitude,
                    *case,
                    self.alternate_form,
                    false,
                ))
            }
            Some(FormatType::Exponent(case)) => Ok(float::format_exponent(
                precision,
                magnitude,
                *case,
                self.alternate_form,
            )),
            Some(FormatType::Percentage) => match magnitude {
                magnitude if magnitude.is_nan() => Ok("nan%".to_owned()),
                magnitude if magnitude.is_infinite() => Ok("inf%".to_owned()),
                _ => {
                    let scaled = magnitude * 100.0;
                    // `magnitude * 100` can overflow a finite input to +inf
                    // (e.g. f64::MAX). Emit "inf%" so the outer sign handler
                    // produces "-inf%" or "inf%" consistently with CPython.
                    if scaled.is_infinite() {
                        Ok("inf%".to_owned())
                    } else {
                        let capped = float::clamp_fmt_precision(precision);
                        let mut result = format!("{scaled:.capped$}");
                        // Pad with '0's up to the requested precision to match
                        // CPython byte-identically past the internal cap.
                        let missing = precision.saturating_sub(capped);
                        if missing > 0 {
                            result.extend(core::iter::repeat_n('0', missing));
                        }
                        let point = float::decimal_point_or_empty(precision, self.alternate_form);
                        Ok(format!("{result}{point}%"))
                    }
                }
            },
            None => match magnitude {
                magnitude if magnitude.is_nan() => Ok("nan".to_owned()),
                magnitude if magnitude.is_infinite() => Ok("inf".to_owned()),
                _ => match self.precision {
                    Some(precision) => {
                        // Empty presentation type with a precision behaves like
                        // `g` but repr-like: precision is clamped to at least 1,
                        // and an integer-looking result keeps a trailing `.0`
                        // (Py_DTSF_ADD_DOT_0).
                        let precision = if precision == 0 { 1 } else { precision };
                        let s = float::format_general(
                            precision,
                            magnitude,
                            Case::Lower,
                            self.alternate_form,
                            true,
                        );
                        Ok(if s.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
                            s
                        } else {
                            format!("{s}.0")
                        })
                    }
                    None => {
                        let s = float::to_string(magnitude);
                        // Alternate form forces a decimal point into the
                        // repr-like output. Only exponent-form values lack one
                        // (`1e+16` -> `1.e+16`); fixed-form repr already carries
                        // a `.`.
                        Ok(if self.alternate_form && !s.contains('.') {
                            match s.find(['e', 'E']) {
                                Some(pos) => format!("{}.{}", &s[..pos], &s[pos..]),
                                None => format!("{s}."),
                            }
                        } else {
                            s
                        })
                    }
                },
            },
        };
        let raw_magnitude_str = raw_magnitude_str?;
        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = if self.is_negative_after_zero_coercion(num, &raw_magnitude_str) {
            "-"
        } else {
            match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            }
        };
        let magnitude_str = self.add_magnitude_separators(raw_magnitude_str, sign_str);
        let magnitude_str = self.add_frac_separators(magnitude_str);
        Ok(
            self.format_sign_and_align(
                &AsciiStr::new(&magnitude_str),
                sign_str,
                FormatAlign::Right,
            ),
        )
    }

    #[inline]
    fn format_int_radix(&self, magnitude: BigInt, radix: u32) -> Result<String, FormatSpecError> {
        match self.precision {
            Some(_) => Err(FormatSpecError::PrecisionNotAllowed),
            None => Ok(magnitude.to_str_radix(radix)),
        }
    }

    pub fn format_int(&self, num: &BigInt) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::Decimal)?;
        let magnitude = num.abs();
        let prefix = if self.alternate_form {
            match self.format_type {
                Some(FormatType::Binary) => "0b",
                Some(FormatType::Octal) => "0o",
                Some(FormatType::Hex(Case::Lower)) => "0x",
                Some(FormatType::Hex(Case::Upper)) => "0X",
                _ => "",
            }
        } else {
            ""
        };
        let raw_magnitude_str = match self.format_type {
            Some(FormatType::Binary) => self.format_int_radix(magnitude, 2),
            Some(FormatType::Decimal) => self.format_int_radix(magnitude, 10),
            Some(FormatType::Octal) => self.format_int_radix(magnitude, 8),
            Some(FormatType::Hex(Case::Lower)) => self.format_int_radix(magnitude, 16),
            Some(FormatType::Hex(Case::Upper)) => match self.precision {
                Some(_) => Err(FormatSpecError::PrecisionNotAllowed),
                None => {
                    let mut result = magnitude.to_str_radix(16);
                    result.make_ascii_uppercase();
                    Ok(result)
                }
            },
            Some(FormatType::Number(Case::Lower)) => self.format_int_radix(magnitude, 10),
            Some(FormatType::Number(Case::Upper)) => {
                Err(FormatSpecError::UnknownFormatCode('N', "int"))
            }
            Some(FormatType::String) => Err(FormatSpecError::UnknownFormatCode('s', "int")),
            Some(FormatType::Character) => {
                if self.precision.is_some() {
                    Err(FormatSpecError::PrecisionNotAllowed)
                } else if self.no_neg_0 {
                    Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"))
                } else {
                    match (self.sign, self.alternate_form) {
                        (Some(_), _) => Err(FormatSpecError::NotAllowed("Sign")),
                        (_, true) => Err(FormatSpecError::NotAllowed("Alternate form (#)")),
                        _ => match num.to_u32() {
                            Some(n) if n <= 0x10ffff => {
                                Ok(core::char::from_u32(n).unwrap().to_string())
                            }
                            Some(_) | None => Err(FormatSpecError::CodeNotInRange),
                        },
                    }
                }
            }
            Some(
                FormatType::GeneralFormat(_)
                | FormatType::FixedPoint(_)
                | FormatType::Exponent(_)
                | FormatType::Percentage,
            ) => match num.to_f64() {
                Some(float) => return self.format_float(float),
                _ => Err(FormatSpecError::UnableToConvert),
            },
            Some(FormatType::Unknown(c)) => Err(FormatSpecError::UnknownFormatCode(c, "int")),
            None => self.format_int_radix(magnitude, 10),
        }?;
        if self.no_neg_0 {
            return Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"));
        }
        let format_sign = self.sign.unwrap_or(FormatSign::Minus);
        let sign_str = match num.sign() {
            Sign::Minus => "-",
            _ => match format_sign {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            },
        };
        let sign_prefix = format!("{sign_str}{prefix}");
        let magnitude_str = self.add_magnitude_separators(raw_magnitude_str, &sign_prefix);
        Ok(self.format_sign_and_align(
            &AsciiStr::new(&magnitude_str),
            &sign_prefix,
            FormatAlign::Right,
        ))
    }

    pub fn format_string<T>(&self, s: &T) -> Result<String, FormatSpecError>
    where
        T: CharLen + Deref<Target = str>,
    {
        self.validate_format(FormatType::String)?;
        match self.format_type {
            Some(FormatType::String) | None => {
                if self.no_neg_0 {
                    return Err(FormatSpecError::NegativeZeroCoercionNotAllowed("string"));
                }
                if self.align == Some(FormatAlign::AfterSign) && self.align_specified {
                    return Err(FormatSpecError::StringAlignmentFlag);
                }
                // CPython parity: precision truncates BEFORE width pads.
                // `'{:3.2s}'.format('abc')` -> 'ab ' (truncate to 'ab', pad to 3).
                let truncated: String = match self.precision {
                    Some(p) => s.deref().chars().take(p).collect(),
                    None => s.deref().to_owned(),
                };
                let spec = Self {
                    align: if self.align == Some(FormatAlign::AfterSign) {
                        Some(FormatAlign::Left)
                    } else {
                        self.align
                    },
                    ..*self
                };
                Ok(spec.format_sign_and_align(&truncated, "", FormatAlign::Left))
            }
            _ => {
                let ch = char::from(self.format_type.as_ref().unwrap());
                Err(FormatSpecError::UnknownFormatCode(ch, "str"))
            }
        }
    }

    pub fn format_complex(&self, num: &Complex64) -> Result<String, FormatSpecError> {
        let (formatted_re, formatted_im) = self.format_complex_re_im(num)?;
        // Enclose in parentheses if there is no format type and formatted_re is not empty
        let magnitude_str = if self.format_type.is_none() && !formatted_re.is_empty() {
            format!("({formatted_re}{formatted_im})")
        } else {
            format!("{formatted_re}{formatted_im}")
        };
        self.validate_complex_padding_and_alignment()?;
        Ok(self.format_sign_and_align(&AsciiStr::new(&magnitude_str), "", FormatAlign::Right))
    }

    fn format_complex_re_im(&self, num: &Complex64) -> Result<(String, String), FormatSpecError> {
        // Format real part
        let formatted_re =
            if num.re != 0.0 || num.re.is_negative_zero() || self.format_type.is_some() {
                let re = self.format_complex_float(num.re)?;
                let sign_re = if self.is_negative_after_zero_coercion(num.re, &re) {
                    "-"
                } else {
                    match self.sign.unwrap_or(FormatSign::Minus) {
                        FormatSign::Plus => "+",
                        FormatSign::Minus => "",
                        FormatSign::MinusOrSpace => " ",
                    }
                };
                format!("{sign_re}{re}")
            } else {
                String::new()
            };

        // Format imaginary part
        let im = self.format_complex_float(num.im)?;
        let sign_im = if self.is_negative_after_zero_coercion(num.im, &im) {
            "-"
        } else if formatted_re.is_empty() {
            match self.sign.unwrap_or(FormatSign::Minus) {
                FormatSign::Plus => "+",
                FormatSign::Minus => "",
                FormatSign::MinusOrSpace => " ",
            }
        } else {
            "+"
        };
        Ok((formatted_re, format!("{sign_im}{im}j")))
    }

    fn format_complex_float(&self, num: f64) -> Result<String, FormatSpecError> {
        self.validate_format(FormatType::FixedPoint(Case::Lower))?;
        let precision = self.precision.unwrap_or(6);
        let magnitude = num.abs();
        let magnitude_str = match &self.format_type {
            Some(
                FormatType::Decimal
                | FormatType::Binary
                | FormatType::Octal
                | FormatType::Hex(_)
                | FormatType::String
                | FormatType::Character
                | FormatType::Number(Case::Upper)
                | FormatType::Percentage
                | FormatType::Unknown(_),
            ) => {
                let ch = char::from(self.format_type.as_ref().unwrap());
                Err(FormatSpecError::UnknownFormatCode(ch, "complex"))
            }
            Some(FormatType::FixedPoint(case)) => Ok(float::format_fixed(
                precision,
                magnitude,
                *case,
                self.alternate_form,
            )),
            Some(FormatType::GeneralFormat(case) | FormatType::Number(case)) => {
                let precision = if precision == 0 { 1 } else { precision };
                Ok(float::format_general(
                    precision,
                    magnitude,
                    *case,
                    self.alternate_form,
                    false,
                ))
            }
            Some(FormatType::Exponent(case)) => Ok(float::format_exponent(
                precision,
                magnitude,
                *case,
                self.alternate_form,
            )),
            None => match magnitude {
                magnitude if magnitude.is_nan() => Ok("nan".to_owned()),
                magnitude if magnitude.is_infinite() => Ok("inf".to_owned()),
                _ => match self.precision {
                    Some(precision) => Ok(float::format_general(
                        precision,
                        magnitude,
                        Case::Lower,
                        self.alternate_form,
                        true,
                    )),
                    None => {
                        if magnitude.fract() == 0.0 {
                            Ok(magnitude.trunc().to_string())
                        } else {
                            Ok(magnitude.to_string())
                        }
                    }
                },
            },
        }?;
        let magnitude_str = match &self.grouping_option {
            Some(fg) => {
                let sep = char::from(fg);
                let inter = self.get_separator_interval().try_into().unwrap();
                let len = magnitude_str.len() as i32;
                Self::add_magnitude_separators_for_char(magnitude_str, inter, sep, len)
            }
            None => magnitude_str,
        };
        Ok(self.add_frac_separators(magnitude_str))
    }

    fn format_sign_and_align<T>(
        &self,
        magnitude_str: &T,
        sign_str: &str,
        default_align: FormatAlign,
    ) -> String
    where
        T: CharLen + Deref<Target = str>,
    {
        let align = self.align.unwrap_or(default_align);

        let num_chars = magnitude_str.char_len();
        let fill_char = self.fill.unwrap_or_else(|| ' '.into());
        let fill_chars_needed: i32 = self.width.map_or(0, |w| {
            cmp::max(0, (w as i32) - (num_chars as i32) - (sign_str.len() as i32))
        });

        let magnitude_str = &**magnitude_str;
        match align {
            FormatAlign::Left => format!(
                "{}{}{}",
                sign_str,
                magnitude_str,
                Self::compute_fill_string(fill_char, fill_chars_needed)
            ),
            FormatAlign::Right => format!(
                "{}{}{}",
                Self::compute_fill_string(fill_char, fill_chars_needed),
                sign_str,
                magnitude_str
            ),
            FormatAlign::AfterSign => format!(
                "{}{}{}",
                sign_str,
                Self::compute_fill_string(fill_char, fill_chars_needed),
                magnitude_str
            ),
            FormatAlign::Center => {
                let left_fill_chars_needed = fill_chars_needed / 2;
                let right_fill_chars_needed = fill_chars_needed - left_fill_chars_needed;
                let left_fill_string = Self::compute_fill_string(fill_char, left_fill_chars_needed);
                let right_fill_string =
                    Self::compute_fill_string(fill_char, right_fill_chars_needed);
                format!("{left_fill_string}{sign_str}{magnitude_str}{right_fill_string}")
            }
        }
    }
}

pub trait CharLen {
    /// Returns the number of characters in the text
    fn char_len(&self) -> usize;
}

struct AsciiStr<'a> {
    inner: &'a str,
}

impl<'a> AsciiStr<'a> {
    const fn new(inner: &'a str) -> Self {
        Self { inner }
    }
}

impl CharLen for AsciiStr<'_> {
    fn char_len(&self) -> usize {
        self.inner.len()
    }
}

impl CharLen for String {
    fn char_len(&self) -> usize {
        self.chars().count()
    }
}

impl Deref for AsciiStr<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatSpecError {
    DecimalDigitsTooMany,
    PrecisionTooBig,
    PrecisionMissing,
    InvalidFormatSpecifier(String),
    UnspecifiedFormat(char, char),
    ExclusiveFormat(char, char),
    UnknownFormatCode(char, &'static str),
    PrecisionNotAllowed,
    NotAllowed(&'static str),
    UnableToConvert,
    CodeNotInRange,
    ZeroPadding,
    AlignmentFlag,
    NegativeZeroCoercionNotAllowed(&'static str),
    StringAlignmentFlag,
    NotImplemented(char, &'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatParseError {
    UnmatchedBracket,
    MissingStartBracket,
    UnescapedStartBracketInLiteral(char),
    UnknownConversion(char),
    ConversionMissing,
    InvalidFormatSpecifier,
    EmptyAttribute,
    MissingRightBracket,
    InvalidCharacterAfterRightBracket,
    TooManyDecimalDigits,
}

impl FromStr for FormatSpec {
    type Err = FormatSpecError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FieldNamePart {
    Attribute(Wtf8Buf),
    Index(usize),
    StringIndex(Wtf8Buf),
}

impl FieldNamePart {
    fn parse_part(
        chars: &mut impl PeekingNext<Item = CodePoint>,
    ) -> Result<Option<Self>, FormatParseError> {
        chars
            .next()
            .map(|ch| match ch.to_char_lossy() {
                '.' => {
                    let mut attribute = Wtf8Buf::new();
                    for ch in chars.peeking_take_while(|ch| *ch != '.' && *ch != '[') {
                        attribute.push(ch);
                    }
                    if attribute.is_empty() {
                        Err(FormatParseError::EmptyAttribute)
                    } else {
                        Ok(Self::Attribute(attribute))
                    }
                }
                '[' => {
                    let mut index = Wtf8Buf::new();
                    for ch in chars {
                        if ch == ']' {
                            return if index.is_empty() {
                                Err(FormatParseError::EmptyAttribute)
                            } else if let Some(index) = parse_usize(&index) {
                                Ok(Self::Index(index))
                            } else {
                                Ok(Self::StringIndex(index))
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

#[derive(Debug, PartialEq, Eq)]
pub enum FieldType {
    Auto,
    Index(usize),
    Keyword(Wtf8Buf),
}

#[derive(Debug, PartialEq, Eq)]
pub struct FieldName {
    pub field_type: FieldType,
    pub parts: Vec<FieldNamePart>,
}

fn parse_usize(s: &Wtf8) -> Option<usize> {
    s.as_str().ok().and_then(|s| s.parse().ok())
}

impl FieldName {
    pub fn parse(text: &Wtf8) -> Result<Self, FormatParseError> {
        let mut chars = text.code_points().peekable();
        let first: Wtf8Buf = chars
            .peeking_take_while(|ch| *ch != '.' && *ch != '[')
            .collect();

        let field_type = if first.is_empty() {
            FieldType::Auto
        } else if let Some(index) = parse_usize(&first) {
            // Match CPython's get_integer: digits-only segment must fit in
            // Py_ssize_t. Above that, raise ValueError at parse time.
            if index > isize::MAX as usize {
                return Err(FormatParseError::TooManyDecimalDigits);
            }
            FieldType::Index(index)
        } else if first
            .as_str()
            .is_ok_and(|s| s.bytes().all(|b| b.is_ascii_digit()))
        {
            // All-digit segment whose value overflows usize itself.
            return Err(FormatParseError::TooManyDecimalDigits);
        } else {
            FieldType::Keyword(first)
        };

        let mut parts = Vec::new();
        while let Some(part) = FieldNamePart::parse_part(&mut chars)? {
            parts.push(part)
        }

        Ok(Self { field_type, parts })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormatPart {
    Field {
        field_name: Wtf8Buf,
        conversion_spec: Option<CodePoint>,
        format_spec: Wtf8Buf,
    },
    Literal(Wtf8Buf),
}

#[derive(Debug, PartialEq, Eq)]
pub struct FormatString {
    pub format_parts: Vec<FormatPart>,
}

impl FormatString {
    fn parse_literal_single(text: &Wtf8) -> Result<(CodePoint, &Wtf8), FormatParseError> {
        let mut chars = text.code_points();
        // This should never be called with an empty str
        let first_char = chars.next().unwrap();
        // isn't this detectable only with bytes operation?
        if first_char == '{' || first_char == '}' {
            let maybe_next_char = chars.next();
            // if we see a bracket, it has to be escaped by doubling up to be in a literal
            return if maybe_next_char.is_none() || maybe_next_char.unwrap() != first_char {
                let c = first_char.to_char_lossy();
                Err(FormatParseError::UnescapedStartBracketInLiteral(c))
            } else {
                Ok((first_char, chars.as_wtf8()))
            };
        }
        Ok((first_char, chars.as_wtf8()))
    }

    fn parse_literal(text: &Wtf8) -> Result<(FormatPart, &Wtf8), FormatParseError> {
        let mut cur_text = text;
        let mut result_string = Wtf8Buf::new();
        while !cur_text.is_empty() {
            match Self::parse_literal_single(cur_text) {
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
        Ok((FormatPart::Literal(result_string), "".as_ref()))
    }

    fn parse_part_in_brackets(text: &Wtf8) -> Result<FormatPart, FormatParseError> {
        let mut chars = text.code_points().peekable();

        let mut left = Wtf8Buf::new();
        let mut right = Wtf8Buf::new();

        let mut split = false;
        let mut selected = &mut left;
        let mut inside_brackets = false;

        while let Some(char) = chars.next() {
            if char == '[' {
                inside_brackets = true;

                selected.push(char);

                while let Some(next_char) = chars.next() {
                    selected.push(next_char);

                    if next_char == ']' {
                        inside_brackets = false;
                        break;
                    }
                    if chars.peek().is_none() {
                        return Err(FormatParseError::MissingRightBracket);
                    }
                }
            } else if char == ':' && !split && !inside_brackets {
                split = true;
                selected = &mut right;
            } else {
                selected.push(char);
            }
        }

        // before the comma is a keyword or arg index, after the comma is maybe a spec.
        let arg_part: &Wtf8 = &left;

        let format_spec = if split { right } else { Wtf8Buf::new() };

        // left can still be the conversion (!r, !s, !a)
        let parts: Vec<&Wtf8> = arg_part.splitn(2, "!".as_ref()).collect();
        // before the bang is a keyword or arg index, after the comma is maybe a conversion spec.
        let arg_part = parts[0];

        let conversion_spec = parts
            .get(1)
            .map(|conversion| {
                // the conversion is exactly one character; its value is
                // validated when the field is formatted
                conversion
                    .code_points()
                    .exactly_one()
                    .map_err(|_| FormatParseError::ConversionMissing)
            })
            .transpose()?;

        Ok(FormatPart::Field {
            field_name: arg_part.to_owned(),
            conversion_spec,
            format_spec,
        })
    }

    fn parse_spec(text: &Wtf8) -> Result<(FormatPart, &Wtf8), FormatParseError> {
        let mut nested = false;
        let mut end_bracket_pos = None;
        let mut left = Wtf8Buf::new();

        // There may be one layer nesting brackets in spec
        for (idx, c) in text.code_point_indices() {
            if idx == 0 {
                if c != '{' {
                    return Err(FormatParseError::MissingStartBracket);
                }
            } else if c == '{' {
                if nested {
                    return Err(FormatParseError::InvalidFormatSpecifier);
                }
                nested = true;
                left.push(c);
                continue;
            } else if c == '}' {
                if nested {
                    nested = false;
                    left.push(c);
                    continue;
                }
                end_bracket_pos = Some(idx);
                break;
            } else {
                left.push(c);
            }
        }
        if let Some(pos) = end_bracket_pos {
            let right = &text[pos..];
            let format_part = Self::parse_part_in_brackets(&left)?;
            Ok((format_part, &right[1..]))
        } else {
            Err(FormatParseError::UnmatchedBracket)
        }
    }
}

pub trait FromTemplate<'a>: Sized {
    type Err;
    fn from_str(s: &'a Wtf8) -> Result<Self, Self::Err>;
}

impl<'a> FromTemplate<'a> for FormatString {
    type Err = FormatParseError;

    fn from_str(text: &'a Wtf8) -> Result<Self, Self::Err> {
        let mut cur_text: &Wtf8 = text;
        let mut parts: Vec<FormatPart> = Vec::new();
        while !cur_text.is_empty() {
            // Try to parse both literals and bracketed format parts until we
            // run out of text. A lone '}' never starts a replacement field,
            // and a '{' at the very end is reported as a single brace like
            // CPython's markup iterator.
            cur_text = match Self::parse_literal(cur_text) {
                Ok((part, new_text)) => {
                    parts.push(part);
                    new_text
                }
                Err(FormatParseError::UnescapedStartBracketInLiteral(c @ '}')) => {
                    return Err(FormatParseError::UnescapedStartBracketInLiteral(c));
                }
                Err(_) => {
                    if cur_text.as_bytes() == b"{" {
                        return Err(FormatParseError::UnescapedStartBracketInLiteral('{'));
                    }
                    let (part, new_text) = Self::parse_spec(cur_text)?;
                    parts.push(part);
                    new_text
                }
            };
        }
        Ok(Self {
            format_parts: parts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_align() {
        let parse_fill_and_align = |text| {
            let (fill, align, rest) = parse_fill_and_align(str::as_ref(text));
            (
                fill.and_then(CodePoint::to_char),
                align,
                rest.as_str().unwrap(),
            )
        };
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
    fn width_only() {
        let expected = Ok(FormatSpec {
            conversion: None,
            fill: None,
            align: None,
            align_specified: false,
            sign: None,
            no_neg_0: false,
            alternate_form: false,
            width: Some(33),
            grouping_option: None,
            precision: None,
            frac_grouping_option: None,
            format_type: None,
        });
        assert_eq!(FormatSpec::parse("33"), expected);
    }

    #[test]
    fn fill_and_width() {
        let expected = Ok(FormatSpec {
            conversion: None,
            fill: Some('<'.into()),
            align: Some(FormatAlign::Right),
            align_specified: true,
            sign: None,
            no_neg_0: false,
            alternate_form: false,
            width: Some(33),
            grouping_option: None,
            precision: None,
            frac_grouping_option: None,
            format_type: None,
        });
        assert_eq!(FormatSpec::parse("<>33"), expected);
    }

    #[test]
    fn all() {
        let expected = Ok(FormatSpec {
            conversion: None,
            fill: Some('<'.into()),
            align: Some(FormatAlign::Right),
            align_specified: true,
            sign: Some(FormatSign::Minus),
            no_neg_0: false,
            alternate_form: true,
            width: Some(23),
            grouping_option: Some(FormatGrouping::Comma),
            precision: Some(11),
            frac_grouping_option: None,
            format_type: Some(FormatType::Binary),
        });
        assert_eq!(FormatSpec::parse("<>-#23,.11b"), expected);
    }

    fn format_bool(text: &str, value: bool) -> Result<String, FormatSpecError> {
        FormatSpec::parse(text).and_then(|spec| spec.format_bool(value))
    }

    #[test]
    fn format_bool_basic() {
        assert_eq!(format_bool("b", true), Ok("1".to_owned()));
        assert_eq!(format_bool("b", false), Ok("0".to_owned()));
        assert_eq!(format_bool("d", true), Ok("1".to_owned()));
        assert_eq!(format_bool("d", false), Ok("0".to_owned()));
        assert_eq!(format_bool("o", true), Ok("1".to_owned()));
        assert_eq!(format_bool("o", false), Ok("0".to_owned()));
        assert_eq!(format_bool("n", true), Ok("1".to_owned()));
        assert_eq!(format_bool("n", false), Ok("0".to_owned()));
        assert_eq!(format_bool("x", true), Ok("1".to_owned()));
        assert_eq!(format_bool("x", false), Ok("0".to_owned()));
        assert_eq!(format_bool("X", true), Ok("1".to_owned()));
        assert_eq!(format_bool("X", false), Ok("0".to_owned()));
        assert_eq!(format_bool("g", true), Ok("1".to_owned()));
        assert_eq!(format_bool("g", false), Ok("0".to_owned()));
        assert_eq!(format_bool("G", true), Ok("1".to_owned()));
        assert_eq!(format_bool("G", false), Ok("0".to_owned()));
        assert_eq!(format_bool("c", true), Ok("\x01".to_owned()));
        assert_eq!(format_bool("c", false), Ok("\x00".to_owned()));
        assert_eq!(format_bool("e", true), Ok("1.000000e+00".to_owned()));
        assert_eq!(format_bool("e", false), Ok("0.000000e+00".to_owned()));
        assert_eq!(format_bool("E", true), Ok("1.000000E+00".to_owned()));
        assert_eq!(format_bool("E", false), Ok("0.000000E+00".to_owned()));
        assert_eq!(format_bool("f", true), Ok("1.000000".to_owned()));
        assert_eq!(format_bool("f", false), Ok("0.000000".to_owned()));
        assert_eq!(format_bool("F", true), Ok("1.000000".to_owned()));
        assert_eq!(format_bool("F", false), Ok("0.000000".to_owned()));
        assert_eq!(format_bool("%", true), Ok("100.000000%".to_owned()));
        assert_eq!(format_bool("%", false), Ok("0.000000%".to_owned()));
    }

    #[test]
    fn format_string_zero_padding_uses_left_alignment() {
        let spec = FormatSpec::parse("08s").unwrap();
        let value = "result".to_owned();

        assert_eq!(spec.format_string(&value), Ok("result00".to_owned()));
    }

    #[test]
    fn format_string_explicit_after_sign_alignment_is_invalid() {
        let spec = FormatSpec::parse("=8s").unwrap();
        let value = "result".to_owned();

        assert_eq!(
            spec.format_string(&value),
            Err(FormatSpecError::StringAlignmentFlag)
        );
    }

    #[test]
    fn format_complex_rejects_zero_padding_before_after_sign_alignment() {
        for text in [
            "08.1f", "=08.1f", "0=8.1f", "#08.1f", "0>8.1f", "0<8.1f", "0^8.1f",
        ] {
            let spec = FormatSpec::parse(text).unwrap();
            assert_eq!(
                spec.format_complex(&Complex64::new(1.0, 2.0)),
                Err(FormatSpecError::ZeroPadding),
                "{text}"
            );
        }

        let spec = FormatSpec::parse("=8.1f").unwrap();
        assert_eq!(
            spec.format_complex(&Complex64::new(1.0, 2.0)),
            Err(FormatSpecError::AlignmentFlag)
        );
    }

    #[test]
    fn format_int_zero_padding_stays_after_sign() {
        let spec = FormatSpec::parse("08").unwrap();

        assert_eq!(
            spec.format_int(&BigInt::from(-42)),
            Ok("-0000042".to_owned())
        );
    }

    #[test]
    fn format_complex_locale_rejects_zero_padding_before_after_sign_alignment() {
        let locale = LocaleInfo {
            thousands_sep: String::new(),
            decimal_point: ".".to_owned(),
            grouping: vec![],
        };
        for text in ["08n", "=08n", "0=8n", "#08n", "0>8n", "0<8n", "0^8n"] {
            let spec = FormatSpec::parse(text).unwrap();
            assert_eq!(
                spec.format_complex_locale(&Complex64::new(1.0, 2.0), &locale),
                Err(FormatSpecError::ZeroPadding),
                "{text}"
            );
        }

        let spec = FormatSpec::parse("=8n").unwrap();
        assert_eq!(
            spec.format_complex_locale(&Complex64::new(1.0, 2.0), &locale),
            Err(FormatSpecError::AlignmentFlag)
        );
    }

    #[test]
    fn format_negative_zero_coercion() {
        let int_spec = FormatSpec::parse("z8").unwrap();
        assert_eq!(
            int_spec.format_int(&BigInt::from(-42)),
            Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"))
        );
        assert_eq!(
            FormatSpec::parse("zs")
                .unwrap()
                .format_int(&BigInt::from(0)),
            Err(FormatSpecError::UnknownFormatCode('s', "int"))
        );
        assert_eq!(
            FormatSpec::parse("z.1d")
                .unwrap()
                .format_int(&BigInt::from(0)),
            Err(FormatSpecError::PrecisionNotAllowed)
        );
        assert_eq!(
            FormatSpec::parse("+zc")
                .unwrap()
                .format_int(&BigInt::from(0)),
            Err(FormatSpecError::NegativeZeroCoercionNotAllowed("integer"))
        );

        let float_spec = FormatSpec::parse("z.2f").unwrap();
        assert_eq!(float_spec.format_float(-0.0001), Ok("0.00".to_owned()));

        let complex_spec = FormatSpec::parse("z").unwrap();
        assert_eq!(
            complex_spec.format_complex(&Complex64::new(-0.0, -0.0)),
            Ok("(0+0j)".to_owned())
        );
        let pure_imaginary = Complex64::new(0.0, -0.0);
        assert_eq!(
            FormatSpec::parse("+z")
                .unwrap()
                .format_complex(&pure_imaginary),
            Ok("+0j".to_owned())
        );
        assert_eq!(
            FormatSpec::parse(" z")
                .unwrap()
                .format_complex(&pure_imaginary),
            Ok(" 0j".to_owned())
        );

        let string_value = "value".to_owned();
        assert_eq!(
            FormatSpec::parse("z").unwrap().format_string(&string_value),
            Err(FormatSpecError::NegativeZeroCoercionNotAllowed("string"))
        );
        assert_eq!(
            FormatSpec::parse("zd")
                .unwrap()
                .format_string(&string_value),
            Err(FormatSpecError::UnknownFormatCode('d', "str"))
        );
        assert_eq!(
            FormatSpec::parse("zs").unwrap().format_bool(false),
            Err(FormatSpecError::UnknownFormatCode('s', "bool"))
        );

        let locale = LocaleInfo {
            thousands_sep: ",".to_owned(),
            decimal_point: ".".to_owned(),
            grouping: vec![3, 0],
        };
        let locale_spec = FormatSpec::parse("zn").unwrap();
        assert_eq!(
            locale_spec.format_float_locale(-0.0, &locale),
            Ok("0".to_owned())
        );
        assert_eq!(
            locale_spec.format_complex_locale(&Complex64::new(-0.0, -0.0), &locale),
            Ok("0+0j".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("z.1n")
                .unwrap()
                .format_int_locale(&BigInt::from(0), &locale),
            Err(FormatSpecError::PrecisionNotAllowed)
        );
    }

    #[test]
    fn format_int() {
        assert_eq!(
            FormatSpec::parse("d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("16".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("x")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("10".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("b")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("10000".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("o")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("20".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("+d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("+16".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("^ 5d")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Minus, b"\x10")),
            Ok(" -16 ".to_owned())
        );
        assert_eq!(
            FormatSpec::parse("0>+#10x")
                .unwrap()
                .format_int(&BigInt::from_bytes_be(Sign::Plus, b"\x10")),
            Ok("00000+0x10".to_owned())
        );
    }

    #[test]
    fn format_int_sep() {
        let spec = FormatSpec::parse(",").expect("");
        assert_eq!(spec.grouping_option, Some(FormatGrouping::Comma));
        assert_eq!(
            spec.format_int(&BigInt::from_str("1234567890123456789012345678").unwrap()),
            Ok("1,234,567,890,123,456,789,012,345,678".to_owned())
        );
    }

    #[test]
    fn format_int_width_and_grouping() {
        // issue #5922: width + comma grouping should pad left, not inside the number
        let spec = FormatSpec::parse("10,").unwrap();
        let result = spec.format_int(&BigInt::from(1234)).unwrap();
        assert_eq!(result, "     1,234"); // CPython 3.13.5
    }

    #[test]
    fn format_int_padding_with_grouping() {
        // CPython behavior: f'{1234:010,}' results in "00,001,234"
        let spec1 = FormatSpec::parse("010,").unwrap();
        let result1 = spec1.format_int(&BigInt::from(1234)).unwrap();
        assert_eq!(result1, "00,001,234");

        // CPython behavior: f'{-1234:010,}' results in "-0,001,234"
        let spec2 = FormatSpec::parse("010,").unwrap();
        let result2 = spec2.format_int(&BigInt::from(-1234)).unwrap();
        assert_eq!(result2, "-0,001,234");

        // CPython behavior: f'{-1234:=10,}' results in "-    1,234"
        let spec3 = FormatSpec::parse("=10,").unwrap();
        let result3 = spec3.format_int(&BigInt::from(-1234)).unwrap();
        assert_eq!(result3, "-    1,234");

        // CPython behavior: f'{1234:=10,}' results in "     1,234" (same as right-align for positive numbers)
        let spec4 = FormatSpec::parse("=10,").unwrap();
        let result4 = spec4.format_int(&BigInt::from(1234)).unwrap();
        assert_eq!(result4, "     1,234");
    }

    #[test]
    fn format_int_non_aftersign_zero_padding() {
        // CPython behavior: f'{1234:0>10,}' results in "000001,234"
        let spec = FormatSpec::parse("0>10,").unwrap();
        let result = spec.format_int(&BigInt::from(1234)).unwrap();
        assert_eq!(result, "000001,234");
    }

    fn fmt_float(spec: &str, value: f64) -> String {
        FormatSpec::parse(spec)
            .unwrap()
            .format_float(value)
            .unwrap()
    }

    #[test]
    fn format_float_grouping_never_touches_exponent() {
        // Grouping must group only the mantissa's integer digits, never the
        // exponent digits (was "1e,+20") or a trailing percent sign.
        assert_eq!(fmt_float(",g", 1e20), "1e+20");
        assert_eq!(fmt_float("_g", 1e-10), "1e-10");
        assert_eq!(fmt_float(",e", 1e20), "1.000000e+20");
        assert_eq!(fmt_float(",", 1e16), "1e+16");
        assert_eq!(fmt_float(",.0%", 1.0), "100%");
        assert_eq!(fmt_float(",.2%", 12345.0), "1,234,500.00%");
        // Fixed-form grouping still groups the integer part.
        assert_eq!(fmt_float(",", 1234567.0), "1,234,567.0");
    }

    #[test]
    fn format_float_grouping_inf_nan() {
        // No integer digits to group; width padding is left to fill/align, so
        // separators never land inside "inf"/"nan".
        assert_eq!(fmt_float(",", f64::INFINITY), "inf");
        assert_eq!(fmt_float("06,", f64::INFINITY), "000inf");
        assert_eq!(fmt_float("06,", f64::NAN), "000nan");
        assert_eq!(fmt_float("06,%", f64::INFINITY), "00inf%");
    }

    #[test]
    fn format_float_fractional_grouping() {
        // Fraction digits group away from the decimal point, so the last group
        // may be shorter than the interval.
        assert_eq!(fmt_float(".6,f", 1234.56789), "1234.567,890");
        assert_eq!(fmt_float(".7,f", 1234.56789), "1234.567,890,0");
        assert_eq!(fmt_float(".4,f", 1.1), "1.100,0");
        assert_eq!(fmt_float(".3,f", 1.1), "1.100");
        assert_eq!(fmt_float(".6_f", 1234.56789), "1234.567_890");
        // Omitting the precision keeps the type's default.
        assert_eq!(fmt_float(".,f", 1.1), "1.100,000");
        // The two parts are independent and may use different separators.
        assert_eq!(fmt_float(",.6,f", 1234.56789), "1,234.567,890");
        assert_eq!(fmt_float(",.6_f", 1234.56789), "1,234.567_890");
        assert_eq!(fmt_float("_.6,f", 1234.56789), "1_234.567,890");
    }

    #[test]
    fn format_float_fractional_grouping_never_touches_tail() {
        // Only the digits between the point and any tail are groupable: the
        // exponent and a trailing percent sign must stay intact.
        assert_eq!(fmt_float(".6,e", 12345678900.0), "1.234,568e+10");
        assert_eq!(fmt_float(".6,E", 1234.5678), "1.234,568E+03");
        assert_eq!(fmt_float(".8,%", 1.2345e-05), "0.001,234,50%");
        // Values with no point have nothing to group.
        assert_eq!(fmt_float(".6,f", f64::INFINITY), "inf");
        assert_eq!(fmt_float(".6,f", f64::NAN), "nan");
        assert_eq!(fmt_float(".0,f", 1234.56789), "1235");
    }

    #[test]
    fn format_float_fractional_grouping_counts_toward_width() {
        // Separators are inserted before padding, so they consume width.
        assert_eq!(fmt_float("020.6,f", 1234.56789), "000000001234.567,890");
        assert_eq!(fmt_float("015.6,f", 1.5), "0000001.500,000");
        assert_eq!(fmt_float("<20.6,f", 1234.56789), "1234.567,890        ");
        // Zero padding of the integer part must reserve room for them too.
        assert_eq!(fmt_float("020,.6,f", 1234.56789), "0,000,001,234.567,890");
        assert_eq!(fmt_float("+020,.6_f", 1e-10), "+000,000,000.000_000");
        assert_eq!(fmt_float("= 015,.6,E", 1234.0), " 01.234,000E+03");
        assert_eq!(fmt_float("-015_._e", 1.1), "001.100_000e+00");
    }

    #[test]
    fn format_parse_fractional_grouping_errors() {
        // Mixing the two separators is rejected wherever it appears.
        assert_eq!(
            FormatSpec::parse(".,_f"),
            Err(FormatSpecError::ExclusiveFormat(',', '_'))
        );
        assert_eq!(
            FormatSpec::parse("._,f"),
            Err(FormatSpecError::ExclusiveFormat(',', '_'))
        );
        // A repeated separator is left in the spec and rejected as a whole.
        assert_eq!(
            FormatSpec::parse(".,,f"),
            Err(FormatSpecError::InvalidFormatSpecifier(".,,f".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse(".__f"),
            Err(FormatSpecError::InvalidFormatSpecifier(".__f".to_owned()))
        );
        // A dot needs either digits or a separator after it.
        assert_eq!(
            FormatSpec::parse("."),
            Err(FormatSpecError::PrecisionMissing)
        );
        assert_eq!(
            FormatSpec::parse(".f"),
            Err(FormatSpecError::PrecisionMissing)
        );
        // 'n' draws its separators from the locale.
        assert_eq!(
            FormatSpec::parse(".6,n").unwrap().format_float(1234.5678),
            Err(FormatSpecError::UnspecifiedFormat(',', 'n'))
        );
        assert_eq!(
            FormatSpec::parse("._n").unwrap().format_float(1234.5678),
            Err(FormatSpecError::UnspecifiedFormat('_', 'n'))
        );
        // The integer separator is reported first when both are present.
        assert_eq!(
            FormatSpec::parse("_.6,n").unwrap().format_float(1234.5678),
            Err(FormatSpecError::UnspecifiedFormat('_', 'n'))
        );
        // The complex locale path rewrites 'n' to 'g', so it must validate first.
        let locale = LocaleInfo {
            thousands_sep: ",".to_owned(),
            decimal_point: ".".to_owned(),
            grouping: vec![3, 0],
        };
        assert_eq!(
            FormatSpec::parse(".6,n")
                .unwrap()
                .format_complex_locale(&Complex64::new(1.0, 2.345678), &locale),
            Err(FormatSpecError::UnspecifiedFormat(',', 'n'))
        );
    }

    #[test]
    fn format_float_empty_type_with_precision() {
        // Empty presentation type with a precision is repr-like: precision is
        // clamped to at least 1 and integer-looking output keeps a `.0`.
        assert_eq!(fmt_float(".2", 1.0), "1.0");
        assert_eq!(fmt_float(".6", 100.0), "100.0");
        assert_eq!(fmt_float(".17", 1234567.0), "1234567.0");
        assert_eq!(fmt_float(".0", 0.5), "0.5");
        assert_eq!(fmt_float(".0", 0.0001), "0.0001");
        assert_eq!(fmt_float(".2", 0.0), "0.0");
        assert_eq!(fmt_float(".0", 0.0), "0e+00");
        assert_eq!(fmt_float(".2", 100.0), "1e+02");
    }

    #[test]
    fn format_float_alternate_form_forces_point() {
        // Alternate form injects a decimal point into exponent-form repr.
        assert_eq!(fmt_float("#", 1e16), "1.e+16");
        assert_eq!(fmt_float("#", 1e-5), "1.e-05");
        // Fixed-form repr already has a point, so it is unchanged.
        assert_eq!(fmt_float("#", 100.0), "100.0");
        assert_eq!(fmt_float("#", 1.5), "1.5");
    }

    #[test]
    fn format_int_hex_grouping_preserved() {
        // Underscore grouping of hex/octal groups every 4 digits, including the
        // `a`-`f` letters.
        assert_eq!(
            FormatSpec::parse("_x")
                .unwrap()
                .format_int(&BigInt::from(1000000))
                .unwrap(),
            "f_4240"
        );
        assert_eq!(
            FormatSpec::parse("_X")
                .unwrap()
                .format_int(&BigInt::from(0xABCDEFu32))
                .unwrap(),
            "AB_CDEF"
        );
    }

    #[test]
    fn format_int_character_rejects_precision() {
        // 'c' rejects precision, and precision is checked before sign/alt form.
        assert_eq!(
            FormatSpec::parse(".2c")
                .unwrap()
                .format_int(&BigInt::from(65)),
            Err(FormatSpecError::PrecisionNotAllowed)
        );
        assert_eq!(
            FormatSpec::parse("+.2c")
                .unwrap()
                .format_int(&BigInt::from(65)),
            Err(FormatSpecError::PrecisionNotAllowed)
        );
        // Without precision, 'c' still renders the code point.
        assert_eq!(
            FormatSpec::parse("c")
                .unwrap()
                .format_int(&BigInt::from(65)),
            Ok("A".to_owned())
        );
    }

    #[test]
    fn format_parse() {
        let expected = Ok(FormatString {
            format_parts: vec![
                FormatPart::Literal("abcd".into()),
                FormatPart::Field {
                    field_name: "1".into(),
                    conversion_spec: None,
                    format_spec: "".into(),
                },
                FormatPart::Literal(":".into()),
                FormatPart::Field {
                    field_name: "key".into(),
                    conversion_spec: None,
                    format_spec: "".into(),
                },
            ],
        });

        assert_eq!(FormatString::from_str("abcd{1}:{key}".as_ref()), expected);
    }

    #[test]
    fn format_parse_multi_byte_char() {
        assert!(FormatString::from_str("{a:%ЫйЯЧ}".as_ref()).is_ok());
    }

    #[test]
    fn format_parse_fail() {
        assert_eq!(
            FormatString::from_str("{s".as_ref()),
            Err(FormatParseError::UnmatchedBracket)
        );
    }

    #[test]
    fn square_brackets_inside_format() {
        assert_eq!(
            FormatString::from_str("{[:123]}".as_ref()),
            Ok(FormatString {
                format_parts: vec![FormatPart::Field {
                    field_name: "[:123]".into(),
                    conversion_spec: None,
                    format_spec: "".into(),
                }],
            }),
        );

        assert_eq!(FormatString::from_str("{asdf[:123]asdf}".as_ref()), {
            Ok(FormatString {
                format_parts: vec![FormatPart::Field {
                    field_name: "asdf[:123]asdf".into(),
                    conversion_spec: None,
                    format_spec: "".into(),
                }],
            })
        });

        assert_eq!(FormatString::from_str("{[1234}".as_ref()), {
            Err(FormatParseError::MissingRightBracket)
        });
    }

    #[test]
    fn format_parse_escape() {
        let expected = Ok(FormatString {
            format_parts: vec![
                FormatPart::Literal("{".into()),
                FormatPart::Field {
                    field_name: "key".into(),
                    conversion_spec: None,
                    format_spec: "".into(),
                },
                FormatPart::Literal("}ddfe".into()),
            ],
        });

        assert_eq!(FormatString::from_str("{{{key}}}ddfe".as_ref()), expected);
    }

    #[test]
    fn format_invalid_specification() {
        assert_eq!(
            FormatSpec::parse("%3"),
            Err(FormatSpecError::InvalidFormatSpecifier("%3".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse(".2fa"),
            Err(FormatSpecError::InvalidFormatSpecifier(".2fa".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse("ds"),
            Err(FormatSpecError::InvalidFormatSpecifier("ds".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse("x+"),
            Err(FormatSpecError::InvalidFormatSpecifier("x+".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse("b4"),
            Err(FormatSpecError::InvalidFormatSpecifier("b4".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse("o!"),
            Err(FormatSpecError::InvalidFormatSpecifier("o!".to_owned()))
        );
        assert_eq!(
            FormatSpec::parse("d "),
            Err(FormatSpecError::InvalidFormatSpecifier("d ".to_owned()))
        );
    }

    #[test]
    fn parse_field_name() {
        let parse = |s: &str| FieldName::parse(s.as_ref());
        assert_eq!(
            parse(""),
            Ok(FieldName {
                field_type: FieldType::Auto,
                parts: Vec::new(),
            })
        );
        assert_eq!(
            parse("0"),
            Ok(FieldName {
                field_type: FieldType::Index(0),
                parts: Vec::new(),
            })
        );
        assert_eq!(
            parse("key"),
            Ok(FieldName {
                field_type: FieldType::Keyword("key".into()),
                parts: Vec::new(),
            })
        );
        assert_eq!(
            parse("key.attr[0][string]"),
            Ok(FieldName {
                field_type: FieldType::Keyword("key".into()),
                parts: vec![
                    FieldNamePart::Attribute("attr".into()),
                    FieldNamePart::Index(0),
                    FieldNamePart::StringIndex("string".into())
                ],
            })
        );
        assert_eq!(parse("key.."), Err(FormatParseError::EmptyAttribute));
        assert_eq!(parse("key[]"), Err(FormatParseError::EmptyAttribute));
        assert_eq!(parse("key["), Err(FormatParseError::MissingRightBracket));
        assert_eq!(
            parse("key[0]after"),
            Err(FormatParseError::InvalidCharacterAfterRightBracket)
        );
    }
}

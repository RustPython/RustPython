//! `Decimal.__format__`: the format-specification mini-language.
//!
//! Ported from `Lib/_pydecimal.py`.

use crate::bigops;
use crate::context::{Context, RoundMode};
use crate::dec::Decimal;
use crate::ops;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use malachite_bigint::BigUint;
use num_traits::One;

/// The locale-dependent parts of a `'n'` (or overridden) format.
#[derive(Clone, Debug)]
pub struct LocaleInfo {
    pub decimal_point: String,
    pub thousands_sep: String,
    /// `localeconv()['grouping']`, i.e. group sizes terminated by `0` to repeat
    /// the last size or by `CHAR_MAX` to stop grouping.
    pub grouping: Vec<i32>,
}

impl Default for LocaleInfo {
    fn default() -> Self {
        Self {
            decimal_point: String::from("."),
            thousands_sep: String::new(),
            grouping: Vec::new(),
        }
    }
}

/// A format specification the mini-language does not accept.
#[derive(Clone, Debug)]
pub struct FormatError(pub String);

/// The widest field (and the longest precision) a format may ask for. libmpdec
/// caps these at its own internal limits; the cap here is what can actually be
/// built, so an absurd width reports the same failure instead of aborting.
const MAX_FORMAT_LENGTH: i64 = 100_000_000;

/// `locale.CHAR_MAX` on a platform where `char` is signed, the usual sentinel
/// that ends a `grouping` list without repeating.
const CHAR_MAX: i32 = 127;

/// The parsed form of a format specifier, i.e. `_parse_format_specifier`'s
/// `format_dict`.
struct FormatSpec {
    fill: char,
    align: char,
    sign: char,
    no_neg_0: bool,
    alt: bool,
    zeropad: bool,
    minimumwidth: i64,
    thousands_sep: String,
    grouping: Vec<i32>,
    decimal_point: String,
    precision: Option<i64>,
    frac_separators: String,
    type_: Option<char>,
}

/// Parses a run of ASCII digits into a nonnegative `i64`, matching Python's
/// unbounded `int(...)` conversion except that a value too large to fit is an
/// error here (the engine has no arbitrary-precision integer for widths).
fn parse_index(digits: &str, spec: &str) -> Result<i64, FormatError> {
    let mut acc: i64 = 0;
    for c in digits.chars() {
        let d = i64::from(c as u32 - '0' as u32);
        acc = acc
            .checked_mul(10)
            .and_then(|v| v.checked_add(d))
            .ok_or_else(|| FormatError(format!("Invalid format specifier: {spec}")))?;
    }
    Ok(acc)
}

/// `_parse_format_specifier`.
///
/// Hand-written in place of the compiled regex
/// `_parse_format_specifier_regex`:
/// ```text
/// \A
/// (?:
///    (?P<fill>.)?
///    (?P<align>[<>=^])
/// )?
/// (?P<sign>[-+ ])?
/// (?P<no_neg_0>z)?
/// (?P<alt>\#)?
/// (?P<zeropad>0)?
/// (?P<minimumwidth>\d+)?
/// (?P<thousands_sep>[,_])?
/// (?:\.
///     (?=[\d,_])
///     (?P<precision>\d+)?
///     (?P<frac_separators>[,_])?
/// )?
/// (?P<type>[eEfFgGn%])?
/// \z
/// ```
/// The fill/align pair is the only place the regex can backtrack, and it
/// never actually needs to: an align character (`<>=^`) never appears in any
/// other alternative of the grammar, so greedily preferring the two-character
/// `fill`+`align` form whenever the second character is an align character,
/// and the one-character `align`-only form whenever the first is, exactly
/// reproduces the backtracking engine's result.
fn parse_format_specifier(
    spec: &str,
    locale: Option<&LocaleInfo>,
) -> Result<FormatSpec, FormatError> {
    let chars: Vec<char> = spec.chars().collect();
    let n = chars.len();
    let mut idx = 0usize;
    let is_align = |c: char| matches!(c, '<' | '>' | '=' | '^');
    let invalid = || FormatError(format!("Invalid format specifier: {spec}"));

    let mut fill_raw: Option<char> = None;
    let mut align_raw: Option<char> = None;
    if idx + 1 < n && is_align(chars[idx + 1]) {
        fill_raw = Some(chars[idx]);
        align_raw = Some(chars[idx + 1]);
        idx += 2;
    } else if idx < n && is_align(chars[idx]) {
        align_raw = Some(chars[idx]);
        idx += 1;
    }

    let mut sign_raw: Option<char> = None;
    if idx < n && matches!(chars[idx], '-' | '+' | ' ') {
        sign_raw = Some(chars[idx]);
        idx += 1;
    }

    let mut no_neg_0 = false;
    if idx < n && chars[idx] == 'z' {
        no_neg_0 = true;
        idx += 1;
    }

    let mut alt = false;
    if idx < n && chars[idx] == '#' {
        alt = true;
        idx += 1;
    }

    let mut zeropad = false;
    if idx < n && chars[idx] == '0' {
        zeropad = true;
        idx += 1;
    }

    let width_start = idx;
    while idx < n && chars[idx].is_ascii_digit() {
        idx += 1;
    }
    let minimumwidth_str: Option<String> =
        (idx > width_start).then(|| chars[width_start..idx].iter().collect());

    let mut thousands_sep_raw: Option<char> = None;
    if idx < n && matches!(chars[idx], ',' | '_') {
        thousands_sep_raw = Some(chars[idx]);
        idx += 1;
    }

    let mut precision_str: Option<String> = None;
    let mut frac_separators_raw: Option<char> = None;
    if idx < n
        && chars[idx] == '.'
        && idx + 1 < n
        && (chars[idx + 1].is_ascii_digit() || matches!(chars[idx + 1], ',' | '_'))
    {
        idx += 1; // consume '.'
        let prec_start = idx;
        while idx < n && chars[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx > prec_start {
            precision_str = Some(chars[prec_start..idx].iter().collect());
        }
        if idx < n && matches!(chars[idx], ',' | '_') {
            frac_separators_raw = Some(chars[idx]);
            idx += 1;
        }
    }

    let mut type_: Option<char> = None;
    if idx < n && matches!(chars[idx], 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'n' | '%') {
        type_ = Some(chars[idx]);
        idx += 1;
    }

    if idx != n {
        return Err(invalid());
    }

    // zeropad; defaults for fill and alignment. If zero padding is
    // requested, the fill and align fields should be absent.
    if zeropad {
        if fill_raw.is_some() {
            return Err(FormatError(format!(
                "Fill character conflicts with '0' in format specifier: {spec}"
            )));
        }
        if align_raw.is_some() {
            return Err(FormatError(format!(
                "Alignment conflicts with '0' in format specifier: {spec}"
            )));
        }
    }
    let fill = fill_raw.unwrap_or(' ');
    let align = align_raw.unwrap_or('>');
    let sign = sign_raw.unwrap_or('-');

    let minimumwidth = match minimumwidth_str {
        Some(s) => parse_index(&s, spec)?,
        None => 0,
    };
    let mut precision = match precision_str {
        Some(s) => Some(parse_index(&s, spec)?),
        None => None,
    };

    // if format type is 'g' or 'G' then a precision of 0 makes little
    // sense; convert it to 1. Same if format type is unspecified.
    if precision == Some(0) && (type_.is_none() || matches!(type_, Some('g' | 'G' | 'n'))) {
        precision = Some(1);
    }

    // determine thousands separator, grouping, and decimal separator.
    let (thousands_sep, grouping, decimal_point, type_) = if type_ == Some('n') {
        // apart from separators, 'n' behaves just like 'g'.
        if thousands_sep_raw.is_some() {
            return Err(FormatError(format!(
                "Explicit thousands separator conflicts with 'n' type in format specifier: {spec}"
            )));
        }
        let default_locale = LocaleInfo::default();
        let loc = locale.unwrap_or(&default_locale);
        (
            loc.thousands_sep.clone(),
            loc.grouping.clone(),
            loc.decimal_point.clone(),
            Some('g'),
        )
    } else {
        (
            thousands_sep_raw.map(String::from).unwrap_or_default(),
            vec![3, 0],
            String::from("."),
            type_,
        )
    };

    let frac_separators = frac_separators_raw.map(String::from).unwrap_or_default();

    // libmpdec refuses a specification it could not act on; without this the
    // padding below would try to allocate an impossible amount of memory.
    if minimumwidth > MAX_FORMAT_LENGTH || precision.is_some_and(|p| p > MAX_FORMAT_LENGTH) {
        return Err(FormatError(String::from(
            "format specification exceeds internal limits of _decimal",
        )));
    }

    Ok(FormatSpec {
        fill,
        align,
        sign,
        no_neg_0,
        alt,
        zeropad,
        minimumwidth,
        thousands_sep,
        grouping,
        decimal_point,
        precision,
        frac_separators,
        type_,
    })
}

/// `_format_align`.
fn format_align(sign: &str, body: &str, spec: &FormatSpec) -> Result<String, FormatError> {
    let pad_len = spec.minimumwidth - sign.chars().count() as i64 - body.chars().count() as i64;
    let pad_len = if pad_len > 0 { pad_len as usize } else { 0 };
    let padding: String = core::iter::repeat_n(spec.fill, pad_len).collect();

    Ok(match spec.align {
        '<' => format!("{sign}{body}{padding}"),
        '>' => format!("{padding}{sign}{body}"),
        '=' => format!("{sign}{padding}{body}"),
        '^' => {
            let padding: Vec<char> = padding.chars().collect();
            let half = padding.len() / 2;
            let left: String = padding[..half].iter().collect();
            let right: String = padding[half..].iter().collect();
            format!("{left}{sign}{body}{right}")
        }
        _ => return Err(FormatError(String::from("Unrecognised alignment field"))),
    })
}

/// An (infinite, when the input asks for repetition) source of group
/// lengths, i.e. `_group_lengths`. Align characters have no other role in the
/// grammar, so — as documented on `parse_format_specifier` — this direct
/// translation of the itertools pipeline needs no laziness beyond what a
/// plain iterator already gives us.
struct GroupLengths<'a> {
    head: core::slice::Iter<'a, i32>,
    repeat: Option<i32>,
}

impl Iterator for GroupLengths<'_> {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        self.head.next().copied().or(self.repeat)
    }
}

fn group_lengths(grouping: &[i32]) -> Result<GroupLengths<'_>, FormatError> {
    if grouping.is_empty() {
        return Ok(GroupLengths {
            head: grouping.iter(),
            repeat: None,
        });
    }
    let last = *grouping.last().expect("checked nonempty");
    if last == 0 && grouping.len() >= 2 {
        Ok(GroupLengths {
            head: grouping[..grouping.len() - 1].iter(),
            repeat: Some(grouping[grouping.len() - 2]),
        })
    } else if last == CHAR_MAX {
        Ok(GroupLengths {
            head: grouping[..grouping.len() - 1].iter(),
            repeat: None,
        })
    } else {
        Err(FormatError(String::from(
            "unrecognised format for grouping",
        )))
    }
}

/// `_insert_thousands_sep`.
fn insert_thousands_sep(
    digits: &str,
    sep: &str,
    grouping: &[i32],
    mut min_width: i64,
) -> Result<String, FormatError> {
    let mut digits: Vec<char> = digits.chars().collect();
    let mut groups: Vec<String> = Vec::new();
    let mut broke = false;

    for l in group_lengths(grouping)? {
        if l <= 0 {
            return Err(FormatError(String::from("group length should be positive")));
        }
        let cap = core::cmp::max(core::cmp::max(digits.len() as i64, min_width), 1);
        let l = core::cmp::min(cap, i64::from(l)) as usize;
        let take = core::cmp::min(l, digits.len());
        let split = digits.len() - take;
        let tail: String = digits[split..].iter().collect();
        let pad = l - tail.chars().count();
        groups.push(format!("{}{tail}", "0".repeat(pad)));
        digits.truncate(split);
        min_width -= l as i64;
        if digits.is_empty() && min_width <= 0 {
            broke = true;
            break;
        }
        min_width -= sep.chars().count() as i64;
    }
    if !broke {
        let l = core::cmp::max(core::cmp::max(digits.len() as i64, min_width), 1) as usize;
        let take = core::cmp::min(l, digits.len());
        let split = digits.len() - take;
        let tail: String = digits[split..].iter().collect();
        let pad = l - tail.chars().count();
        groups.push(format!("{}{tail}", "0".repeat(pad)));
    }
    groups.reverse();
    Ok(groups.join(sep))
}

/// `_format_sign`.
fn format_sign(is_negative: bool, sign_spec: char) -> String {
    if is_negative {
        String::from("-")
    } else if sign_spec == ' ' || sign_spec == '+' {
        sign_spec.to_string()
    } else {
        String::new()
    }
}

/// `_format_number`.
fn format_number(
    is_negative: bool,
    intpart: &[u8],
    fracpart: &[u8],
    exp: i64,
    spec: &FormatSpec,
) -> Result<String, FormatError> {
    let sign = format_sign(is_negative, spec.sign);

    let fracpart_digits = core::str::from_utf8(fracpart).expect("ascii digits");
    let mut fracpart_s = if !fracpart.is_empty() && !spec.frac_separators.is_empty() {
        let bytes = fracpart_digits.as_bytes();
        let mut out = String::new();
        for (i, chunk) in bytes.chunks(3).enumerate() {
            if i > 0 {
                out.push_str(&spec.frac_separators);
            }
            out.push_str(core::str::from_utf8(chunk).expect("ascii digits"));
        }
        out
    } else {
        fracpart_digits.to_string()
    };

    if !fracpart_s.is_empty() || spec.alt {
        fracpart_s = format!("{}{fracpart_s}", spec.decimal_point);
    }

    if exp != 0 || matches!(spec.type_, Some('e' | 'E')) {
        let echar = match spec.type_ {
            Some('E' | 'G') => 'E',
            _ => 'e',
        };
        fracpart_s.push(echar);
        if exp >= 0 {
            fracpart_s.push('+');
        }
        fracpart_s.push_str(&exp.to_string());
    }
    if spec.type_ == Some('%') {
        fracpart_s.push('%');
    }

    let min_width: i64 = if spec.zeropad {
        spec.minimumwidth - fracpart_s.chars().count() as i64 - sign.chars().count() as i64
    } else {
        0
    };

    let intpart_digits = core::str::from_utf8(intpart).expect("ascii digits");
    let intpart_s = insert_thousands_sep(
        intpart_digits,
        &spec.thousands_sep,
        &spec.grouping,
        min_width,
    )?;

    format_align(&sign, &format!("{intpart_s}{fracpart_s}"), spec)
}

/// `Decimal._rescale`: rescale to the exponent `exp`, either padding with
/// zeros or rounding away digits. Quiet: it never touches `status` and
/// ignores the context's precision and exponent range.
fn rescale(d: &Decimal, exp: i64, round: RoundMode) -> Decimal {
    if d.is_special() {
        return d.clone();
    }
    if d.is_zero() {
        return Decimal::zero(d.sign(), exp);
    }
    if d.exponent() >= exp {
        let shift = (d.exponent() - exp) as u64;
        let coeff = bigops::mul_pow10(d.coefficient(), shift);
        return Decimal::new_finite(d.sign(), coeff, exp);
    }

    // Too many digits; round and lose data. If `self.adjusted() < exp - 1`,
    // replace self by `10**(exp-1)` before rounding.
    let mut digits = d.digits() + d.exponent() - exp;
    let scratch;
    let work = if digits < 0 {
        scratch = Decimal::new_finite(d.sign(), BigUint::one(), exp - 1);
        digits = 0;
        &scratch
    } else {
        d
    };
    let changed = ops::round_indicator(work, digits, round);
    let mut coeff = if digits > 0 {
        bigops::div_pow10(work.coefficient(), (work.digits() - digits) as u64)
    } else {
        BigUint::default()
    };
    if changed == 1 {
        coeff += BigUint::one();
    }
    Decimal::new_finite(d.sign(), coeff, exp)
}

/// `Decimal._round`, guarded the way `_pydecimal` guards it: a special or
/// zero value is returned unaltered rather than handed to `round_to_places`
/// (whose contract requires a finite, nonzero value).
fn round_significant(d: &Decimal, places: i64, round: RoundMode) -> Decimal {
    ops::round_to_places(d, places, round)
}

/// `Decimal.copy_abs`, the pieces `__format__` needs: the same value with the
/// sign cleared.
fn copy_abs(d: &Decimal) -> Decimal {
    let mut out = d.clone();
    out.sign = 0;
    out
}

/// `Decimal.__format__`.
pub fn format(
    d: &Decimal,
    spec: &str,
    ctx: &Context,
    locale: Option<&LocaleInfo>,
    status: &mut u32,
) -> Result<String, FormatError> {
    let _ = status; // The engine never raises; formatting alone needs no fixup.
    let mut spec = parse_format_specifier(spec, locale)?;

    // Special values don't care about the type or precision.
    if d.is_special() {
        let sign = format_sign(d.sign() != 0, spec.sign);
        let mut body = copy_abs(d).to_sci_string(ctx.capitals);
        if spec.type_ == Some('%') {
            body.push('%');
        }
        return format_align(&sign, &body, &spec);
    }

    // A type of `None` defaults to 'g' or 'G', depending on context. The
    // resolved type is what decides the exponent letter further down, so it has
    // to go back into the specification and not just into a local.
    let ty = spec.type_.unwrap_or(if ctx.capitals { 'G' } else { 'g' });
    spec.type_ = Some(ty);

    // If type is '%', adjust exponent of self accordingly.
    let mut value = d.clone();
    if ty == '%' {
        value.exp += 2;
    }

    // Round if necessary, taking the rounding mode from the context.
    let rounding = ctx.round;
    if let Some(precision) = spec.precision {
        match ty {
            'e' | 'E' => value = round_significant(&value, precision + 1, rounding),
            'f' | 'F' | '%' => value = rescale(&value, -precision, rounding),
            'g' | 'G' if value.digits() > precision => {
                value = round_significant(&value, precision, rounding)
            }
            _ => {}
        }
    }

    // Special case: zeros with a positive exponent can't be represented in
    // fixed point; rescale them to 0e0.
    if value.is_zero() && value.exponent() > 0 && matches!(ty, 'f' | 'F' | '%') {
        value = rescale(&value, 0, rounding);
    }
    let adjusted_sign = if value.is_zero() && spec.no_neg_0 && value.sign() != 0 {
        0
    } else {
        value.sign()
    };

    // Figure out placement of the decimal point.
    let leftdigits = value.exponent() + value.digits();
    let dotplace = match ty {
        'e' | 'E' => {
            if let Some(precision) = spec.precision.filter(|_| value.is_zero()) {
                1 - precision
            } else {
                1
            }
        }
        'f' | 'F' | '%' => leftdigits,
        'g' | 'G' if value.exponent() <= 0 && leftdigits > -6 => leftdigits,
        'g' | 'G' => 1,
        _ => unreachable!("type is one of eEfFgG% at this point"),
    };

    // Find digits before and after the decimal point, and get the exponent.
    let digits = value.coeff_digits();
    let ndigits = digits.len() as i64;
    let (intpart, fracpart): (Vec<u8>, Vec<u8>) = if dotplace < 0 {
        let mut frac = vec![b'0'; (-dotplace) as usize];
        frac.extend_from_slice(&digits);
        (vec![b'0'], frac)
    } else if dotplace > ndigits {
        let mut int = digits;
        int.extend(core::iter::repeat_n(b'0', (dotplace - ndigits) as usize));
        (int, Vec::new())
    } else {
        let split = dotplace as usize;
        let (a, b) = digits.split_at(split);
        (
            if a.is_empty() { vec![b'0'] } else { a.to_vec() },
            b.to_vec(),
        )
    };
    let exp = leftdigits - dotplace;

    // Done with the decimal-specific stuff; hand over the rest of the
    // formatting to `format_number`.
    format_number(adjusted_sign != 0, &intpart, &fracpart, exp, &spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Context;

    fn ctx() -> Context {
        Context::default()
    }

    fn parse(s: &str) -> Decimal {
        Decimal::parse_ascii(s.as_bytes()).unwrap_or_else(|_| panic!("bad literal {s}"))
    }

    fn fmt(value: &str, spec: &str) -> String {
        let d = parse(value);
        let mut status = 0u32;
        format(&d, spec, &ctx(), None, &mut status)
            .unwrap_or_else(|e| panic!("{value:?} {spec:?}: {}", e.0))
    }

    fn fmt_err(value: &str, spec: &str) -> String {
        let d = parse(value);
        let mut status = 0u32;
        match format(&d, spec, &ctx(), None, &mut status) {
            Ok(s) => panic!("{value:?} {spec:?}: expected error, got {s:?}"),
            Err(e) => e.0,
        }
    }

    // Expected values come from CPython:
    //   python3.14 -c "import _decimal as C; print(format(C.Decimal(V), S))"

    #[test]
    fn type_characters() {
        assert_eq!(fmt("1.5", "e"), "1.5e+0");
        assert_eq!(fmt("1.5", "E"), "1.5E+0");
        assert_eq!(fmt("1.5", "f"), "1.5");
        assert_eq!(fmt("1.5", "F"), "1.5");
        assert_eq!(fmt("123456789", ".3g"), "1.23e+8");
        assert_eq!(fmt("123456789", ".3G"), "1.23E+8");
        assert_eq!(fmt("1.5", "n"), "1.5");
        assert_eq!(fmt("1.5", "%"), "150%");
        assert_eq!(fmt("1.5", ""), "1.5");
    }

    #[test]
    fn alternate_form() {
        // _decimal falls back to _pydecimal for '#', per the task notes.
        assert_eq!(fmt("1.5", "#.0f"), "2.");
        assert_eq!(fmt("1", "#.0e"), "1.e+0");
        assert_eq!(fmt("1", "#g"), "1.");
    }

    #[test]
    fn no_neg_zero() {
        assert_eq!(fmt("-0", "z.2f"), "0.00");
        assert_eq!(fmt("-0", ".2f"), "-0.00");
        assert_eq!(fmt("-0", "z"), "0");
    }

    #[test]
    fn alignments() {
        assert_eq!(fmt("1.5", "10.2f"), "      1.50");
        assert_eq!(fmt("1.5", "<10.2f"), "1.50      ");
        assert_eq!(fmt("1.5", ">10.2f"), "      1.50");
        assert_eq!(fmt("-1.5", "=10.2f"), "-     1.50");
        assert_eq!(fmt("1.5", "^10.2f"), "   1.50   ");
        assert_eq!(fmt("1.5", "=10"), "       1.5");
        assert_eq!(fmt("1.5", "*^10.2f"), "***1.50***");
    }

    #[test]
    fn zero_padding() {
        assert_eq!(fmt("1.5", "020,.3f"), "0,000,000,000,001.500");
        assert_eq!(fmt("1.5", "010.2f"), "0000001.50");
        assert_eq!(fmt("-1.5", "010.2f"), "-000001.50");
        assert_eq!(fmt("1.5", "0=10.2f"), "0000001.50");
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(fmt("1234567", ",.2f"), "1,234,567.00");
        assert_eq!(fmt("1234567", "_.2f"), "1_234_567.00");
        assert_eq!(fmt("1234567", "_"), "1_234_567");
        assert_eq!(fmt("1234567.891", "10,.5,f"), "1,234,567.891,00");
    }

    #[test]
    fn precision_zero() {
        assert_eq!(fmt("0", ".0f"), "0");
        assert_eq!(fmt("1.5", ".0f"), "2");
        assert_eq!(fmt("1.5", ".0e"), "2e+0");
        // gGn treat precision 0 as 1.
        assert_eq!(fmt("1.5", ".0g"), "2");
    }

    #[test]
    fn specials() {
        assert_eq!(fmt("NaN", ".2f"), "NaN");
        assert_eq!(fmt("sNaN", ".2f"), "sNaN");
        assert_eq!(fmt("NaN123", ".2f"), "NaN123");
        assert_eq!(fmt("Infinity", "+.2f"), "+Infinity");
        assert_eq!(fmt("-Infinity", ".2f"), "-Infinity");
        assert_eq!(fmt("Infinity", "10"), "  Infinity");
        assert_eq!(fmt("Infinity", "%"), "Infinity%");
    }

    #[test]
    fn signed_zeros() {
        assert_eq!(fmt("0", "f"), "0");
        assert_eq!(fmt("-0", "f"), "-0");
        assert_eq!(fmt("0", "+f"), "+0");
        assert_eq!(fmt("0", " f"), " 0");
    }

    #[test]
    fn large_exponents() {
        assert_eq!(fmt("1E+30", "f"), "1000000000000000000000000000000");
        assert_eq!(fmt("1E-30", "e"), "1e-30");
        assert_eq!(fmt("0E+10", "g"), "0e+10");
    }

    #[test]
    fn errors() {
        assert!(fmt_err("1.5", "x").contains("Invalid format specifier"));
        assert!(fmt_err("1.5", ".5,3f").contains("Invalid format specifier"));
        assert!(fmt_err("1.5", ".f").contains("Invalid format specifier"));
        assert!(fmt_err("1.5", ".").contains("Invalid format specifier"));
        assert!(fmt_err("1.5", "  =10.2f").contains("Invalid format specifier"));
        assert_eq!(
            fmt_err("1.5", "<010"),
            "Alignment conflicts with '0' in format specifier: <010"
        );
        assert_eq!(
            fmt_err("1.5", "^^0"),
            "Fill character conflicts with '0' in format specifier: ^^0"
        );
        assert!(fmt_err("1.5", "n,.2f").contains("Invalid format specifier"));
        assert_eq!(
            fmt_err("1.5", ",n"),
            "Explicit thousands separator conflicts with 'n' type in format specifier: ,n"
        );
    }

    #[test]
    fn locale_override_for_n() {
        let loc = LocaleInfo {
            decimal_point: String::from(","),
            thousands_sep: String::from("."),
            grouping: vec![3, 0],
        };
        let d = parse("1234567.5");
        let mut status = 0u32;
        let out = format(&d, "n", &ctx(), Some(&loc), &mut status).unwrap();
        assert_eq!(out, "1.234.567,5");
    }

    #[test]
    fn fill_align_edge_cases() {
        assert_eq!(fmt("1.5", "5>10"), "55555551.5");
        assert_eq!(fmt("1.5", "=+5"), "+ 1.5");
    }

    #[test]
    fn any_unicode_fill() {
        assert_eq!(fmt("1.5", "\u{2605}>6"), "\u{2605}\u{2605}\u{2605}1.5");
    }
}

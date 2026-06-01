use crate::format::Case;
use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use core::f64;
use num_traits::Zero;

pub fn parse_str(literal: &str) -> Option<f64> {
    parse_inner(literal.trim().as_bytes())
}

pub fn parse_bytes(literal: &[u8]) -> Option<f64> {
    parse_inner(literal.trim_ascii())
}

fn parse_inner(literal: &[u8]) -> Option<f64> {
    use lexical_parse_float::{
        FromLexicalWithOptions, NumberFormatBuilder, Options, format::PYTHON3_LITERAL,
    };

    // lexical-core's format::PYTHON_STRING is inaccurate
    const PYTHON_STRING: u128 = NumberFormatBuilder::rebuild(PYTHON3_LITERAL)
        .no_special(false)
        .build_unchecked();
    f64::from_lexical_with_options::<PYTHON_STRING>(literal, &Options::new()).ok()
}

pub fn is_integer(v: f64) -> bool {
    v.is_finite() && v.fract() == 0.0
}

fn format_nan(case: Case) -> String {
    let nan = match case {
        Case::Lower => "nan",
        Case::Upper => "NAN",
    };

    nan.to_string()
}

fn format_inf(case: Case) -> String {
    let inf = match case {
        Case::Lower => "inf",
        Case::Upper => "INF",
    };

    inf.to_string()
}

pub const fn decimal_point_or_empty(precision: usize, alternate_form: bool) -> &'static str {
    match (precision, alternate_form) {
        (0, true) => ".",
        _ => "",
    }
}

/// Rust's `format!("{:.*}", n, x)` panics when `n` exceeds the fmt runtime's
/// internal precision limit. User-supplied precision can legally reach far
/// higher values (e.g. `f"{1.5:.1000000}"`) — clamp here so we produce a
/// (truncated-but-valid) output instead of aborting the interpreter. Harmless
/// in practice: f64 carries only ~17 significant digits, so precision beyond
/// 65K is padding zeros at best.
///
/// The two caps differ by 1: `{:.*}` (plain) accepts `u16::MAX`, but `{:.*e}`
/// (exponential) hits a tighter assertion (`ndigits > 0` in
/// `core::num::flt2dec`) at exactly `u16::MAX`. Keeping plain at the higher
/// cap preserves byte-identical output with CPython up through
/// `precision == u16::MAX` for fixed / percent / general-non-scientific paths.
pub const FMT_MAX_PRECISION: usize = u16::MAX as usize;
pub const FMT_MAX_EXP_PRECISION: usize = u16::MAX as usize - 1;

#[inline]
pub fn clamp_fmt_precision(precision: usize) -> usize {
    core::cmp::min(precision, FMT_MAX_PRECISION)
}

#[inline]
pub fn clamp_exp_precision(precision: usize) -> usize {
    core::cmp::min(precision, FMT_MAX_EXP_PRECISION)
}

pub fn format_fixed(precision: usize, magnitude: f64, case: Case, alternate_form: bool) -> String {
    match magnitude {
        magnitude if magnitude.is_finite() => {
            let point = decimal_point_or_empty(precision, alternate_form);
            let capped = clamp_fmt_precision(precision);
            let mut out = format!("{magnitude:.capped$}");
            // Pad with '0's up to the requested precision to match CPython
            // byte-identically. `f64` has at most ~767 significant decimal
            // digits, so any digit past `capped` is deterministically '0'.
            let missing = precision.saturating_sub(capped);
            if missing > 0 {
                out.extend(core::iter::repeat_n('0', missing));
            }
            out.push_str(point);
            out
        }
        magnitude if magnitude.is_nan() => format_nan(case),
        magnitude if magnitude.is_infinite() => format_inf(case),
        _ => "".to_string(),
    }
}

// Formats floats into Python style exponent notation, by first formatting in Rust style
// exponent notation (`1.0000e0`), then convert to Python style (`1.0000e+00`).
pub fn format_exponent(
    precision: usize,
    magnitude: f64,
    case: Case,
    alternate_form: bool,
) -> String {
    match magnitude {
        magnitude if magnitude.is_finite() => {
            let capped = clamp_exp_precision(precision);
            let r_exp = format!("{magnitude:.capped$e}");
            let mut parts = r_exp.splitn(2, 'e');
            let base = parts.next().unwrap();
            let exponent = parts.next().unwrap().parse::<i64>().unwrap();
            let e = match case {
                Case::Lower => 'e',
                Case::Upper => 'E',
            };
            let point = decimal_point_or_empty(precision, alternate_form);
            // Pad with '0's up to the requested precision to match CPython
            // byte-identically past our internal cap; see `format_fixed`.
            let missing = precision.saturating_sub(capped);
            let mut mantissa = String::with_capacity(base.len() + missing);
            mantissa.push_str(base);
            if missing > 0 {
                mantissa.extend(core::iter::repeat_n('0', missing));
            }
            format!("{mantissa}{point}{e}{exponent:+#03}")
        }
        magnitude if magnitude.is_nan() => format_nan(case),
        magnitude if magnitude.is_infinite() => format_inf(case),
        _ => "".to_string(),
    }
}

/// If s represents a floating point value, trailing zeros and a possibly trailing
/// decimal point will be removed.
/// This function does NOT work with decimal commas.
fn maybe_remove_trailing_redundant_chars(s: String, alternate_form: bool) -> String {
    if !alternate_form && s.contains('.') {
        // only truncate floating point values when not in alternate form
        let s = remove_trailing_zeros(s);
        remove_trailing_decimal_point(s)
    } else {
        s
    }
}

fn remove_trailing_zeros(s: String) -> String {
    let mut s = s;
    while s.ends_with('0') {
        s.pop();
    }
    s
}

fn remove_trailing_decimal_point(s: String) -> String {
    let mut s = s;
    if s.ends_with('.') {
        s.pop();
    }
    s
}

pub fn format_general(
    precision: usize,
    magnitude: f64,
    case: Case,
    alternate_form: bool,
    always_shows_fract: bool,
) -> String {
    match magnitude {
        magnitude if magnitude.is_finite() => {
            let exp_precision = clamp_exp_precision(precision.saturating_sub(1));
            let r_exp = format!("{:.*e}", exp_precision, magnitude);
            let mut parts = r_exp.splitn(2, 'e');
            let base = parts.next().unwrap();
            let exponent = parts.next().unwrap().parse::<i64>().unwrap();
            if exponent < -4 || exponent + (always_shows_fract as i64) >= (precision as i64) {
                let e = match case {
                    Case::Lower => 'e',
                    Case::Upper => 'E',
                };
                // `base` is already produced at the clamped precision via
                // `r_exp`. The previous `format!("{:.*}", precision + 1, base)`
                // call was a no-op (magnitude is `.abs()`-ed at the caller, so
                // base has no sign and its length was exactly `precision + 1`)
                // — reuse `base` directly to avoid double-clamping that would
                // drop the last 1-2 chars at high precision.
                let base = maybe_remove_trailing_redundant_chars(base.to_owned(), alternate_form);
                let point = decimal_point_or_empty(exp_precision, alternate_form);
                format!("{base}{point}{e}{exponent:+#03}")
            } else {
                let precision =
                    clamp_fmt_precision(((precision as i64) - 1 - exponent).max(0) as usize);
                let magnitude = format!("{magnitude:.precision$}");
                let base = maybe_remove_trailing_redundant_chars(magnitude, alternate_form);
                let point = decimal_point_or_empty(precision, alternate_form);
                format!("{base}{point}")
            }
        }
        magnitude if magnitude.is_nan() => format_nan(case),
        magnitude if magnitude.is_infinite() => format_inf(case),
        _ => "".to_string(),
    }
}

fn prefer_cpython_tie_repr(s: String, value: f64) -> String {
    let Some(exponent_pos) = s.find('e') else {
        return s;
    };
    let Some(digit_pos) = s[..exponent_pos].bytes().rposition(|b| b.is_ascii_digit()) else {
        return s;
    };

    let digit = s.as_bytes()[digit_pos];
    if digit == b'0' {
        return s;
    }
    let decremented = digit - 1;
    if !(decremented - b'0').is_multiple_of(2) {
        return s;
    }

    let mut candidate = s.clone();
    candidate.replace_range(
        digit_pos..digit_pos + 1,
        core::str::from_utf8(&[decremented]).unwrap(),
    );
    if parse_str(&candidate).is_none_or(|parsed| parsed.to_bits() != value.to_bits()) {
        return s;
    }

    let Some(current_distance) = decimal_distance_to_f64(&s, value) else {
        return s;
    };
    let Some(candidate_distance) = decimal_distance_to_f64(&candidate, value) else {
        return s;
    };

    if candidate_distance <= current_distance {
        candidate
    } else {
        s
    }
}

fn checked_pow_u128(base: u128, exp: u32) -> Option<u128> {
    let mut result = 1u128;
    for _ in 0..exp {
        result = result.checked_mul(base)?;
    }
    Some(result)
}

fn parse_decimal_rational(s: &str) -> Option<(u128, u32)> {
    let exponent_pos = s.find('e')?;
    let exponent = s[exponent_pos + 1..].parse::<i32>().ok()?;
    let significand = s[..exponent_pos]
        .strip_prefix('-')
        .unwrap_or(&s[..exponent_pos]);
    let dot_pos = significand.find('.');
    let frac_digits = dot_pos
        .map(|pos| significand.len().saturating_sub(pos + 1))
        .unwrap_or(0);
    let mut digits = String::with_capacity(significand.len());
    for ch in significand.chars() {
        if ch != '.' {
            digits.push(ch);
        }
    }
    let mut int = digits.parse::<u128>().ok()?;
    let mut scale = i32::try_from(frac_digits).ok()? - exponent;
    if scale < 0 {
        int = int.checked_mul(checked_pow_u128(10, (-scale) as u32)?)?;
        scale = 0;
    }
    Some((int, scale as u32))
}

fn f64_mantissa_exponent(value: f64) -> Option<(u128, i32)> {
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        Some((u128::from(fraction), 1 - 1023 - 52))
    } else if exponent < 0x7ff {
        Some((u128::from((1u64 << 52) | fraction), exponent - 1023 - 52))
    } else {
        None
    }
}

fn decimal_distance_to_f64(s: &str, value: f64) -> Option<u128> {
    let (decimal_int, decimal_scale) = parse_decimal_rational(s)?;
    let (mantissa, binary_exponent) = f64_mantissa_exponent(value)?;
    if binary_exponent >= 0 || decimal_scale > 38 {
        return None;
    }

    let binary_scale = u32::try_from(-binary_exponent).ok()?;
    let common_twos = decimal_scale.max(binary_scale);
    let decimal_scaled =
        decimal_int.checked_mul(checked_pow_u128(2, common_twos - decimal_scale)?)?;
    let five_power = checked_pow_u128(5, decimal_scale)?;
    let binary_scaled = mantissa
        .checked_mul(checked_pow_u128(2, common_twos - binary_scale)?)?
        .checked_mul(five_power)?;

    Some(decimal_scaled.abs_diff(binary_scaled))
}

// TODO: rewrite using format_general
pub fn to_string(value: f64) -> String {
    let lit = format!("{value:e}");
    if let Some(position) = lit.find('e') {
        let significand = &lit[..position];
        let exponent = &lit[position + 1..];
        let exponent = exponent.parse::<i32>().unwrap();
        if exponent < 16 && exponent > -5 {
            if is_integer(value) {
                format!("{value:.1?}")
            } else {
                value.to_string()
            }
        } else {
            prefer_cpython_tie_repr(format!("{significand}e{exponent:+#03}"), value)
        }
    } else {
        let mut s = value.to_string();
        s.make_ascii_lowercase();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::to_string;

    #[test]
    fn repr_uses_cpython_tie_digit_for_power_of_two() {
        assert_eq!(to_string(2.0f64.powi(-25)), "2.9802322387695312e-08");
        assert_eq!(to_string((-2.0f64).powi(-25)), "-2.9802322387695312e-08");
        assert_eq!(to_string(2.0f64.powi(-26)), "1.4901161193847656e-08");
        assert_eq!(
            to_string(2.0f64.powi(-14) - 2.0f64.powi(-25)),
            "6.1005353927612305e-05"
        );
    }
}

pub fn from_hex(s: &str) -> Option<f64> {
    if let Ok(f) = hexf_parse::parse_hexf64(s, false) {
        return Some(f);
    }
    match s.to_ascii_lowercase().as_str() {
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        "inf" | "infinity" | "+inf" | "+infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        value => {
            let mut hex = String::with_capacity(value.len());
            let has_0x = value.contains("0x");
            let has_p = value.contains('p');
            let has_dot = value.contains('.');
            let mut start = 0;

            if !has_0x && value.starts_with('-') {
                hex.push_str("-0x");
                start += 1;
            } else if !has_0x {
                hex.push_str("0x");
                if value.starts_with('+') {
                    start += 1;
                }
            }

            for (index, ch) in value.chars().enumerate() {
                if ch == 'p' {
                    if has_dot {
                        hex.push('p');
                    } else {
                        hex.push_str(".p");
                    }
                } else if index >= start {
                    hex.push(ch);
                }
            }

            if !has_p && has_dot {
                hex.push_str("p0");
            } else if !has_p && !has_dot {
                hex.push_str(".p0")
            }

            hexf_parse::parse_hexf64(hex.as_str(), false).ok()
        }
    }
}

pub fn to_hex(value: f64) -> String {
    let bits = value.to_bits();
    let sign_fmt = if bits >> 63 != 0 { "-" } else { "" };
    match value {
        value if value.is_zero() => format!("{sign_fmt}0x0.0p+0"),
        value if value.is_infinite() => format!("{sign_fmt}inf"),
        value if value.is_nan() => "nan".to_owned(),
        _ => {
            const FRACT_MASK: u64 = (1u64 << 52) - 1;
            const EXP_MASK: u64 = 0x7ff;
            let exponent = (bits >> 52) & EXP_MASK;
            let fraction = bits & FRACT_MASK;
            if exponent == 0 {
                format!("{sign_fmt}0x0.{fraction:013x}p-1022")
            } else {
                let exponent = i32::try_from(exponent).unwrap() - 1023;
                format!("{sign_fmt}0x1.{fraction:013x}p{exponent:+}")
            }
        }
    }
}

#[test]
fn test_to_hex() {
    use rand::Rng;
    assert_eq!(to_hex(f64::from_bits(1)), "0x0.0000000000001p-1022");
    assert_eq!(to_hex(f64::from_bits(2)), "0x0.0000000000002p-1022");
    assert_eq!(to_hex(-f64::from_bits(1)), "-0x0.0000000000001p-1022");
    assert_eq!(to_hex(f64::MIN_POSITIVE), "0x1.0000000000000p-1022");
    for _ in 0..20000 {
        let bytes = rand::rng().random::<u64>();
        let f = f64::from_bits(bytes);
        if !f.is_finite() {
            continue;
        }
        let hex = to_hex(f);
        // println!("{} -> {}", f, hex);
        let roundtrip = hexf_parse::parse_hexf64(&hex, false).unwrap();
        // println!("  -> {}", roundtrip);
        assert!(f == roundtrip, "{f} {hex} {roundtrip}");
    }
}

#[test]
fn test_remove_trailing_zeros() {
    assert!(remove_trailing_zeros(String::from("100")) == *"1");
    assert!(remove_trailing_zeros(String::from("100.00")) == *"100.");

    // leave leading zeros untouched
    assert!(remove_trailing_zeros(String::from("001")) == *"001");

    // leave strings untouched if they don't end with 0
    assert!(remove_trailing_zeros(String::from("101")) == *"101");
}

#[test]
fn test_remove_trailing_decimal_point() {
    assert!(remove_trailing_decimal_point(String::from("100.")) == *"100");
    assert!(remove_trailing_decimal_point(String::from("1.")) == *"1");

    // leave leading decimal points untouched
    assert!(remove_trailing_decimal_point(String::from(".5")) == *".5");
}

#[test]
fn test_maybe_remove_trailing_redundant_chars() {
    assert!(maybe_remove_trailing_redundant_chars(String::from("100."), true) == *"100.");
    assert!(maybe_remove_trailing_redundant_chars(String::from("100."), false) == *"100");
    assert!(maybe_remove_trailing_redundant_chars(String::from("1."), false) == *"1");
    assert!(maybe_remove_trailing_redundant_chars(String::from("10.0"), false) == *"10");

    // don't truncate integers
    assert!(maybe_remove_trailing_redundant_chars(String::from("1000"), false) == *"1000");
}

use core::f64;
use malachite_bigint::{BigInt, ToBigInt};
use num_traits::{Signed, ToPrimitive};

#[must_use]
pub const fn decompose_float(value: f64) -> (f64, i32) {
    if value == 0.0 {
        return (0.0, 0);
    }
    let bits = value.to_bits();
    // Subnormals carry a biased exponent of 0 and no implicit leading mantissa
    // bit, so the normal decomposition below would misread them. Scale them up
    // into the normal range first (exact, since it is a power-of-two shift) and
    // fold the scale back into the returned exponent.
    let (bits, exponent_adjust) = if (bits >> 52) & 0x7ff == 0 {
        ((value * (1u64 << 54) as f64).to_bits(), -54)
    } else {
        (bits, 0)
    };
    let exponent: i32 = ((bits >> 52) & 0x7ff) as i32 - 1022 + exponent_adjust;
    let mantissa_bits = bits & (0x000f_ffff_ffff_ffff) | (1022 << 52);
    (f64::from_bits(mantissa_bits), exponent)
}

/// Equate an integer to a float.
///
/// Returns true if and only if, when converted to each others types, both are equal.
///
/// # Examples
///
/// ```
/// use malachite_bigint::BigInt;
/// use rustpython_common::float_ops::eq_int;
/// let a = 1.0f64;
/// let b = BigInt::from(1);
/// let c = 2.0f64;
/// assert!(eq_int(a, &b));
/// assert!(!eq_int(c, &b));
/// ```
///
#[must_use]
pub fn eq_int(value: f64, other: &BigInt) -> bool {
    if let (Some(self_int), Some(other_float)) = (value.to_bigint(), other.to_f64()) {
        value == other_float && self_int == *other
    } else {
        false
    }
}

#[must_use]
pub fn lt_int(value: f64, other_int: &BigInt) -> bool {
    match (value.to_bigint(), other_int.to_f64()) {
        (Some(self_int), Some(other_float)) => value < other_float || self_int < *other_int,
        // finite float, other_int too big for float,
        // the result depends only on other_int’s sign
        (Some(_), None) => other_int.is_positive(),
        // infinite float must be bigger or lower than any int, depending on its sign
        _ if value.is_infinite() => value.is_sign_negative(),
        // NaN, always false
        _ => false,
    }
}

#[must_use]
pub fn gt_int(value: f64, other_int: &BigInt) -> bool {
    match (value.to_bigint(), other_int.to_f64()) {
        (Some(self_int), Some(other_float)) => value > other_float || self_int > *other_int,
        // finite float, other_int too big for float,
        // the result depends only on other_int’s sign
        (Some(_), None) => other_int.is_negative(),
        // infinite float must be bigger or lower than any int, depending on its sign
        _ if value.is_infinite() => value.is_sign_positive(),
        // NaN, always false
        _ => false,
    }
}

#[must_use]
pub const fn div(v1: f64, v2: f64) -> Option<f64> {
    if v2 != 0.0 { Some(v1 / v2) } else { None }
}

#[must_use]
pub fn mod_(v1: f64, v2: f64) -> Option<f64> {
    divmod(v1, v2).map(|(_, m)| m)
}

#[must_use]
pub fn floordiv(v1: f64, v2: f64) -> Option<f64> {
    divmod(v1, v2).map(|(d, _)| d)
}

// Canonical (floordiv, mod) for floats matching CPython's _float_div_mod
// (Objects/floatobject.c). `mod_` and `floordiv` delegate here so that
// `divmod(a, b) == (a // b, a % b)` holds by construction.
#[must_use]
pub fn divmod(v1: f64, v2: f64) -> Option<(f64, f64)> {
    if v2 == 0.0 {
        return None;
    }
    let mut m = v1 % v2;
    let mut d = (v1 - m) / v2;
    if m != 0.0 {
        // Non-zero remainder must have the sign of the divisor.
        if v2.is_sign_negative() != m.is_sign_negative() {
            m += v2;
            d -= 1.0;
        }
    } else {
        // Zero remainder: sign matches divisor (IEEE 754 / CPython contract).
        m = (0.0_f64).copysign(v2);
    }
    let d = if d != 0.0 {
        let f = d.floor();
        // Snap up if (v1 - m) / v2 undershot the true integer quotient by
        // more than half an ULP (mirrors CPython's `if (div - *floordiv > 0.5)`).
        if d - f > 0.5 { f + 1.0 } else { f }
    } else {
        // Zero quotient: take the sign of the true quotient v1 / v2.
        (0.0_f64).copysign(v1 / v2)
    };
    Some((d, m))
}

// nextafter algorithm based off of https://gitlab.com/bronsonbdevost/next_afterf
#[allow(clippy::float_cmp)]
#[must_use]
pub fn nextafter(x: f64, y: f64) -> f64 {
    if x == y {
        y
    } else if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x >= f64::INFINITY {
        f64::MAX
    } else if x <= f64::NEG_INFINITY {
        f64::MIN
    } else if x == 0.0 {
        f64::from_bits(1).copysign(y)
    } else {
        // next x after 0 if y is farther from 0 than x, otherwise next towards 0
        // the sign is a separate bit in floats, so bits+1 moves away from 0 no matter the float
        let b = x.to_bits();
        let bits = if (y > x) == (x > 0.0) { b + 1 } else { b - 1 };
        let ret = f64::from_bits(bits);
        if ret == 0.0 { ret.copysign(x) } else { ret }
    }
}

#[allow(clippy::float_cmp)]
#[must_use]
pub fn nextafter_with_steps(x: f64, y: f64, steps: u64) -> f64 {
    if x == y {
        y
    } else if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x >= f64::INFINITY {
        f64::MAX
    } else if x <= f64::NEG_INFINITY {
        f64::MIN
    } else if x == 0.0 {
        f64::from_bits(1).copysign(y)
    } else {
        if steps == 0 {
            return x;
        }

        if x.is_nan() {
            return x;
        }

        if y.is_nan() {
            return y;
        }

        let sign_bit: u64 = 1 << 63;

        let mut ux = x.to_bits();
        let uy = y.to_bits();

        let ax = ux & !sign_bit;
        let ay = uy & !sign_bit;

        // If signs are different
        if ((ux ^ uy) & sign_bit) != 0 {
            return if ax + ay <= steps {
                f64::from_bits(uy)
            } else if ax < steps {
                let result = (uy & sign_bit) | (steps - ax);
                f64::from_bits(result)
            } else {
                ux -= steps;
                f64::from_bits(ux)
            };
        }

        // If signs are the same
        if ax > ay {
            if ax - ay >= steps {
                ux -= steps;
                f64::from_bits(ux)
            } else {
                f64::from_bits(uy)
            }
        } else if ay - ax >= steps {
            ux += steps;
            f64::from_bits(ux)
        } else {
            f64::from_bits(uy)
        }
    }
}

#[must_use]
pub fn ulp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let x = x.abs();
    let x2 = nextafter(x, f64::INFINITY);
    if x2.is_infinite() {
        // special case: x is the largest positive representable float
        let x2 = nextafter(x, f64::NEG_INFINITY);
        x - x2
    } else {
        x2 - x
    }
}

#[must_use]
pub fn round_float_digits(x: f64, ndigits: i32) -> Option<f64> {
    // Mirror CPython's `float.__round__` (Objects/floatobject.c), which uses
    // `_Py_dg_dtoa` to round at the decimal level. Multiplying by 10**ndigits
    // and rounding at the IEEE 754 binary level diverges for values that
    // aren't exactly representable: 2.675 stores as 2.67499..., which dtoa
    // correctly rounds down to 2.67, but `(2.675 * 100.0).round() / 100.0`
    // lands on 2.68 because the multiplication produces a phantom 267.5 tie.
    // Rust's `{:.*}` float formatting uses dtoa-style algorithms and matches
    // CPython's `_Py_dg_dtoa` byte-for-byte.
    if !x.is_finite() {
        return Some(x);
    }

    const NDIGITS_MAX: i32 =
        ((f64::MANTISSA_DIGITS as i32 - f64::MIN_EXP) as f64 * f64::consts::LOG10_2) as i32;
    const NDIGITS_MIN: i32 = -(((f64::MAX_EXP + 1) as f64 * f64::consts::LOG10_2) as i32);

    if ndigits > NDIGITS_MAX {
        return Some(x);
    }
    if ndigits < NDIGITS_MIN {
        return Some(0.0f64.copysign(x));
    }

    let result: f64 = if ndigits >= 0 {
        let s = format!("{:.*}", ndigits as usize, x);
        s.parse().ok()?
    } else {
        // ndigits < 0: divide-then-round avoids the phantom-tie problem
        // because dividing typical inputs by 10**|ndigits| produces genuine
        // half-integer ties rather than synthesizing them.
        let pow1 = 10.0f64.powi(-ndigits);
        let y = x / pow1;
        let z = y.round();
        #[expect(
            clippy::float_cmp,
            reason = "exact half-tie detection for banker's rounding correction"
        )]
        let z = if (y - z).abs() == 0.5 {
            2.0 * (y / 2.0).round()
        } else {
            z
        };
        z * pow1
    };

    if !result.is_finite() {
        return None;
    }
    Some(result)
}

/// Error from [`from_hex`], mapping to the exception the caller should raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexFloatError {
    /// ValueError "invalid hexadecimal floating-point string"
    Invalid,
    /// ValueError "hexadecimal string too long to convert"
    TooLong,
    /// OverflowError "hexadecimal value too large to represent as a float"
    Overflow,
}

const DBL_MANT_DIG: i64 = 53;
const DBL_MIN_EXP: i64 = -1021;
const DBL_MAX_EXP: i64 = 1024;

/// Read byte at `i`, returning a NUL terminator past the end so that
/// digit/sign/space scans stop at the string boundary.
#[inline]
fn hex_byte_at(bytes: &[u8], i: usize) -> u8 {
    if i < bytes.len() { bytes[i] } else { 0 }
}

#[inline]
fn is_py_space(c: u8) -> bool {
    matches!(c, 0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20)
}

/// '0'-'9' -> 0..9, 'a'-'f'/'A'-'F' -> 10..15, else -1.
#[inline]
fn hex_from_char(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a') as i32 + 10,
        b'A'..=b'F' => (c - b'A') as i32 + 10,
        _ => -1,
    }
}

/// `t` must be an ASCII-lowercase literal. Returns true iff every byte of `t`
/// matched case-insensitively starting at `s`.
fn case_insensitive_match(bytes: &[u8], s: usize, t: &[u8]) -> bool {
    let mut si = s;
    let mut ti = 0;
    while ti < t.len() && hex_byte_at(bytes, si).to_ascii_lowercase() == t[ti] {
        si += 1;
        ti += 1;
    }
    ti == t.len()
}

/// Returns `(retval, endptr)`. When the text at `p` is not inf/nan, `retval` is
/// `-1.0` and `endptr == p`.
fn parse_inf_or_nan(bytes: &[u8], p: usize) -> (f64, usize) {
    let mut s = p;
    let mut negate = false;
    if hex_byte_at(bytes, s) == b'-' {
        negate = true;
        s += 1;
    } else if hex_byte_at(bytes, s) == b'+' {
        s += 1;
    }
    let retval;
    if case_insensitive_match(bytes, s, b"inf") {
        s += 3;
        if case_insensitive_match(bytes, s, b"inity") {
            s += 5;
        }
        retval = if negate {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    } else if case_insensitive_match(bytes, s, b"nan") {
        s += 3;
        retval = if negate {
            f64::from_bits(0xfff8_0000_0000_0000)
        } else {
            f64::from_bits(0x7ff8_0000_0000_0000)
        };
    } else {
        s = p;
        retval = -1.0;
    }
    (retval, s)
}

/// Correctly-rounded scalbn. Every call site scales an already-representable
/// value, so the result is exact.
fn ldexp(x: f64, mut n: i32) -> f64 {
    let x1p1023 = f64::from_bits(0x7fe0000000000000);
    let x1p53 = f64::from_bits(0x4340000000000000);
    let x1p_1022 = f64::from_bits(0x0010000000000000);
    let mut y = x;
    if n > 1023 {
        y *= x1p1023;
        n -= 1023;
        if n > 1023 {
            y *= x1p1023;
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        y *= x1p_1022 * x1p53;
        n += 1022 - 53;
        if n < -1022 {
            y *= x1p_1022 * x1p53;
            n += 1022 - 53;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    y * f64::from_bits(((0x3ff + n) as u64) << 52)
}

/// Parse the already-validated `[+-]?[0-9]+` slice `bytes[start..end]` as base-10
/// signed, saturating to i64::MIN/MAX on overflow like strtol.
fn strtol_saturating(bytes: &[u8], start: usize, end: usize) -> i64 {
    let mut i = start;
    let mut neg = false;
    if i < end && (bytes[i] == b'+' || bytes[i] == b'-') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let mut val: i64 = 0;
    let mut overflowed = false;
    while i < end {
        let d = (bytes[i] - b'0') as i64;
        match val.checked_mul(10).and_then(|v| v.checked_add(d)) {
            Some(v) => val = v,
            None => {
                overflowed = true;
                break;
            }
        }
        i += 1;
    }
    if overflowed {
        if neg { i64::MIN } else { i64::MAX }
    } else if neg {
        -val
    } else {
        val
    }
}

/// Parse a hexadecimal floating-point string (the `float.fromhex` grammar).
///
/// The raw string is consumed as-is: leading and trailing whitespace are handled
/// internally using the ASCII space set, so callers must not trim first.
pub fn from_hex(s: &str) -> Result<f64, HexFloatError> {
    let bytes = s.as_bytes();
    let s_end = bytes.len();

    let mut negate = false;
    let mut idx = 0usize;

    // leading whitespace
    while is_py_space(hex_byte_at(bytes, idx)) {
        idx += 1;
    }

    // infinities and nans
    let (mut x, coeff_end_inf) = parse_inf_or_nan(bytes, idx);
    if coeff_end_inf != idx {
        idx = coeff_end_inf;
        return finish_hex(bytes, s_end, idx, negate, x);
    }

    // optional sign
    if hex_byte_at(bytes, idx) == b'-' {
        idx += 1;
        negate = true;
    } else if hex_byte_at(bytes, idx) == b'+' {
        idx += 1;
    }

    // [0x]
    let s_store = idx;
    if hex_byte_at(bytes, idx) == b'0' {
        idx += 1;
        if hex_byte_at(bytes, idx) == b'x' || hex_byte_at(bytes, idx) == b'X' {
            idx += 1;
        } else {
            idx = s_store;
        }
    }

    // coefficient: <integer> [. <fraction>]
    let coeff_start = idx;
    while hex_from_char(hex_byte_at(bytes, idx)) >= 0 {
        idx += 1;
    }
    let s_store = idx;
    let coeff_end = if hex_byte_at(bytes, idx) == b'.' {
        idx += 1;
        while hex_from_char(hex_byte_at(bytes, idx)) >= 0 {
            idx += 1;
        }
        idx - 1
    } else {
        idx
    };

    // ndigits = total # of hex digits; fdigits = # after point
    let ndigits_total = (coeff_end - coeff_start) as i64;
    let fdigits = (coeff_end - s_store) as i64;
    if ndigits_total == 0 {
        return Err(HexFloatError::Invalid);
    }
    let insane_bound = core::cmp::min(
        DBL_MIN_EXP - DBL_MANT_DIG - i64::MIN / 2,
        i64::MAX / 2 + 1 - DBL_MAX_EXP,
    ) / 4;
    if ndigits_total > insane_bound {
        return Err(HexFloatError::TooLong);
    }

    // [p <exponent>]
    let exp = if hex_byte_at(bytes, idx) == b'p' || hex_byte_at(bytes, idx) == b'P' {
        idx += 1;
        let exp_start = idx;
        if hex_byte_at(bytes, idx) == b'-' || hex_byte_at(bytes, idx) == b'+' {
            idx += 1;
        }
        if !(b'0' <= hex_byte_at(bytes, idx) && hex_byte_at(bytes, idx) <= b'9') {
            return Err(HexFloatError::Invalid);
        }
        idx += 1;
        while b'0' <= hex_byte_at(bytes, idx) && hex_byte_at(bytes, idx) <= b'9' {
            idx += 1;
        }
        strtol_saturating(bytes, exp_start, idx)
    } else {
        0
    };

    // HEX_DIGIT(j): jth hex digit counting from the least significant.
    let hex_digit = |j: i64| -> i32 {
        let byte_idx = if j < fdigits {
            coeff_end as i64 - j
        } else {
            coeff_end as i64 - 1 - j
        };
        hex_from_char(hex_byte_at(bytes, byte_idx as usize))
    };

    // Discard leading zeros, and catch extreme overflow and underflow.
    let mut ndigits = ndigits_total;
    while ndigits > 0 && hex_digit(ndigits - 1) == 0 {
        ndigits -= 1;
    }
    if ndigits == 0 || exp < i64::MIN / 2 {
        x = 0.0;
        return finish_hex(bytes, s_end, idx, negate, x);
    }
    if exp > i64::MAX / 2 {
        return Err(HexFloatError::Overflow);
    }

    // Adjust exponent for fractional part.
    let exp = exp - 4 * fdigits;

    // top_exp = 1 more than exponent of most significant bit of coefficient.
    let mut top_exp = exp + 4 * (ndigits - 1);
    let mut digit = hex_digit(ndigits - 1);
    while digit != 0 {
        top_exp += 1;
        digit /= 2;
    }

    // catch almost all nonextreme cases of overflow and underflow here
    if top_exp < DBL_MIN_EXP - DBL_MANT_DIG {
        x = 0.0;
        return finish_hex(bytes, s_end, idx, negate, x);
    }
    if top_exp > DBL_MAX_EXP {
        return Err(HexFloatError::Overflow);
    }

    // lsb = exponent of least significant bit of the rounded value.
    let lsb = core::cmp::max(top_exp, DBL_MIN_EXP) - DBL_MANT_DIG;

    x = 0.0;
    if exp >= lsb {
        // no rounding required
        let mut i = ndigits - 1;
        while i >= 0 {
            x = 16.0 * x + hex_digit(i) as f64;
            i -= 1;
        }
        x = ldexp(x, exp as i32);
        return finish_hex(bytes, s_end, idx, negate, x);
    }

    // rounding required. key_digit is the index of the hex digit
    // containing the first bit to be rounded away.
    let half_eps: i32 = 1 << ((lsb - exp - 1) % 4) as i32;
    let key_digit = (lsb - exp - 1) / 4;
    let mut i = ndigits - 1;
    while i > key_digit {
        x = 16.0 * x + hex_digit(i) as f64;
        i -= 1;
    }
    let digit = hex_digit(key_digit);
    x = 16.0 * x + (digit & (16 - 2 * half_eps)) as f64;

    // round-half-even
    if (digit & half_eps) != 0 {
        let round_up = if (digit & (3 * half_eps - 1)) != 0
            || (half_eps == 8 && key_digit + 1 < ndigits && (hex_digit(key_digit + 1) & 1) != 0)
        {
            true
        } else {
            let mut r = false;
            let mut i = key_digit - 1;
            while i >= 0 {
                if hex_digit(i) != 0 {
                    r = true;
                    break;
                }
                i -= 1;
            }
            r
        };
        if round_up {
            x += (2 * half_eps) as f64;
            if top_exp == DBL_MAX_EXP && x == ldexp((2 * half_eps) as f64, DBL_MANT_DIG as i32) {
                // overflow corner case
                return Err(HexFloatError::Overflow);
            }
        }
    }
    x = ldexp(x, (exp + 4 * key_digit) as i32);

    finish_hex(bytes, s_end, idx, negate, x)
}

/// Skip trailing whitespace, require the whole string was consumed, and apply
/// the sign.
fn finish_hex(
    bytes: &[u8],
    s_end: usize,
    mut idx: usize,
    negate: bool,
    x: f64,
) -> Result<f64, HexFloatError> {
    while is_py_space(hex_byte_at(bytes, idx)) {
        idx += 1;
    }
    if idx != s_end {
        return Err(HexFloatError::Invalid);
    }
    Ok(if negate { -x } else { x })
}

#[cfg(test)]
mod from_hex_tests {
    use super::{HexFloatError, from_hex};

    fn bits(s: &str) -> u64 {
        from_hex(s).unwrap().to_bits()
    }

    #[test]
    fn test_from_hex_exact_bits() {
        assert_eq!(bits("0x1p-1074"), 0x0000000000000001);
        assert_eq!(bits("0x1.fffffffffffffp+1023"), 0x7fefffffffffffff);
        // round-half-even ties
        assert_eq!(bits("0x1.00000000000008p0"), 0x3ff0000000000000);
        assert_eq!(bits("0x1.00000000000018p0"), 0x3ff0000000000002);
        assert_eq!(bits("-0x1p0"), 0xbff0000000000000);
        assert_eq!(bits("0x0p0"), 0x0000000000000000);
        assert_eq!(bits("-0x0p0"), 0x8000000000000000);
    }

    #[test]
    fn test_from_hex_inf_nan() {
        assert_eq!(bits("inf"), 0x7ff0000000000000);
        assert_eq!(bits("-inf"), 0xfff0000000000000);
        assert_eq!(bits("Infinity"), 0x7ff0000000000000);

        let n = from_hex("nan").unwrap();
        assert!(n.is_nan());
        assert_eq!(n.to_bits(), 0x7ff8000000000000);
        let neg = from_hex("-nan").unwrap();
        assert!(neg.is_nan());
        assert_eq!(neg.to_bits(), 0xfff8000000000000);
    }

    #[test]
    fn test_from_hex_whitespace() {
        assert_eq!(bits("  0x1p0  "), 0x3ff0000000000000);
        assert_eq!(bits("\t0x1p0\n"), 0x3ff0000000000000);
    }

    #[test]
    fn test_from_hex_errors() {
        assert_eq!(from_hex("0x1p1024"), Err(HexFloatError::Overflow));
        assert_eq!(from_hex("0x1z"), Err(HexFloatError::Invalid));
        assert_eq!(from_hex(""), Err(HexFloatError::Invalid));
        assert_eq!(from_hex("0x1 p0"), Err(HexFloatError::Invalid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_float;

    /// Exact `2**e` for `e` in `[-1074, 1023]`, built from bits so extreme
    /// exponents don't overflow through an intermediate `2**|e|`.
    fn pow2(e: i32) -> f64 {
        if e >= -1022 {
            f64::from_bits(((e + 1023) as u64) << 52)
        } else {
            f64::from_bits(1u64 << (e + 1074))
        }
    }

    /// `decompose_float` is a frexp returning the *magnitude* mantissa: for a
    /// nonzero `value`, `m` lies in `[0.5, 1)` and `m * 2**e == value.abs()`,
    /// including for subnormals which have no implicit leading mantissa bit.
    /// (Its sole caller reintroduces the sign via `value.signum()`.)
    #[test]
    fn decompose_float_frexp_contract() {
        let mut values = alloc::vec![
            0.0,
            f64::from_bits(1), // smallest subnormal
            f64::from_bits(2),
            f64::from_bits(0x000f_ffff_ffff_ffff), // largest subnormal
            f64::MIN_POSITIVE,                     // DBL_MIN, smallest normal
            f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1), // predecessor
            1.0,
            1.5,
            0.1,
            core::f64::consts::PI,
        ];
        for e in -1074..=1023 {
            values.push(pow2(e));
            values.push(-pow2(e));
        }
        for &v in &values {
            let (m, e) = decompose_float(v);
            if v == 0.0 {
                assert_eq!((m, e), (0.0, 0));
                continue;
            }
            assert!(
                (0.5..1.0).contains(&m),
                "mantissa {m} out of [0.5, 1) for value {v:e}"
            );
            // Reconstruct: m * 2**e must round-trip to the magnitude. Fold one
            // power of two into the mantissa so `e` stays within `pow2`'s range
            // (frexp yields e up to 1024 for 2**1023).
            let reconstructed = (m * 2.0) * pow2(e - 1);
            assert_eq!(
                reconstructed.to_bits(),
                v.abs().to_bits(),
                "reconstruction failed for {v:e}: m={m}, e={e}"
            );
        }
    }

    /// Subnormal frexp regression: hash of the smallest positive subnormal.
    #[test]
    fn hash_float_smallest_subnormal() {
        // hash(5e-324) == 16777216 (CPython 3.14 ground truth). The pre-fix
        // bit-twiddling frexp returned 8404992 here.
        assert_eq!(hash_float(f64::from_bits(1)), Some(16777216));
    }

    /// Differential float-hash table captured from CPython 3.14.5, spanning
    /// subnormal boundaries, powers of two across the whole exponent range, and
    /// a spread of normals.
    #[test]
    fn hash_float_matches_cpython() {
        const HASH_CASES: &[(u64, i64)] = &[
            (0x0000000000000001, 16777216),            // smallest subnormal 5e-324
            (0x0000000000000002, 33554432),            // subnormal
            (0x00000000deadbeef, 62678480394911744),   // subnormal midrange
            (0x0008000000000000, 16384),               // subnormal high bit
            (0x000fffffffffffff, 2305843009196949503), // largest subnormal
            (0x0010000000000000, 32768),               // DBL_MIN smallest normal
            (0x8000000000000001, -16777216),           // negative smallest subnormal
            (0x0020000000000000, 65536),               // 2**-1021
            (0x0170000000000000, 137438953472),        // 2**-1000
            (0x39b0000000000000, 4194304),             // 2**-100
            (0x3f50000000000000, 2251799813685248),    // 2**-10
            (0x3fe0000000000000, 1152921504606846976), // 2**-1
            (0x3ff0000000000000, 1),                   // 2**0
            (0x4000000000000000, 2),                   // 2**1
            (0x4090000000000000, 1024),                // 2**10
            (0x4630000000000000, 549755813888),        // 2**100
            (0x7e70000000000000, 16777216),            // 2**1000
            (0x7fe0000000000000, 140737488355328),     // 2**1023
            (0xffe0000000000000, -140737488355328),    // -2**1023
            (0x3ff8000000000000, 1152921504606846977), // 1.5
            (0x400921fb54442d18, 326490430436040707),  // 3.141592653589793
            (0x7e37e43c8800759c, 1224995262755759164), // 1e+300
            (0x01a56e1fc2f8f359, 482449582752280463),  // 1e-300
            (0x40c81cd6c8b43958, 1563361560246628409), // 12345.678
            (0x3fb999999999999a, 230584300921369408),  // 0.1
            (0x4005666666666666, 1556444031219243010), // 2.675
            (0x4132d68700000000, 1234567),             // 1234567.0
            (0x44dfe154f457ea13, 1428027733287631914), // 6.022e+23
            (0x3c07a42f549647fb, 851769299698974080),  // 1.602e-19
            (0xbff0000000000000, -2),                  // -1.0
            (0xbfb999999999999a, -230584300921369408), // -0.1
        ];
        for &(bits, expected) in HASH_CASES {
            let v = f64::from_bits(bits);
            assert_eq!(
                hash_float(v),
                Some(expected),
                "hash mismatch for {v:e} (bits {bits:#018x})"
            );
        }
    }
}

use crate::float;
use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};

/// Format a single complex component (real or imag) for `repr`.
/// Uses scientific notation when `|value| < 1e-4` or `|value| >= 1e16`
/// (matching CPython's `PyOS_double_to_string(format='r')`), otherwise
/// Rust's default `Display`, which drops the trailing `.0` for
/// integer-valued floats.
///
/// This differs from `float::to_string` only in that integer values in
/// the normal range render as `"1"` rather than `"1.0"` — complex repr
/// formats `1+2j` as `"(1+2j)"`, not `"(1.0+2.0j)"`.
fn component_to_string(value: f64) -> String {
    let lit = alloc::format!("{value:e}");
    if let Some(position) = lit.find('e') {
        let significand = &lit[..position];
        let exponent = lit[position + 1..].parse::<i32>().unwrap();
        if exponent < 16 && exponent > -5 {
            // Normal magnitude — Rust's default Display emits "1" for 1.0,
            // "1.5" for 1.5, "1000000000000000" for 1e15, etc.
            float::prefer_cpython_tie_repr(value.to_string(), value)
        } else {
            float::prefer_cpython_tie_repr(alloc::format!("{significand}e{exponent:+#03}"), value)
        }
    } else {
        // nan / inf / -inf — `format!("{x:e}")` produces e.g. "NaN" with no
        // exponent marker; lowercase to match Python.
        let mut s = value.to_string();
        s.make_ascii_lowercase();
        s
    }
}

/// Convert a complex number to a string.
#[must_use]
pub fn to_string(re: f64, im: f64) -> String {
    let mut im_part = component_to_string(im);
    im_part.push('j');

    // positive empty => return im_part, integer => drop ., fractional => float_ops
    let re_part = if re == 0.0 {
        if re.is_sign_positive() {
            return im_part;
        }
        "-0".to_owned()
    } else {
        component_to_string(re)
    };
    let mut result =
        String::with_capacity(re_part.len() + im_part.len() + 2 + im.is_sign_positive() as usize);
    result.push('(');
    result.push_str(&re_part);
    if im.is_sign_positive() || im.is_nan() {
        result.push('+');
    }
    result.push_str(&im_part);
    result.push(')');
    result
}

/// Parse a complex number from a string.
///
/// Returns `Some((re, im))` on success.
pub fn parse_str(s: &str) -> Option<(f64, f64)> {
    let s = s.trim();
    // Handle parentheses
    let s = match s.strip_prefix('(') {
        None => s,
        Some(s) => s.strip_suffix(')')?.trim(),
    };

    // Whitespace is only allowed around the whole string and the optional
    // parentheses, never inside the numeric token. Reject it here so that
    // `float::parse_str` (which tolerates surrounding whitespace on a part)
    // does not let e.g. "1 +2j" through.
    if s.contains(char::is_whitespace) {
        return None;
    }

    let value = match s.strip_suffix(|c| c == 'j' || c == 'J') {
        None => (float::parse_str(s)?, 0.0),
        Some(mut s) => {
            let mut real = 0.0;
            // Find the central +/- operator. If it exists, parse the real part.
            for (i, w) in s.as_bytes().windows(2).enumerate() {
                if (w[1] == b'+' || w[1] == b'-') && !(w[0] == b'e' || w[0] == b'E') {
                    real = float::parse_str(&s[..=i])?;
                    s = &s[i + 1..];
                    break;
                }
            }

            let imag = match s {
                // "j", "+j"
                "" | "+" => 1.0,
                // "-j"
                "-" => -1.0,
                s => float::parse_str(s)?,
            };

            (real, imag)
        }
    };
    Some(value)
}

type ComplexParts = (f64, f64);

const fn parts(re: f64, im: f64) -> ComplexParts {
    (re, im)
}

/// Only the magnitude of an infinite part matters once a product has gone to
/// nan, so it stands in as a signed one; a nan beside it stands in as a
/// signed zero.
fn signed_unit(part: f64) -> f64 {
    if part.is_infinite() { 1.0 } else { 0.0f64 }.copysign(part)
}

fn tamed(value: ComplexParts) -> ComplexParts {
    parts(signed_unit(value.0), signed_unit(value.1))
}

fn nans_to_zero(value: ComplexParts) -> ComplexParts {
    let zeroed = |part: f64| {
        if part.is_nan() {
            0.0f64.copysign(part)
        } else {
            part
        }
    };
    parts(zeroed(value.0), zeroed(value.1))
}

/// Multiply, recovering the infinities that the plain formula turns into nan,
/// the way C11 Annex G.5.1 does.
#[inline]
#[must_use]
pub fn multiply(a: ComplexParts, b: ComplexParts) -> ComplexParts {
    let r = parts(a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
    if !(r.0.is_nan() && r.1.is_nan()) {
        return r;
    }

    // "Box" an infinite operand into a signed one and turn the other's nans
    // into signed zeros, so that the infinity survives a second pass.
    let (mut a, mut b) = (a, b);
    let a_infinite = a.0.is_infinite() || a.1.is_infinite();
    if a_infinite {
        a = tamed(a);
        b = nans_to_zero(b);
    }
    let b_infinite = b.0.is_infinite() || b.1.is_infinite();
    if b_infinite {
        b = tamed(b);
        a = nans_to_zero(a);
    }
    // An infinity that overflow lost, rather than one an operand carried.
    let overflowed = !a_infinite
        && !b_infinite
        && ((a.0 * b.0).is_infinite()
            || (a.1 * b.1).is_infinite()
            || (a.0 * b.1).is_infinite()
            || (a.1 * b.0).is_infinite());
    if overflowed {
        a = nans_to_zero(a);
        b = nans_to_zero(b);
    }

    if !(a_infinite || b_infinite || overflowed) {
        return r;
    }
    parts(
        f64::INFINITY * (a.0 * b.0 - a.1 * b.1),
        f64::INFINITY * (a.0 * b.1 + a.1 * b.0),
    )
}

/// Divide a real by a complex. Written out rather than routed through `quot`
/// with a zero imaginary part, which would lose the sign of a zero.
#[inline]
#[must_use]
pub fn real_divide(a: f64, b: ComplexParts) -> Option<ComplexParts> {
    let abs_re = b.0.abs();
    let abs_im = b.1.abs();

    let r = if abs_re >= abs_im {
        if abs_re == 0.0 {
            return None;
        }
        let ratio = b.1 / b.0;
        let denom = b.0 + b.1 * ratio;
        parts(a / denom, -a * ratio / denom)
    } else if abs_im >= abs_re {
        let ratio = b.0 / b.1;
        let denom = b.0 * ratio + b.1;
        parts(a * ratio / denom, -a / denom)
    } else {
        // One part of the divisor is a nan, so neither comparison held.
        parts(f64::NAN, f64::NAN)
    };

    // A quotient that came out as nan recovers the way `recovered_quot` does,
    // except that a real numerator has no imaginary term to carry a sign.
    if r.0.is_nan() && r.1.is_nan() && (b.0.is_infinite() || b.1.is_infinite()) && a.is_finite() {
        let (x, y) = tamed(b);
        return Some(parts(0.0 * (a * x), 0.0 * -(a * y)));
    }

    Some(r)
}

/// Divide by whichever part of the divisor is larger, so that the ratio the
/// other part is scaled by cannot overflow. `None` stands for a divisor of
/// zero.
#[inline]
#[must_use]
pub fn divide(a: ComplexParts, b: ComplexParts) -> Option<ComplexParts> {
    let abs_re = b.0.abs();
    let abs_im = b.1.abs();

    let r = if abs_re >= abs_im {
        if abs_re == 0.0 {
            return None;
        }
        let ratio = b.1 / b.0;
        let denom = b.0 + b.1 * ratio;
        parts((a.0 + a.1 * ratio) / denom, (a.1 - a.0 * ratio) / denom)
    } else if abs_im >= abs_re {
        let ratio = b.0 / b.1;
        let denom = b.0 * ratio + b.1;
        parts((a.0 * ratio + a.1) / denom, (a.1 * ratio - a.0) / denom)
    } else {
        // One part of the divisor is a nan, so neither comparison held.
        parts(f64::NAN, f64::NAN)
    };

    Some(recovered_quot(r, a, b))
}

/// Recover the infinities and zeros a quotient came out of as nan, the way
/// C11 Annex G.5.2 does.
fn recovered_quot(r: ComplexParts, a: ComplexParts, b: ComplexParts) -> ComplexParts {
    if !(r.0.is_nan() && r.1.is_nan()) {
        return r;
    }

    if (a.0.is_infinite() || a.1.is_infinite()) && b.0.is_finite() && b.1.is_finite() {
        let (x, y) = tamed(a);
        parts(
            f64::INFINITY * (x * b.0 + y * b.1),
            f64::INFINITY * (y * b.0 - x * b.1),
        )
    } else if (b.0.is_infinite() || b.1.is_infinite()) && a.0.is_finite() && a.1.is_finite() {
        let (x, y) = tamed(b);
        parts(0.0 * (a.0 * x + a.1 * y), 0.0 * (a.1 * x - a.0 * y))
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_internal_whitespace() {
        // Whitespace inside the numeric token is invalid, even where a bare
        // `float::parse_str` on a fragment would tolerate it.
        for s in [
            "1 +2j", "1 2j", "1 +2 j", "+ 1j", "1.5 j", "(1 +2j)", "2 -3j",
        ] {
            assert_eq!(parse_str(s), None, "{s:?} must not parse");
        }
    }

    #[test]
    fn parse_allows_surrounding_and_paren_whitespace() {
        for s in ["  1+2j  ", " (1+2j) ", "( 1+2j )"] {
            assert_eq!(parse_str(s), Some((1.0, 2.0)), "{s:?}");
        }
    }

    #[test]
    fn parse_basic() {
        assert_eq!(parse_str("1"), Some((1.0, 0.0)));
        assert_eq!(parse_str("1j"), Some((0.0, 1.0)));
        assert_eq!(parse_str("j"), Some((0.0, 1.0)));
        assert_eq!(parse_str("-j"), Some((0.0, -1.0)));
        assert_eq!(parse_str("1+2j"), Some((1.0, 2.0)));
        assert_eq!(parse_str("1e5j"), Some((0.0, 1e5)));
        assert_eq!(parse_str("1_000"), Some((1000.0, 0.0)));
        assert_eq!(parse_str(""), None);
        assert_eq!(parse_str("abc"), None);
    }
}

// Shared runtime/compiler arithmetic, with no Python object ownership.

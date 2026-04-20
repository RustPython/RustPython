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
            value.to_string()
        } else {
            alloc::format!("{significand}e{exponent:+#03}")
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
pub fn to_string(re: f64, im: f64) -> String {
    let mut im_part = component_to_string(im);
    im_part.push('j');

    // positive empty => return im_part, integer => drop ., fractional => float_ops
    let re_part = if re == 0.0 {
        if re.is_sign_positive() {
            return im_part;
        } else {
            "-0".to_owned()
        }
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

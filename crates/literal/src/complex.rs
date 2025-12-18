use crate::float;

/// Convert a complex number to a string.
pub fn to_string(re: f64, im: f64) -> String {
    // integer => drop ., fractional => float_ops
    let mut im_part = if im.fract() == 0.0 {
        im.to_string()
    } else {
        float::to_string(im)
    };
    im_part.push('j');

    // positive empty => return im_part, integer => drop ., fractional => float_ops
    let re_part = if re == 0.0 {
        if re.is_sign_positive() {
            return im_part;
        } else {
            "-0".to_owned()
        }
    } else if re.fract() == 0.0 {
        re.to_string()
    } else {
        float::to_string(re)
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

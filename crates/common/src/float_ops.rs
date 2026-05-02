use core::f64;
use malachite_bigint::{BigInt, ToBigInt};
use num_traits::{Signed, ToPrimitive};

pub const fn decompose_float(value: f64) -> (f64, i32) {
    if 0.0 == value {
        (0.0, 0i32)
    } else {
        let bits = value.to_bits();
        let exponent: i32 = ((bits >> 52) & 0x7ff) as i32 - 1022;
        let mantissa_bits = bits & (0x000f_ffff_ffff_ffff) | (1022 << 52);
        (f64::from_bits(mantissa_bits), exponent)
    }
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
pub fn eq_int(value: f64, other: &BigInt) -> bool {
    if let (Some(self_int), Some(other_float)) = (value.to_bigint(), other.to_f64()) {
        value == other_float && self_int == *other
    } else {
        false
    }
}

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

pub const fn div(v1: f64, v2: f64) -> Option<f64> {
    if v2 != 0.0 { Some(v1 / v2) } else { None }
}

pub fn mod_(v1: f64, v2: f64) -> Option<f64> {
    divmod(v1, v2).map(|(_, m)| m)
}

pub fn floordiv(v1: f64, v2: f64) -> Option<f64> {
    divmod(v1, v2).map(|(d, _)| d)
}

// Canonical (floordiv, mod) for floats matching CPython's _float_div_mod
// (Objects/floatobject.c). `mod_` and `floordiv` delegate here so that
// `divmod(a, b) == (a // b, a % b)` holds by construction.
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
        #[allow(clippy::float_cmp)]
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

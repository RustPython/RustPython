use malachite_bigint::{BigInt, ToBigInt};
use num_traits::{Float, Signed, ToPrimitive, Zero};
use std::f64;

pub fn ufrexp(value: f64) -> (f64, i32) {
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

pub fn div(v1: f64, v2: f64) -> Option<f64> {
    if v2 != 0.0 { Some(v1 / v2) } else { None }
}

pub fn mod_(v1: f64, v2: f64) -> Option<f64> {
    if v2 != 0.0 {
        let val = v1 % v2;
        match (v1.signum() as i32, v2.signum() as i32) {
            (1, 1) | (-1, -1) => Some(val),
            _ if (v1 == 0.0) || (v1.abs() == v2.abs()) => Some(val.copysign(v2)),
            _ => Some((val + v2).copysign(v2)),
        }
    } else {
        None
    }
}

pub fn floordiv(v1: f64, v2: f64) -> Option<f64> {
    if v2 != 0.0 {
        Some((v1 / v2).floor())
    } else {
        None
    }
}

pub fn divmod(v1: f64, v2: f64) -> Option<(f64, f64)> {
    if v2 != 0.0 {
        let mut m = v1 % v2;
        let mut d = (v1 - m) / v2;
        if v2.is_sign_negative() != m.is_sign_negative() {
            m += v2;
            d -= 1.0;
        }
        Some((d, m))
    } else {
        None
    }
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
    let float = if ndigits.is_zero() {
        let fract = x.fract();
        if (fract.abs() - 0.5).abs() < f64::EPSILON {
            if x.trunc() % 2.0 == 0.0 {
                x - fract
            } else {
                x + fract
            }
        } else {
            x.round()
        }
    } else {
        const NDIGITS_MAX: i32 =
            ((f64::MANTISSA_DIGITS as i32 - f64::MIN_EXP) as f64 * f64::consts::LOG10_2) as i32;
        const NDIGITS_MIN: i32 = -(((f64::MAX_EXP + 1) as f64 * f64::consts::LOG10_2) as i32);
        if ndigits > NDIGITS_MAX {
            x
        } else if ndigits < NDIGITS_MIN {
            0.0f64.copysign(x)
        } else {
            let (y, pow1, pow2) = if ndigits >= 0 {
                // according to cpython: pow1 and pow2 are each safe from overflow, but
                //                       pow1*pow2 ~= pow(10.0, ndigits) might overflow
                let (pow1, pow2) = if ndigits > 22 {
                    (10.0.powf((ndigits - 22) as f64), 1e22)
                } else {
                    (10.0.powf(ndigits as f64), 1.0)
                };
                let y = (x * pow1) * pow2;
                if !y.is_finite() {
                    return Some(x);
                }
                (y, pow1, Some(pow2))
            } else {
                let pow1 = 10.0.powf((-ndigits) as f64);
                (x / pow1, pow1, None)
            };
            let z = y.round();
            #[allow(clippy::float_cmp)]
            let z = if (y - z).abs() == 0.5 {
                2.0 * (y / 2.0).round()
            } else {
                z
            };
            let z = if let Some(pow2) = pow2 {
                // ndigits >= 0
                (z / pow2) / pow1
            } else {
                z * pow1
            };

            if !z.is_finite() {
                // overflow
                return None;
            }

            z
        }
    };
    Some(float)
}

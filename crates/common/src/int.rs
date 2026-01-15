use malachite_base::{num::conversion::traits::RoundingInto, rounding_modes::RoundingMode};
use malachite_bigint::{BigInt, BigUint, Sign};
use malachite_q::Rational;
use num_traits::{One, ToPrimitive, Zero};

pub fn true_div(numerator: &BigInt, denominator: &BigInt) -> f64 {
    let rational = Rational::from_integers_ref(numerator.into(), denominator.into());
    match rational.rounding_into(RoundingMode::Nearest) {
        // returned value is $t::MAX but still less than the original
        (val, core::cmp::Ordering::Less) if val == f64::MAX => f64::INFINITY,
        // returned value is $t::MIN but still greater than the original
        (val, core::cmp::Ordering::Greater) if val == f64::MIN => f64::NEG_INFINITY,
        (val, _) => val,
    }
}

pub fn float_to_ratio(value: f64) -> Option<(BigInt, BigInt)> {
    let sign = match core::cmp::PartialOrd::partial_cmp(&value, &0.0)? {
        core::cmp::Ordering::Less => Sign::Minus,
        core::cmp::Ordering::Equal => return Some((BigInt::zero(), BigInt::one())),
        core::cmp::Ordering::Greater => Sign::Plus,
    };
    Rational::try_from(value).ok().map(|x| {
        let (numer, denom) = x.into_numerator_and_denominator();
        (
            BigInt::from_biguint(sign, numer.into()),
            BigUint::from(denom).into(),
        )
    })
}

#[derive(Debug, Eq, PartialEq)]
pub enum BytesToIntError {
    InvalidLiteral { base: u32 },
    InvalidBase,
    DigitLimit { got: usize, limit: usize },
}

// https://github.com/python/cpython/blob/4e665351082c50018fb31d80db25b4693057393e/Objects/longobject.c#L2977
// https://github.com/python/cpython/blob/4e665351082c50018fb31d80db25b4693057393e/Objects/longobject.c#L2884
pub fn bytes_to_int(
    buf: &[u8],
    mut base: u32,
    digit_limit: usize,
) -> Result<BigInt, BytesToIntError> {
    if base != 0 && !(2..=36).contains(&base) {
        return Err(BytesToIntError::InvalidBase);
    }

    let mut buf = buf.trim_ascii();

    // split sign
    let sign = match buf.first() {
        Some(b'+') => Some(Sign::Plus),
        Some(b'-') => Some(Sign::Minus),
        None => return Err(BytesToIntError::InvalidLiteral { base }),
        _ => None,
    };

    if sign.is_some() {
        buf = &buf[1..];
    }

    let mut error_if_nonzero = false;
    if base == 0 {
        match (buf.first(), buf.get(1)) {
            (Some(v), _) if *v != b'0' => base = 10,
            (_, Some(b'x' | b'X')) => base = 16,
            (_, Some(b'o' | b'O')) => base = 8,
            (_, Some(b'b' | b'B')) => base = 2,
            (_, _) => {
                // "old" (C-style) octal literal, now invalid. it might still be zero though
                base = 10;
                error_if_nonzero = true;
            }
        }
    }

    if error_if_nonzero {
        if let [_first, others @ .., last] = buf {
            let is_zero = *last == b'0' && others.iter().all(|&c| c == b'0' || c == b'_');
            if !is_zero {
                return Err(BytesToIntError::InvalidLiteral { base });
            }
        }
        return Ok(BigInt::zero());
    }

    if buf.first().is_some_and(|&v| v == b'0')
        && buf.get(1).is_some_and(|&v| {
            (base == 16 && (v == b'x' || v == b'X'))
                || (base == 8 && (v == b'o' || v == b'O'))
                || (base == 2 && (v == b'b' || v == b'B'))
        })
    {
        buf = &buf[2..];

        // One underscore allowed here
        if buf.first().is_some_and(|&v| v == b'_') {
            buf = &buf[1..];
        }
    }

    // Reject empty strings
    let mut prev = *buf
        .first()
        .ok_or(BytesToIntError::InvalidLiteral { base })?;

    // Leading underscore not allowed
    if prev == b'_' || !prev.is_ascii_alphanumeric() {
        return Err(BytesToIntError::InvalidLiteral { base });
    }

    // Verify all characters are digits and underscores
    let mut digits = 1;
    for &cur in buf.iter().skip(1) {
        if cur == b'_' {
            // Double underscore not allowed
            if prev == b'_' {
                return Err(BytesToIntError::InvalidLiteral { base });
            }
        } else if cur.is_ascii_alphanumeric() {
            digits += 1;
        } else {
            return Err(BytesToIntError::InvalidLiteral { base });
        }

        prev = cur;
    }

    // Trailing underscore not allowed
    if prev == b'_' {
        return Err(BytesToIntError::InvalidLiteral { base });
    }

    if digit_limit > 0 && !base.is_power_of_two() && digits > digit_limit {
        return Err(BytesToIntError::DigitLimit {
            got: digits,
            limit: digit_limit,
        });
    }

    let uint = BigUint::parse_bytes(buf, base).ok_or(BytesToIntError::InvalidLiteral { base })?;
    Ok(BigInt::from_biguint(sign.unwrap_or(Sign::Plus), uint))
}

// num-bigint now returns Some(inf) for to_f64() in some cases, so just keep that the same for now
#[inline(always)]
pub fn bigint_to_finite_float(int: &BigInt) -> Option<f64> {
    int.to_f64().filter(|f| f.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGIT_LIMIT: usize = 4300; // Default of Cpython

    #[test]
    fn bytes_to_int_valid() {
        for ((buf, base), expected) in [
            (("0b101", 2), BigInt::from(5)),
            (("0x_10", 16), BigInt::from(16)),
            (("0b", 16), BigInt::from(11)),
            (("+0b101", 2), BigInt::from(5)),
            (("0_0_0", 10), BigInt::from(0)),
            (("000", 0), BigInt::from(0)),
            (("0_100", 10), BigInt::from(100)),
        ] {
            assert_eq!(
                bytes_to_int(buf.as_bytes(), base, DIGIT_LIMIT),
                Ok(expected)
            );
        }
    }

    #[test]
    fn bytes_to_int_invalid_literal() {
        for ((buf, base), expected) in [
            (("09_99", 0), BytesToIntError::InvalidLiteral { base: 10 }),
            (("0_", 0), BytesToIntError::InvalidLiteral { base: 10 }),
            (("0_", 2), BytesToIntError::InvalidLiteral { base: 2 }),
        ] {
            assert_eq!(
                bytes_to_int(buf.as_bytes(), base, DIGIT_LIMIT),
                Err(expected)
            )
        }
    }

    #[test]
    fn bytes_to_int_invalid_base() {
        for base in [1, 37] {
            assert_eq!(
                bytes_to_int("012345".as_bytes(), base, DIGIT_LIMIT),
                Err(BytesToIntError::InvalidBase)
            )
        }
    }

    #[test]
    fn bytes_to_int_digit_limit() {
        assert_eq!(
            bytes_to_int("012345".as_bytes(), 10, 5),
            Err(BytesToIntError::DigitLimit { got: 6, limit: 5 })
        );
    }
}

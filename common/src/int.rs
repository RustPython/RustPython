use malachite_base::{num::conversion::traits::RoundingInto, rounding_modes::RoundingMode};
use malachite_bigint::{BigInt, BigUint, Sign};
use malachite_q::Rational;
use num_traits::{One, ToPrimitive, Zero};

pub fn true_div(numerator: &BigInt, denominator: &BigInt) -> f64 {
    let rational = Rational::from_integers_ref(numerator.into(), denominator.into());
    match rational.rounding_into(RoundingMode::Nearest) {
        // returned value is $t::MAX but still less than the original
        (val, std::cmp::Ordering::Less) if val == f64::MAX => f64::INFINITY,
        // returned value is $t::MIN but still greater than the original
        (val, std::cmp::Ordering::Greater) if val == f64::MIN => f64::NEG_INFINITY,
        (val, _) => val,
    }
}

pub fn float_to_ratio(value: f64) -> Option<(BigInt, BigInt)> {
    let sign = match std::cmp::PartialOrd::partial_cmp(&value, &0.0)? {
        std::cmp::Ordering::Less => Sign::Minus,
        std::cmp::Ordering::Equal => return Some((BigInt::zero(), BigInt::one())),
        std::cmp::Ordering::Greater => Sign::Plus,
    };
    Rational::try_from(value).ok().map(|x| {
        let (numer, denom) = x.into_numerator_and_denominator();
        (
            BigInt::from_biguint(sign, numer.into()),
            BigUint::from(denom).into(),
        )
    })
}

pub fn bytes_to_int(lit: &[u8], mut base: u32) -> Option<BigInt> {
    // split sign
    let mut lit = lit.trim_ascii();
    let sign = match lit.first()? {
        b'+' => Some(Sign::Plus),
        b'-' => Some(Sign::Minus),
        _ => None,
    };
    if sign.is_some() {
        lit = &lit[1..];
    }

    // split radix
    let first = *lit.first()?;
    let has_radix = if first == b'0' {
        match base {
            0 => {
                if let Some(parsed) = lit.get(1).and_then(detect_base) {
                    base = parsed;
                    true
                } else {
                    if let [_first, ref others @ .., last] = lit {
                        let is_zero =
                            others.iter().all(|&c| c == b'0' || c == b'_') && *last == b'0';
                        if !is_zero {
                            return None;
                        }
                    }
                    return Some(BigInt::zero());
                }
            }
            16 => lit.get(1).map_or(false, |&b| matches!(b, b'x' | b'X')),
            2 => lit.get(1).map_or(false, |&b| matches!(b, b'b' | b'B')),
            8 => lit.get(1).map_or(false, |&b| matches!(b, b'o' | b'O')),
            _ => false,
        }
    } else {
        if base == 0 {
            base = 10;
        }
        false
    };
    if has_radix {
        lit = &lit[2..];
        if lit.first()? == &b'_' {
            lit = &lit[1..];
        }
    }

    // remove zeroes
    let mut last = *lit.first()?;
    if last == b'0' {
        let mut count = 0;
        for &cur in &lit[1..] {
            if cur == b'_' {
                if last == b'_' {
                    return None;
                }
            } else if cur != b'0' {
                break;
            };
            count += 1;
            last = cur;
        }
        let prefix_last = lit[count];
        lit = &lit[count + 1..];
        if lit.is_empty() && prefix_last == b'_' {
            return None;
        }
    }

    // validate
    for c in lit {
        let c = *c;
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return None;
        }

        if c == b'_' && last == b'_' {
            return None;
        }

        last = c;
    }
    if last == b'_' {
        return None;
    }

    // parse
    let number = if lit.is_empty() {
        BigInt::zero()
    } else {
        let uint = BigUint::parse_bytes(lit, base)?;
        BigInt::from_biguint(sign.unwrap_or(Sign::Plus), uint)
    };
    Some(number)
}

#[inline]
pub fn detect_base(c: &u8) -> Option<u32> {
    let base = match c {
        b'x' | b'X' => 16,
        b'b' | b'B' => 2,
        b'o' | b'O' => 8,
        _ => return None,
    };
    Some(base)
}

// num-bigint now returns Some(inf) for to_f64() in some cases, so just keep that the same for now
#[inline(always)]
pub fn bigint_to_finite_float(int: &BigInt) -> Option<f64> {
    int.to_f64().filter(|f| f.is_finite())
}

#[test]
fn test_bytes_to_int() {
    assert_eq!(bytes_to_int(&b"0b101"[..], 2).unwrap(), BigInt::from(5));
    assert_eq!(bytes_to_int(&b"0x_10"[..], 16).unwrap(), BigInt::from(16));
    assert_eq!(bytes_to_int(&b"0b"[..], 16).unwrap(), BigInt::from(11));
    assert_eq!(bytes_to_int(&b"+0b101"[..], 2).unwrap(), BigInt::from(5));
    assert_eq!(bytes_to_int(&b"0_0_0"[..], 10).unwrap(), BigInt::from(0));
    assert_eq!(bytes_to_int(&b"09_99"[..], 0), None);
    assert_eq!(bytes_to_int(&b"000"[..], 0).unwrap(), BigInt::from(0));
    assert_eq!(bytes_to_int(&b"0_"[..], 0), None);
    assert_eq!(bytes_to_int(&b"0_100"[..], 10).unwrap(), BigInt::from(100));
}

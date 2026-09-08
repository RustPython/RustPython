//! Operations on decimals.
//!
//! Every routine takes the context by reference and reports the conditions it
//! raised through a status word; nothing is ever raised here. That is libmpdec's
//! model, and it is what makes a trapped signal carry the full list of
//! conditions an operation produced rather than only the first one.

pub mod arith;
pub mod compare;
pub mod misc;

use crate::bigops;
use crate::context::{Context, RoundMode};
use crate::dec::Decimal;
use crate::status;
use malachite_bigint::BigUint;
use num_traits::{One, Zero};

/// Above this many digits a coefficient is treated as unallocatable, which is
/// what libmpdec reports when a context's precision is set absurdly high.
const MAX_MATERIALIZABLE_DIGITS: i64 = 100_000_000;

/// Bounds that keep an exponent computation inside `i64` while staying far
/// enough outside the representable range that no comparison changes outcome.
const EXP_CEILING: i128 = i64::MAX as i128 / 4;
const EXP_FLOOR: i128 = i64::MIN as i128 / 4;

/// `'9' * n` as an integer, or `None` when that would not fit in memory.
pub(crate) fn nines(n: i64) -> Option<BigUint> {
    if n <= 0 {
        return Some(BigUint::zero());
    }
    if n > MAX_MATERIALIZABLE_DIGITS {
        return None;
    }
    Some(bigops::pow10(n as u64) - BigUint::one())
}

/// `Overflow.handle`: the value an overflowing operation settles on, which
/// depends on the rounding mode.
pub fn overflow_result(ctx: &Context, sign: u8, status: &mut u32) -> Decimal {
    let to_infinity = match ctx.round {
        RoundMode::HalfUp | RoundMode::HalfEven | RoundMode::HalfDown | RoundMode::Up => true,
        RoundMode::Ceiling => sign == 0,
        RoundMode::Floor => sign == 1,
        _ => false,
    };
    if to_infinity {
        return Decimal::infinity(sign);
    }
    match nines(ctx.prec) {
        Some(coeff) => Decimal::new_finite(sign, coeff, ctx.emax - ctx.prec + 1),
        None => {
            *status |= status::MALLOC_ERROR;
            Decimal::nan(0, BigUint::zero(), false)
        }
    }
}

/// `Decimal._fix_nan`: trim a NaN payload to what the context can hold.
pub fn fix_nan(d: &Decimal, ctx: &Context) -> Decimal {
    let max_payload_len = ctx.prec - i64::from(ctx.clamp);
    if d.digits() > max_payload_len {
        let payload = if max_payload_len <= 0 {
            BigUint::zero()
        } else {
            bigops::rem_pow10(d.coefficient(), max_payload_len as u64)
        };
        return Decimal::nan(d.sign(), payload, d.is_snan());
    }
    d.clone()
}

/// `Decimal._check_nans`: the result an operation must return when one of its
/// operands is a NaN, or `None` when neither is.
pub fn check_nans(
    a: &Decimal,
    b: Option<&Decimal>,
    ctx: &Context,
    status: &mut u32,
) -> Option<Decimal> {
    let a_nan = a.is_nan();
    let b_nan = b.is_some_and(Decimal::is_nan);
    if !a_nan && !b_nan {
        return None;
    }
    if a.is_snan() {
        return Some(quiet_snan(a, ctx, status));
    }
    if let Some(b) = b
        && b.is_snan()
    {
        return Some(quiet_snan(b, ctx, status));
    }
    if a_nan {
        return Some(fix_nan(a, ctx));
    }
    Some(fix_nan(b.expect("one operand is a NaN"), ctx))
}

/// `InvalidOperation.handle` for a signaling NaN operand: the same payload,
/// quieted.
pub fn quiet_snan(d: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    *status |= status::INVALID_OPERATION;
    fix_nan(&Decimal::nan(d.sign(), d.coefficient().clone(), false), ctx)
}

/// A plain quiet NaN, the result every unhandled invalid operation settles on.
pub fn invalid_operation(status: &mut u32) -> Decimal {
    *status |= status::INVALID_OPERATION;
    Decimal::nan(0, BigUint::zero(), false)
}

/// How a coefficient truncated to `keep` digits must be adjusted, following
/// `_pydecimal`'s rounding helpers: `1` to round away from zero, `0` when the
/// dropped digits were all zero, `-1` when they were not but the value stays.
///
/// `d` must be finite and nonzero, and `0 <= keep < d.digits()`.
pub fn round_indicator(d: &Decimal, keep: i64, round: RoundMode) -> i32 {
    debug_assert!(keep >= 0 && keep < d.digits());
    let drop = (d.digits() - keep) as u64;
    let (top, rest) = bigops::split_pow10(d.coefficient(), drop);
    let rest_is_zero = rest.is_zero();
    let down = || if rest_is_zero { 0 } else { -1 };
    // The first dropped digit, i.e. `_int[keep]`.
    let lead = if rest_is_zero {
        0u8
    } else {
        let scaled = bigops::div_pow10(&rest, drop - 1);
        u8::try_from(&scaled % BigUint::from(10u32)).unwrap_or(0)
    };
    // `_exact_half`: the dropped digits are exactly a five followed by zeros.
    let exact_half = lead == 5 && bigops::rem_pow10(&rest, drop - 1).is_zero();
    // The last kept digit, i.e. `_int[keep - 1]`.
    let last_kept = if keep > 0 {
        u8::try_from(&top % BigUint::from(10u32)).unwrap_or(0)
    } else {
        0
    };
    let half_up = || {
        if lead >= 5 {
            1
        } else if rest_is_zero {
            0
        } else {
            -1
        }
    };
    match round {
        RoundMode::Down => down(),
        RoundMode::Up => -down(),
        RoundMode::HalfUp => half_up(),
        RoundMode::HalfDown => {
            if exact_half {
                -1
            } else {
                half_up()
            }
        }
        RoundMode::HalfEven => {
            if exact_half && (keep == 0 || last_kept % 2 == 0) {
                -1
            } else {
                half_up()
            }
        }
        RoundMode::Ceiling => {
            if d.sign() != 0 {
                down()
            } else {
                -down()
            }
        }
        RoundMode::Floor => {
            if d.sign() == 0 {
                down()
            } else {
                -down()
            }
        }
        RoundMode::ZeroFiveUp => {
            if keep > 0 && last_kept != 0 && last_kept != 5 {
                down()
            } else {
                -down()
            }
        }
    }
}

/// `Decimal._fix`: round to the context's precision and pull the exponent into
/// the context's range.
pub fn fix(d: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if d.is_special() {
        return if d.is_nan() {
            fix_nan(d, ctx)
        } else {
            d.clone()
        };
    }

    let etiny = ctx.etiny();
    let etop = ctx.etop();

    if d.is_zero() {
        let exp_max = if ctx.clamp { etop } else { ctx.emax };
        let new_exp = d.exponent().max(etiny).min(exp_max);
        if new_exp != d.exponent() {
            *status |= status::CLAMPED;
            return Decimal::zero(d.sign(), new_exp);
        }
        return d.clone();
    }

    // The Python computes this with unbounded integers; a tuple-built value can
    // carry an exponent near the bottom of `i64`, so the arithmetic is done wide
    // and then pulled back into a range that keeps every comparison below exact.
    let exp_min_wide = i128::from(d.digits()) + i128::from(d.exponent()) - i128::from(ctx.prec);
    let mut exp_min = exp_min_wide.clamp(EXP_FLOOR, EXP_CEILING) as i64;
    if exp_min > etop {
        *status |= status::OVERFLOW | status::INEXACT | status::ROUNDED;
        return overflow_result(ctx, d.sign(), status);
    }

    let self_is_subnormal = exp_min < etiny;
    if self_is_subnormal {
        exp_min = etiny;
    }

    if d.exponent() < exp_min {
        let digits_wide = i128::from(d.digits()) + i128::from(d.exponent()) - i128::from(exp_min);
        let mut digits = digits_wide.clamp(EXP_FLOOR, EXP_CEILING) as i64;
        let scratch;
        let work = if digits < 0 {
            scratch = Decimal::new_finite(d.sign(), BigUint::one(), exp_min - 1);
            digits = 0;
            &scratch
        } else {
            d
        };
        let changed = round_indicator(work, digits, ctx.round);
        let mut coeff = if digits > 0 {
            bigops::div_pow10(work.coefficient(), (work.digits() - digits) as u64)
        } else {
            BigUint::zero()
        };
        if changed > 0 {
            coeff += BigUint::one();
            if bigops::digit_count(&coeff) > ctx.prec {
                coeff /= BigUint::from(10u32);
                exp_min += 1;
            }
        }

        let ans = if exp_min > etop {
            *status |= status::OVERFLOW;
            overflow_result(ctx, d.sign(), status)
        } else {
            Decimal::new_finite(d.sign(), coeff, exp_min)
        };

        // Signal precedence follows the specification: Underflow before
        // Subnormal before Inexact before Rounded, and Clamped last.
        if changed != 0 && self_is_subnormal {
            *status |= status::UNDERFLOW;
        }
        if self_is_subnormal {
            *status |= status::SUBNORMAL;
        }
        if changed != 0 {
            *status |= status::INEXACT;
        }
        *status |= status::ROUNDED;
        if !ans.is_nonzero() {
            *status |= status::CLAMPED;
        }
        return ans;
    }

    if self_is_subnormal {
        *status |= status::SUBNORMAL;
    }

    if ctx.clamp && d.exponent() > etop {
        *status |= status::CLAMPED;
        let shift = (d.exponent() - etop) as u64;
        let coeff = bigops::mul_pow10(d.coefficient(), shift);
        return Decimal::new_finite(d.sign(), coeff, etop);
    }

    d.clone()
}

/// `Decimal._round`: round to `places` significant digits.
///
/// `places` must be positive; a special or zero value comes back unaltered.
pub fn round_to_places(d: &Decimal, places: i64, round: RoundMode) -> Decimal {
    debug_assert!(places > 0);
    if d.is_special() || !d.is_nonzero() {
        return d.clone();
    }
    let target = |value: &Decimal| {
        (i128::from(value.adjusted()) + 1 - i128::from(places)).clamp(EXP_FLOOR, EXP_CEILING) as i64
    };
    let ans = arith::rescale(d, target(d), round);
    // Rounding 99.97 to three digits carries into a new leading digit, leaving
    // one digit too many; a second rescale trims it.
    if ans.adjusted() != d.adjusted() {
        return arith::rescale(&ans, target(&ans), round);
    }
    ans
}

/// `Decimal._islogical`: whether a logical operation may accept this operand.
pub fn is_logical(d: &Decimal) -> bool {
    if d.sign() != 0 || d.is_special() || d.exponent() != 0 {
        return false;
    }
    bigops::to_digits(d.coefficient())
        .into_iter()
        .all(|digit| digit == b'0' || digit == b'1')
}

/// `Decimal._cmp`: order two values that are known not to be NaN.
pub fn cmp_values(a: &Decimal, b: &Decimal) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    fn infinity_rank(d: &Decimal) -> i8 {
        if d.is_infinite() {
            if d.sign() == 0 { 1 } else { -1 }
        } else {
            0
        }
    }
    if a.is_special() || b.is_special() {
        return infinity_rank(a).cmp(&infinity_rank(b));
    }
    let a_zero = a.is_zero();
    let b_zero = b.is_zero();
    if a_zero {
        return if b_zero {
            Ordering::Equal
        } else if b.sign() == 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    if b_zero {
        return if a.sign() == 0 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if a.sign() != b.sign() {
        return if a.sign() == 0 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    let flip = |ord: Ordering| if a.sign() == 0 { ord } else { ord.reverse() };
    match a.adjusted().cmp(&b.adjusted()) {
        Ordering::Equal => {
            // Equal adjusted exponents bound the padding by the digit counts.
            let (x, y) = if a.exponent() >= b.exponent() {
                (
                    bigops::mul_pow10(a.coefficient(), (a.exponent() - b.exponent()) as u64),
                    b.coefficient().clone(),
                )
            } else {
                (
                    a.coefficient().clone(),
                    bigops::mul_pow10(b.coefficient(), (b.exponent() - a.exponent()) as u64),
                )
            };
            flip(x.cmp(&y))
        }
        other => flip(other),
    }
}

//! Core arithmetic: the operations the specification calls `add`, `subtract`,
//! `multiply`, `divide`, `divide-integer`, `remainder`, `remainder-near`,
//! `fused-multiply-add`, `quantize`, `round-to-integral` and `reduce`.
//!
//! Ported from `Lib/_pydecimal.py`.

use super::{MAX_MATERIALIZABLE_DIGITS, check_nans, fix, invalid_operation, quiet_snan};
use crate::bigops;
use crate::context::{Context, RoundMode};
use crate::dec::Decimal;
use crate::status;
use crate::{Special, ops};
use core::cmp::Ordering;
use core::mem::swap;
use malachite_bigint::BigUint;
use num_traits::{One, Zero};

// ---------------------------------------------------------------------------
// Shared helpers. `_pydecimal` never has to worry about a coefficient outgrowing
// memory because Python ints are unbounded; here every place that would build
// `'0' * n` padding or a product without an intervening `_fix` first checks
// that the result stays within what this engine is willing to materialize.
// ---------------------------------------------------------------------------

/// Saturating narrow of an exponent computed in wider arithmetic. The value is
/// only ever used to pick a branch or to seed a `Decimal` that a subsequent
/// `_fix`-equivalent will immediately clamp into the context's range, so
/// saturating instead of panicking preserves the same outcome.
fn sat_i64(x: i128) -> i64 {
    x.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// `Decimal.copy_negate`: flip the sign, otherwise unchanged. Defined locally
/// because `ops::compare` (where `_pydecimal` places it) is off limits.
fn copy_negate(d: &Decimal) -> Decimal {
    let sign = 1 - d.sign();
    match d.special() {
        Special::Finite => Decimal::new_finite(sign, d.coefficient().clone(), d.exponent()),
        Special::Inf => Decimal::infinity(sign),
        Special::Nan => Decimal::nan(sign, d.coefficient().clone(), false),
        Special::Snan => Decimal::nan(sign, d.coefficient().clone(), true),
    }
}

/// `Decimal.copy_abs`: force the sign positive, otherwise unchanged.
fn copy_abs(d: &Decimal) -> Decimal {
    match d.special() {
        Special::Finite => Decimal::new_finite(0, d.coefficient().clone(), d.exponent()),
        Special::Inf => Decimal::infinity(0),
        Special::Nan => Decimal::nan(0, d.coefficient().clone(), false),
        Special::Snan => Decimal::nan(0, d.coefficient().clone(), true),
    }
}

/// A plain quiet NaN raised for a condition other than `InvalidOperation`
/// itself (`DivisionUndefined`, `DivisionImpossible`), which still settle on
/// the same value but must not tag the result with the `InvalidOperation` bit.
fn quiet_nan_with(bit: u32, status: &mut u32) -> Decimal {
    *status |= bit;
    Decimal::nan(0, BigUint::zero(), false)
}

/// `DivisionByZero.handle`.
fn division_by_zero(sign: u8, status: &mut u32) -> Decimal {
    *status |= status::DIVISION_BY_ZERO;
    Decimal::infinity(sign)
}

/// `DivisionImpossible.handle`.
fn division_impossible(status: &mut u32) -> Decimal {
    quiet_nan_with(status::DIVISION_IMPOSSIBLE, status)
}

/// The value an operation that could not allocate its result settles on: a
/// quiet NaN tagged `MALLOC_ERROR`, libmpdec's answer to a request it cannot
/// satisfy in bounded memory.
fn malloc_error(status: &mut u32) -> Decimal {
    *status |= status::MALLOC_ERROR;
    Decimal::nan(0, BigUint::zero(), false)
}

/// Whether padding a `cur_digits`-digit coefficient with `shift` trailing
/// zeros (`shift` may be negative, meaning no padding at all) stays within
/// what this engine will materialize.
fn padding_is_safe(cur_digits: i64, shift: i128) -> bool {
    shift <= 0 || cur_digits as i128 + shift <= MAX_MATERIALIZABLE_DIGITS as i128
}

/// `n * 10**shift`, refusing to build a coefficient beyond the materializable
/// limit. `shift` may be given in the wide arithmetic exponent computations
/// use; a non-positive shift is always safe and returns `n` unchanged.
fn guarded_mul_pow10(
    n: &BigUint,
    cur_digits: i64,
    shift: i128,
    status: &mut u32,
) -> Option<BigUint> {
    if shift <= 0 {
        return Some(n.clone());
    }
    if !padding_is_safe(cur_digits, shift) {
        *status |= status::MALLOC_ERROR;
        return None;
    }
    Some(bigops::mul_pow10(n, shift as u64))
}

/// `rescale`, but refusing to pad a coefficient beyond the materializable
/// limit; `None` means the guard tripped and `status` has been updated.
fn safe_rescale(d: &Decimal, exp: i64, round: RoundMode, status: &mut u32) -> Option<Decimal> {
    if d.is_finite() && d.is_nonzero() && d.exponent() >= exp {
        let shift = d.exponent() as i128 - exp as i128;
        if !padding_is_safe(d.digits(), shift) {
            *status |= status::MALLOC_ERROR;
            return None;
        }
    }
    Some(rescale(d, exp, round))
}

/// `_WorkRep`: a decimal's sign, coefficient and exponent, mutated in place by
/// `add`'s alignment step.
struct WorkRep {
    sign: u8,
    int: BigUint,
    exp: i64,
}

impl WorkRep {
    fn from_decimal(d: &Decimal) -> Self {
        Self {
            sign: d.sign(),
            int: d.coefficient().clone(),
            exp: d.exponent(),
        }
    }
}

/// `_normalize`: align two work representations to a common exponent, without
/// letting the gap between exponents blow the shift up when the two operands'
/// magnitudes are wildly different (this is exactly the guard `_pydecimal`'s
/// docstring describes; here it is also a hard limit on materialized size).
/// Returns `false`, with `status` updated, if that limit cannot be honoured.
fn normalize_work_reps(op1: &mut WorkRep, op2: &mut WorkRep, prec: i64, status: &mut u32) -> bool {
    let (tmp, other): (&mut WorkRep, &mut WorkRep) = if op1.exp < op2.exp {
        (op2, op1)
    } else {
        (op1, op2)
    };
    let tmp_len = bigops::digit_count(&tmp.int);
    let other_len = bigops::digit_count(&other.int);
    let delta = (tmp_len as i128 - prec as i128 - 2).min(-1);
    let exp = tmp.exp as i128 + delta;
    if other_len as i128 + other.exp as i128 - 1 < exp {
        other.int = BigUint::one();
        other.exp = sat_i64(exp);
    }
    let shift = tmp.exp as i128 - other.exp as i128;
    debug_assert!(shift >= 0, "tmp is chosen to have the larger exponent");
    if !padding_is_safe(tmp_len, shift) {
        *status |= status::MALLOC_ERROR;
        return false;
    }
    tmp.int = bigops::mul_pow10(&tmp.int, shift as u64);
    tmp.exp = other.exp;
    true
}

/// `Decimal.__neg__`.
pub fn neg(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special()
        && let Some(ans) = check_nans(a, None, ctx, status)
    {
        return ans;
    }

    let ans = if !a.is_nonzero() && ctx.round != RoundMode::Floor {
        // -Decimal('0') is Decimal('0'), not Decimal('-0'), except in
        // ROUND_FLOOR mode.
        copy_abs(a)
    } else {
        copy_negate(a)
    };
    fix(&ans, ctx, status)
}

/// `Decimal.__pos__`.
pub fn pos(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special()
        && let Some(ans) = check_nans(a, None, ctx, status)
    {
        return ans;
    }

    let ans = if !a.is_nonzero() && ctx.round != RoundMode::Floor {
        copy_abs(a)
    } else {
        a.clone()
    };
    fix(&ans, ctx, status)
}

/// `Decimal.__abs__`; `round` is false for `Context.copy_abs`-style callers
/// that must not apply the context.
pub fn abs(a: &Decimal, round: bool, ctx: &Context, status: &mut u32) -> Decimal {
    if !round {
        return copy_abs(a);
    }
    if a.is_special()
        && let Some(ans) = check_nans(a, None, ctx, status)
    {
        return ans;
    }
    if a.sign() != 0 {
        neg(a, ctx, status)
    } else {
        pos(a, ctx, status)
    }
}

/// `Decimal.__add__`.
pub fn add(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() || b.is_special() {
        if let Some(ans) = check_nans(a, Some(b), ctx, status) {
            return ans;
        }
        if a.is_infinite() {
            if a.sign() != b.sign() && b.is_infinite() {
                return invalid_operation(status);
            }
            return a.clone();
        }
        if b.is_infinite() {
            return b.clone();
        }
    }

    let exp = a.exponent().min(b.exponent());
    let negativezero = ctx.round == RoundMode::Floor && a.sign() != b.sign();

    if !a.is_nonzero() && !b.is_nonzero() {
        let sign = if negativezero {
            1
        } else {
            a.sign().min(b.sign())
        };
        let ans = Decimal::new_finite(sign, BigUint::zero(), exp);
        return fix(&ans, ctx, status);
    }
    if !a.is_nonzero() {
        let exp = sat_i64((exp as i128).max(b.exponent() as i128 - ctx.prec as i128 - 1));
        let ans = match safe_rescale(b, exp, ctx.round, status) {
            Some(ans) => ans,
            None => return malloc_error(status),
        };
        return fix(&ans, ctx, status);
    }
    if !b.is_nonzero() {
        let exp = sat_i64((exp as i128).max(a.exponent() as i128 - ctx.prec as i128 - 1));
        let ans = match safe_rescale(a, exp, ctx.round, status) {
            Some(ans) => ans,
            None => return malloc_error(status),
        };
        return fix(&ans, ctx, status);
    }

    let mut op1 = WorkRep::from_decimal(a);
    let mut op2 = WorkRep::from_decimal(b);
    if !normalize_work_reps(&mut op1, &mut op2, ctx.prec, status) {
        return malloc_error(status);
    }

    let (result_sign, result_int);
    if op1.sign != op2.sign {
        if op1.int == op2.int {
            let ans = Decimal::new_finite(u8::from(negativezero), BigUint::zero(), exp);
            return fix(&ans, ctx, status);
        }
        if op1.int < op2.int {
            swap(&mut op1, &mut op2);
        }
        // op1 is now the larger magnitude, so op1.int - op2.int > 0.
        result_sign = op1.sign;
        result_int = &op1.int - &op2.int;
    } else {
        result_sign = op1.sign;
        result_int = &op1.int + &op2.int;
    }

    let ans = Decimal::new_finite(result_sign, result_int, op1.exp);
    fix(&ans, ctx, status)
}

/// `Decimal.__sub__`.
pub fn sub(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if (a.is_special() || b.is_special())
        && let Some(ans) = check_nans(a, Some(b), ctx, status)
    {
        return ans;
    }
    // self - other is computed as self + other.copy_negate().
    add(a, &copy_negate(b), ctx, status)
}

/// `Decimal.__mul__`.
pub fn mul(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    let result_sign = a.sign() ^ b.sign();

    if a.is_special() || b.is_special() {
        if let Some(ans) = check_nans(a, Some(b), ctx, status) {
            return ans;
        }
        if a.is_infinite() {
            if !b.is_nonzero() {
                return invalid_operation(status);
            }
            return Decimal::infinity(result_sign);
        }
        if b.is_infinite() {
            if !a.is_nonzero() {
                return invalid_operation(status);
            }
            return Decimal::infinity(result_sign);
        }
    }

    let result_exp = sat_i64(a.exponent() as i128 + b.exponent() as i128);

    if !a.is_nonzero() || !b.is_nonzero() {
        let ans = Decimal::new_finite(result_sign, BigUint::zero(), result_exp);
        return fix(&ans, ctx, status);
    }

    let one = BigUint::one();
    if *a.coefficient() == one {
        let ans = Decimal::new_finite(result_sign, b.coefficient().clone(), result_exp);
        return fix(&ans, ctx, status);
    }
    if *b.coefficient() == one {
        let ans = Decimal::new_finite(result_sign, a.coefficient().clone(), result_exp);
        return fix(&ans, ctx, status);
    }

    if a.digits().saturating_add(b.digits()) > MAX_MATERIALIZABLE_DIGITS {
        return malloc_error(status);
    }
    let coeff = a.coefficient() * b.coefficient();
    let ans = Decimal::new_finite(result_sign, coeff, result_exp);
    fix(&ans, ctx, status)
}

/// `Decimal.__truediv__`.
/// The exact quotient of two coefficients, when it terminates.
///
/// `div` normally scales the dividend by the context precision, which is what
/// correct rounding of a non-terminating quotient needs. Under the unbounded
/// context (`prec = MAX_PREC`) that scaling is unaffordable even though the
/// answer may be tiny -- `1 / 256` is `0.00390625` -- so this computes the
/// terminating case directly instead. A quotient terminates exactly when the
/// divisor, stripped of its factors of two and five, divides the dividend;
/// what is left is a shift by the larger of the two factor counts.
fn exact_quotient(num: &BigUint, den: &BigUint) -> Option<(BigUint, u64)> {
    let mut stripped = den.clone();
    let twos = stripped.trailing_zeros().unwrap_or(0);
    stripped >>= twos;
    let mut fives: u64 = 0;
    let five = BigUint::from(5u32);
    while (&stripped % &five).is_zero() {
        stripped /= &five;
        fives += 1;
    }
    if !stripped.is_one() || !(num % &stripped).is_zero() {
        return None;
    }
    let shift = twos.max(fives);
    // `pow` below takes a `u32`, and a shift anywhere near that many digits
    // could not be held in memory anyway.
    if shift > MAX_MATERIALIZABLE_DIGITS as u64 {
        return None;
    }
    let mut coeff = num / &stripped;
    coeff <<= shift - twos;
    coeff *= five.pow(u32::try_from(shift - fives).ok()?);
    Some((coeff, shift))
}

pub fn div(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    let sign = a.sign() ^ b.sign();

    if a.is_special() || b.is_special() {
        if let Some(ans) = check_nans(a, Some(b), ctx, status) {
            return ans;
        }
        if a.is_infinite() && b.is_infinite() {
            return invalid_operation(status);
        }
        if a.is_infinite() {
            return Decimal::infinity(sign);
        }
        if b.is_infinite() {
            *status |= status::CLAMPED;
            return Decimal::zero(sign, ctx.etiny());
        }
    }

    if !b.is_nonzero() {
        if !a.is_nonzero() {
            return quiet_nan_with(status::DIVISION_UNDEFINED, status);
        }
        return division_by_zero(sign, status);
    }

    if !a.is_nonzero() {
        let exp = sat_i64(a.exponent() as i128 - b.exponent() as i128);
        let ans = Decimal::new_finite(sign, BigUint::zero(), exp);
        return fix(&ans, ctx, status);
    }

    let shift = b.digits() as i128 - a.digits() as i128 + ctx.prec as i128 + 1;
    let exp = a.exponent() as i128 - b.exponent() as i128 - shift;

    // A precision this large cannot be scaled to, but the quotient may still
    // terminate well inside it; `_pylong` divides under exactly such a context
    // to build exact reciprocals.
    if shift > 0
        && !padding_is_safe(a.digits(), shift)
        && let Some((coeff, used)) = exact_quotient(a.coefficient(), b.coefficient())
    {
        let exp = a.exponent() as i128 - b.exponent() as i128 - used as i128;
        let ans = Decimal::new_finite(sign, coeff, sat_i64(exp));
        return fix(&ans, ctx, status);
    }

    let (mut coeff, remainder) = if shift >= 0 {
        let lhs = match guarded_mul_pow10(a.coefficient(), a.digits(), shift, status) {
            Some(v) => v,
            None => return malloc_error(status),
        };
        (&lhs / b.coefficient(), &lhs % b.coefficient())
    } else {
        let rhs = match guarded_mul_pow10(b.coefficient(), b.digits(), -shift, status) {
            Some(v) => v,
            None => return malloc_error(status),
        };
        (a.coefficient() / &rhs, a.coefficient() % &rhs)
    };

    let mut exp = exp;
    if !remainder.is_zero() {
        // Result is not exact; adjust to ensure correct rounding.
        if (&coeff % BigUint::from(5u32)).is_zero() {
            coeff += BigUint::one();
        }
    } else {
        // Result is exact; get as close to the ideal exponent as possible.
        let ideal_exp = a.exponent() as i128 - b.exponent() as i128;
        while exp < ideal_exp && !coeff.is_zero() && (&coeff % BigUint::from(10u32)).is_zero() {
            coeff /= BigUint::from(10u32);
            exp += 1;
        }
    }

    let ans = Decimal::new_finite(sign, coeff, sat_i64(exp));
    fix(&ans, ctx, status)
}

/// `Decimal._divide`: quotient and remainder to `ctx.prec` precision. `a` must
/// be finite, `b` must be nonzero, and neither may be a NaN.
fn divide(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> (Decimal, Decimal) {
    let sign = a.sign() ^ b.sign();
    let ideal_exp = if b.is_infinite() {
        a.exponent()
    } else {
        a.exponent().min(b.exponent())
    };

    let expdiff = a.adjusted() as i128 - b.adjusted() as i128;
    if !a.is_nonzero() || b.is_infinite() || expdiff <= -2 {
        let q = Decimal::new_finite(sign, BigUint::zero(), 0);
        let r = match safe_rescale(a, ideal_exp, ctx.round, status) {
            Some(r) => r,
            None => {
                let nan = malloc_error(status);
                return (nan.clone(), nan);
            }
        };
        return (q, r);
    }

    if expdiff <= ctx.prec as i128 {
        let mut int1 = a.coefficient().clone();
        let mut int2 = b.coefficient().clone();
        let shift = a.exponent() as i128 - b.exponent() as i128;
        let ok = if shift >= 0 {
            guarded_mul_pow10(&int1, a.digits(), shift, status).map(|v| int1 = v)
        } else {
            guarded_mul_pow10(&int2, b.digits(), -shift, status).map(|v| int2 = v)
        };
        if ok.is_some() {
            let q = &int1 / &int2;
            let r = &int1 - &(&q * &int2);
            if q.is_zero() || bigops::digit_count(&q) <= ctx.prec {
                let quotient = Decimal::new_finite(sign, q, 0);
                let remainder = Decimal::new_finite(a.sign(), r, ideal_exp);
                return (quotient, remainder);
            }
        } else {
            let nan = malloc_error(status);
            return (nan.clone(), nan);
        }
    }

    // Here the quotient is too large to be representable.
    let ans = division_impossible(status);
    (ans.clone(), ans)
}

/// `Decimal.__divmod__`, returning the quotient and the remainder.
pub fn divmod(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> (Decimal, Decimal) {
    if let Some(ans) = check_nans(a, Some(b), ctx, status) {
        return (ans.clone(), ans);
    }

    let sign = a.sign() ^ b.sign();
    if a.is_infinite() {
        if b.is_infinite() {
            let ans = invalid_operation(status);
            return (ans.clone(), ans);
        }
        return (Decimal::infinity(sign), invalid_operation(status));
    }

    if !b.is_nonzero() {
        if !a.is_nonzero() {
            let ans = quiet_nan_with(status::DIVISION_UNDEFINED, status);
            return (ans.clone(), ans);
        }
        return (division_by_zero(sign, status), invalid_operation(status));
    }

    let (quotient, remainder) = divide(a, b, ctx, status);
    let quotient = finalize_quotient(&quotient, b, ctx, status);
    let remainder = fix(&remainder, ctx, status);
    (quotient, remainder)
}

/// libmpdec finalizes the quotient of `//` and `divmod`, so one outside the
/// exponent range overflows rather than escaping the context. `_pydecimal`
/// leaves it alone, and `_decimal` is the reference here. The one exception is
/// an infinite divisor, where libmpdec returns the zero quotient untouched.
fn finalize_quotient(quotient: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if b.is_infinite() {
        quotient.clone()
    } else {
        fix(quotient, ctx, status)
    }
}

/// `Decimal.__mod__`.
pub fn rem(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(ans) = check_nans(a, Some(b), ctx, status) {
        return ans;
    }

    if a.is_infinite() {
        return invalid_operation(status);
    }
    if !b.is_nonzero() {
        if a.is_nonzero() {
            return invalid_operation(status);
        }
        return quiet_nan_with(status::DIVISION_UNDEFINED, status);
    }

    let remainder = divide(a, b, ctx, status).1;
    fix(&remainder, ctx, status)
}

/// `Decimal.__floordiv__`.
pub fn floordiv(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(ans) = check_nans(a, Some(b), ctx, status) {
        return ans;
    }

    if a.is_infinite() {
        if b.is_infinite() {
            return invalid_operation(status);
        }
        return Decimal::infinity(a.sign() ^ b.sign());
    }
    if !b.is_nonzero() {
        if a.is_nonzero() {
            return division_by_zero(a.sign() ^ b.sign(), status);
        }
        return quiet_nan_with(status::DIVISION_UNDEFINED, status);
    }

    let quotient = divide(a, b, ctx, status).0;
    finalize_quotient(&quotient, b, ctx, status)
}

/// `Decimal.remainder_near`.
pub fn remainder_near(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(ans) = check_nans(a, Some(b), ctx, status) {
        return ans;
    }

    if a.is_infinite() {
        return invalid_operation(status);
    }
    if !b.is_nonzero() {
        if a.is_nonzero() {
            return invalid_operation(status);
        }
        return quiet_nan_with(status::DIVISION_UNDEFINED, status);
    }
    if b.is_infinite() {
        return fix(a, ctx, status);
    }

    let ideal_exponent = a.exponent().min(b.exponent());
    if !a.is_nonzero() {
        let ans = Decimal::new_finite(a.sign(), BigUint::zero(), ideal_exponent);
        return fix(&ans, ctx, status);
    }

    // Catch most cases of a very large or very small quotient.
    let expdiff = a.adjusted() as i128 - b.adjusted() as i128;
    if expdiff > ctx.prec as i128 {
        return division_impossible(status);
    }
    if expdiff <= -2 {
        let ans = match safe_rescale(a, ideal_exponent, ctx.round, status) {
            Some(ans) => ans,
            None => return malloc_error(status),
        };
        return fix(&ans, ctx, status);
    }

    // Adjust both arguments to have the same exponent, then divide.
    let mut int1 = a.coefficient().clone();
    let mut int2 = b.coefficient().clone();
    let shift = a.exponent() as i128 - b.exponent() as i128;
    let guarded = if shift >= 0 {
        guarded_mul_pow10(&int1, a.digits(), shift, status).map(|v| int1 = v)
    } else {
        guarded_mul_pow10(&int2, b.digits(), -shift, status).map(|v| int2 = v)
    };
    if guarded.is_none() {
        return malloc_error(status);
    }

    let mut q = &int1 / &int2;
    let mut r = &int1 - &(&q * &int2);

    // Remainder is r*10**ideal_exponent; other is +/-int2*10**ideal_exponent.
    // Apply a correction to ensure that abs(remainder) <= abs(other)/2.
    let q_odd = &q % BigUint::from(2u32) == BigUint::one();
    let two_r = &r * BigUint::from(2u32);
    let lhs = if q_odd { two_r + BigUint::one() } else { two_r };
    let mut r_negative = false;
    if lhs > int2 {
        // r - int2 is always negative here, since r < int2 (a divmod
        // remainder).
        r = &int2 - &r;
        r_negative = true;
        q += BigUint::one();
    }

    if bigops::digit_count(&q) > ctx.prec {
        return division_impossible(status);
    }

    // Result has the same sign as self, unless r went negative above.
    let sign = if r_negative { 1 - a.sign() } else { a.sign() };
    let ans = Decimal::new_finite(sign, r, ideal_exponent);
    fix(&ans, ctx, status)
}

/// `Decimal.fma`: `a * b + c` with a single rounding.
pub fn fma(a: &Decimal, b: &Decimal, c: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    let product = if a.is_special() || b.is_special() {
        if a.is_snan() {
            return quiet_snan(a, ctx, status);
        }
        if b.is_snan() {
            return quiet_snan(b, ctx, status);
        }
        if a.is_qnan() {
            a.clone()
        } else if b.is_qnan() {
            b.clone()
        } else if a.is_infinite() {
            if !b.is_nonzero() {
                return invalid_operation(status);
            }
            Decimal::infinity(a.sign() ^ b.sign())
        } else if b.is_infinite() {
            if !a.is_nonzero() {
                return invalid_operation(status);
            }
            Decimal::infinity(a.sign() ^ b.sign())
        } else {
            // Both finite, non-special: unreachable, since the outer `if`
            // required one of them to be special.
            unreachable!("a or b is special but neither NaN nor infinite")
        }
    } else {
        if a.digits().saturating_add(b.digits()) > MAX_MATERIALIZABLE_DIGITS {
            return malloc_error(status);
        }
        let coeff = a.coefficient() * b.coefficient();
        let exp = sat_i64(a.exponent() as i128 + b.exponent() as i128);
        Decimal::new_finite(a.sign() ^ b.sign(), coeff, exp)
    };

    add(&product, c, ctx, status)
}

/// `Decimal.normalize`, the specification's `reduce`.
pub fn normalize(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special()
        && let Some(ans) = check_nans(a, None, ctx, status)
    {
        return ans;
    }

    let dup = fix(a, ctx, status);
    if dup.is_infinite() {
        return dup;
    }
    if !dup.is_nonzero() {
        return Decimal::new_finite(dup.sign(), BigUint::zero(), 0);
    }

    let exp_max = if ctx.clamp { ctx.etop() } else { ctx.emax };
    let tz = bigops::trailing_zeros10(dup.coefficient()).unwrap_or(0);
    let max_remove = (exp_max as i128 - dup.exponent() as i128).max(0);
    let remove = (tz as i128).min(max_remove) as u64;

    let new_coeff = bigops::div_pow10(dup.coefficient(), remove);
    let new_exp = sat_i64(dup.exponent() as i128 + remove as i128);
    Decimal::new_finite(dup.sign(), new_coeff, new_exp)
}

/// `Decimal.quantize`.
pub fn quantize(
    a: &Decimal,
    exp: &Decimal,
    round: RoundMode,
    ctx: &Context,
    status: &mut u32,
) -> Decimal {
    if a.is_special() || exp.is_special() {
        if let Some(ans) = check_nans(a, Some(exp), ctx, status) {
            return ans;
        }
        if exp.is_infinite() || a.is_infinite() {
            if exp.is_infinite() && a.is_infinite() {
                return a.clone();
            }
            return invalid_operation(status);
        }
    }

    let target_exp = exp.exponent();
    if target_exp < ctx.etiny() || target_exp > ctx.emax {
        return invalid_operation(status);
    }

    if !a.is_nonzero() {
        let ans = Decimal::new_finite(a.sign(), BigUint::zero(), target_exp);
        return fix(&ans, ctx, status);
    }

    let self_adjusted = a.adjusted();
    if self_adjusted > ctx.emax {
        return invalid_operation(status);
    }
    if self_adjusted as i128 - target_exp as i128 + 1 > ctx.prec as i128 {
        return invalid_operation(status);
    }

    let ans = match safe_rescale(a, target_exp, round, status) {
        Some(ans) => ans,
        None => return malloc_error(status),
    };
    if ans.adjusted() > ctx.emax {
        return invalid_operation(status);
    }
    if ans.digits() > ctx.prec {
        return invalid_operation(status);
    }

    if ans.is_nonzero() && ans.adjusted() < ctx.emin {
        *status |= status::SUBNORMAL;
    }
    if ans.exponent() > a.exponent() {
        if ops::cmp_values(&ans, a) != Ordering::Equal {
            *status |= status::INEXACT;
        }
        *status |= status::ROUNDED;
    }

    // The call to fix takes care of any necessary folddown and signals
    // Clamped if necessary.
    fix(&ans, ctx, status)
}

/// `Decimal._rescale`: set the exponent to `exp`, rounding the coefficient.
/// Raises no conditions; the caller decides what the change means.
pub fn rescale(a: &Decimal, exp: i64, round: RoundMode) -> Decimal {
    if a.is_special() {
        return a.clone();
    }
    if !a.is_nonzero() {
        return Decimal::new_finite(a.sign(), BigUint::zero(), exp);
    }

    if a.exponent() >= exp {
        // Pad with zeros if necessary.
        let shift = (a.exponent() as i128 - exp as i128) as u64;
        let coeff = bigops::mul_pow10(a.coefficient(), shift);
        return Decimal::new_finite(a.sign(), coeff, exp);
    }

    // Too many digits; round and lose data. If the value's adjusted exponent
    // is below exp - 1, replace it by 10**(exp - 1) before rounding.
    let digits_wide = a.digits() as i128 + a.exponent() as i128 - exp as i128;
    let (work, digits) = if digits_wide < 0 {
        (Decimal::new_finite(a.sign(), BigUint::one(), exp - 1), 0i64)
    } else {
        (a.clone(), digits_wide as i64)
    };

    let changed = ops::round_indicator(&work, digits, round);
    let mut coeff = if digits > 0 {
        bigops::div_pow10(work.coefficient(), (work.digits() - digits) as u64)
    } else {
        BigUint::zero()
    };
    if changed == 1 {
        coeff += BigUint::one();
    }
    Decimal::new_finite(a.sign(), coeff, exp)
}

/// `Decimal.to_integral_exact`.
pub fn to_integral_exact(
    a: &Decimal,
    round: RoundMode,
    ctx: &Context,
    status: &mut u32,
) -> Decimal {
    if a.is_special() {
        if let Some(ans) = check_nans(a, None, ctx, status) {
            return ans;
        }
        return a.clone();
    }
    if a.exponent() >= 0 {
        return a.clone();
    }
    if !a.is_nonzero() {
        return Decimal::new_finite(a.sign(), BigUint::zero(), 0);
    }

    // exp = 0 > a.exponent() here, so this only ever drops digits and never
    // needs the padding guard.
    let ans = rescale(a, 0, round);
    if ops::cmp_values(&ans, a) != Ordering::Equal {
        *status |= status::INEXACT;
    }
    *status |= status::ROUNDED;
    ans
}

/// `Decimal.to_integral_value`.
pub fn to_integral_value(
    a: &Decimal,
    round: RoundMode,
    ctx: &Context,
    status: &mut u32,
) -> Decimal {
    if a.is_special() {
        if let Some(ans) = check_nans(a, None, ctx, status) {
            return ans;
        }
        return a.clone();
    }
    if a.exponent() >= 0 {
        return a.clone();
    }
    rescale(a, 0, round)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, RoundMode};
    use crate::dec::Decimal;

    fn d(s: &str) -> Decimal {
        Decimal::parse_ascii(s.as_bytes()).expect("valid literal")
    }

    fn ctx(prec: i64, round: RoundMode) -> Context {
        Context {
            prec,
            round,
            ..Context::default()
        }
    }

    fn check(got: &Decimal, got_status: u32, expected_str: &str, expected_status: u32) {
        assert_eq!(got.to_sci_string(true), expected_str, "value mismatch");
        assert_eq!(
            got_status, expected_status,
            "status mismatch for {expected_str}"
        );
    }

    // --- add -----------------------------------------------------------

    #[test]
    fn add_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("1.1"), &d("2.2"), &c, &mut s);
        check(&r, s, "3.3", 0);
    }

    #[test]
    fn add_signed_zeros_default_round() {
        // 0 + (-0) => 0 outside ROUND_FLOOR.
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("0"), &d("-0"), &c, &mut s);
        check(&r, s, "0", 0);
    }

    #[test]
    fn add_signed_zeros_floor_round() {
        // Python: Context(rounding=ROUND_FLOOR).add(Decimal('0'), Decimal('-0')) == -0
        let c = ctx(9, RoundMode::Floor);
        let mut s = 0u32;
        let r = add(&d("0"), &d("-0"), &c, &mut s);
        check(&r, s, "-0", 0);
    }

    #[test]
    fn add_opposite_infinities_invalid() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("Infinity"), &d("-Infinity"), &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn add_same_sign_infinities() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("Infinity"), &d("Infinity"), &c, &mut s);
        check(&r, s, "Infinity", 0);
    }

    #[test]
    fn add_snan_propagates() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("sNaN123"), &d("1"), &c, &mut s);
        check(&r, s, "NaN123", status::INVALID_OPERATION);
    }

    #[test]
    fn add_rounds_and_flags_inexact_rounded() {
        // python: Context(prec=3).add(Decimal('1.23'), Decimal('0.0049')) -> 1.23, Inexact+Rounded
        let c = ctx(3, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = add(&d("1.23"), &d("0.0049"), &c, &mut s);
        check(&r, s, "1.23", status::INEXACT | status::ROUNDED);
    }

    #[test]
    fn add_overflow() {
        // python: Context(prec=3, Emax=5).add(Decimal('9.99E+5'), Decimal('9.99E+5'))
        let mut c = ctx(3, RoundMode::HalfEven);
        c.emax = 5;
        let mut s = 0u32;
        let r = add(&d("9.99E+5"), &d("9.99E+5"), &c, &mut s);
        check(
            &r,
            s,
            "Infinity",
            status::OVERFLOW | status::INEXACT | status::ROUNDED,
        );
    }

    // --- sub -------------------------------------------------------------

    #[test]
    fn sub_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = sub(&d("5"), &d("3.2"), &c, &mut s);
        check(&r, s, "1.8", 0);
    }

    // --- mul ---------------------------------------------------------------

    #[test]
    fn mul_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = mul(&d("1.2"), &d("3"), &c, &mut s);
        check(&r, s, "3.6", 0);
    }

    #[test]
    fn mul_zero_times_infinity_invalid() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = mul(&d("0"), &d("Infinity"), &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn mul_signs() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = mul(&d("-2"), &d("3"), &c, &mut s);
        check(&r, s, "-6", 0);
    }

    // --- div -----------------------------------------------------------

    #[test]
    fn div_one_third() {
        // python: Context(prec=9).divide(Decimal('1'), Decimal('3')) -> 0.333333333, Inexact+Rounded
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = div(&d("1"), &d("3"), &c, &mut s);
        check(&r, s, "0.333333333", status::INEXACT | status::ROUNDED);
    }

    #[test]
    fn div_exact() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = div(&d("1"), &d("4"), &c, &mut s);
        check(&r, s, "0.25", 0);
    }

    #[test]
    fn div_by_zero() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = div(&d("1"), &d("0"), &c, &mut s);
        check(&r, s, "Infinity", status::DIVISION_BY_ZERO);
    }

    #[test]
    fn div_zero_by_zero() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = div(&d("0"), &d("0"), &c, &mut s);
        check(&r, s, "NaN", status::DIVISION_UNDEFINED);
    }

    #[test]
    fn div_infinities_invalid() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = div(&d("Infinity"), &d("Infinity"), &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn div_by_infinity() {
        let mut c = ctx(9, RoundMode::HalfEven);
        c.emin = -999_999;
        let mut s = 0u32;
        let r = div(&d("5"), &d("Infinity"), &c, &mut s);
        check(&r, s, &format!("0E-{}", 999_999 + 9 - 1), status::CLAMPED);
    }

    // --- divmod / rem / floordiv -----------------------------------------

    #[test]
    fn divmod_basic() {
        // python: Context(prec=9).divmod(Decimal('7'), Decimal('2')) -> (3, 1)
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let (q, r) = divmod(&d("7"), &d("2"), &c, &mut s);
        check(&q, 0, "3", 0);
        check(&r, s, "1", 0);
    }

    #[test]
    fn divmod_negative() {
        // python: Context(prec=9).divmod(Decimal('-7'), Decimal('2')) -> (-3, -1)
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let (q, r) = divmod(&d("-7"), &d("2"), &c, &mut s);
        check(&q, 0, "-3", 0);
        check(&r, s, "-1", 0);
    }

    #[test]
    fn rem_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = rem(&d("7"), &d("2"), &c, &mut s);
        check(&r, s, "1", 0);
    }

    #[test]
    fn rem_infinity_invalid() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = rem(&d("Infinity"), &d("2"), &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn floordiv_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = floordiv(&d("7"), &d("2"), &c, &mut s);
        check(&r, s, "3", 0);
    }

    // --- remainder_near ----------------------------------------------------

    #[test]
    fn remainder_near_basic() {
        // python: Decimal(10).remainder_near(6) -> -2
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = remainder_near(&d("10"), &d("6"), &c, &mut s);
        check(&r, s, "-2", 0);
    }

    #[test]
    fn remainder_near_half_goes_even() {
        // python: Decimal(3).remainder_near(6) -> 3 (tie broken toward even quotient q=0 vs 1... verified via oracle)
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = remainder_near(&d("3"), &d("6"), &c, &mut s);
        check(&r, s, "3", 0);
    }

    // --- fma -----------------------------------------------------------

    #[test]
    fn fma_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = fma(&d("3"), &d("4"), &d("5"), &c, &mut s);
        check(&r, s, "17", 0);
    }

    #[test]
    fn fma_no_intermediate_rounding() {
        // Product isn't rounded before the add, unlike self*other then +third.
        let c = ctx(3, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = fma(&d("123456"), &d("1"), &d("0"), &c, &mut s);
        // product = 123456 exactly, then +0, then rounded to prec=3 at the end.
        check(&r, s, "1.23E+5", status::INEXACT | status::ROUNDED);
    }

    // --- normalize -------------------------------------------------------

    #[test]
    fn normalize_strips_trailing_zeros() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = normalize(&d("1.230"), &c, &mut s);
        check(&r, s, "1.23", 0);
    }

    #[test]
    fn normalize_zero() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = normalize(&d("0.000"), &c, &mut s);
        check(&r, s, "0", 0);
    }

    // --- quantize --------------------------------------------------------

    #[test]
    fn quantize_basic() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = quantize(
            &d("1.41421356"),
            &d("1.000"),
            RoundMode::HalfEven,
            &c,
            &mut s,
        );
        check(&r, s, "1.414", status::INEXACT | status::ROUNDED);
    }

    #[test]
    fn quantize_too_many_digits_invalid() {
        // python: Context(prec=3).quantize(Decimal('1234'), Decimal('1')) -> InvalidOperation
        let c = ctx(3, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = quantize(&d("1234"), &d("1"), RoundMode::HalfEven, &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn quantize_exponent_out_of_range() {
        let mut c = ctx(9, RoundMode::HalfEven);
        c.emax = 5;
        let mut s = 0u32;
        let r = quantize(&d("1"), &d("1E+10"), RoundMode::HalfEven, &c, &mut s);
        check(&r, s, "NaN", status::INVALID_OPERATION);
    }

    #[test]
    fn quantize_both_infinite() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = quantize(
            &d("Infinity"),
            &d("-Infinity"),
            RoundMode::HalfEven,
            &c,
            &mut s,
        );
        check(&r, s, "Infinity", 0);
    }

    // --- rescale ---------------------------------------------------------

    #[test]
    fn rescale_pads_zeros() {
        let r = rescale(&d("1.2"), -5, RoundMode::HalfEven);
        check(&r, 0, "1.20000", 0);
    }

    #[test]
    fn rescale_rounds() {
        let r = rescale(&d("1.256"), -2, RoundMode::HalfEven);
        check(&r, 0, "1.26", 0);
    }

    // --- to_integral_exact / to_integral_value ----------------------------

    #[test]
    fn to_integral_exact_rounds_half_even() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = to_integral_exact(&d("2.5"), RoundMode::HalfEven, &c, &mut s);
        check(&r, s, "2", status::INEXACT | status::ROUNDED);
    }

    #[test]
    fn to_integral_value_no_flags() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = to_integral_value(&d("2.5"), RoundMode::HalfEven, &c, &mut s);
        check(&r, s, "2", 0);
    }

    #[test]
    fn to_integral_already_integer() {
        let c = ctx(9, RoundMode::HalfEven);
        let mut s = 0u32;
        let r = to_integral_value(&d("100"), RoundMode::HalfEven, &c, &mut s);
        check(&r, s, "100", 0);
    }

    // --- subnormal / clamp -------------------------------------------------

    #[test]
    fn add_subnormal_result() {
        // python: Context(prec=5, Emin=-3, Emax=6, clamp=1).add(Decimal('1E-6'), Decimal('1E-6'))
        // -> 0.000002, Subnormal
        let mut c = ctx(5, RoundMode::HalfEven);
        c.emin = -3;
        c.emax = 6;
        c.clamp = true;
        let mut s = 0u32;
        let r = add(&d("1E-6"), &d("1E-6"), &c, &mut s);
        check(&r, s, "0.000002", status::SUBNORMAL);
    }

    #[test]
    fn mul_subnormal_result() {
        let mut c = ctx(5, RoundMode::HalfEven);
        c.emin = -3;
        c.emax = 6;
        c.clamp = true;
        let mut s = 0u32;
        let r = mul(&d("1.5E-3"), &d("1E-3"), &c, &mut s);
        check(&r, s, "0.0000015", status::SUBNORMAL);
    }

    // --- rounding modes for rescale (used throughout _rescale) ------------

    #[test]
    fn round_modes_on_half() {
        let cases: [(RoundMode, &str); 8] = [
            (RoundMode::Down, "1.2"),
            (RoundMode::Up, "1.3"),
            (RoundMode::HalfUp, "1.3"),
            (RoundMode::HalfDown, "1.2"),
            (RoundMode::HalfEven, "1.2"),
            (RoundMode::Ceiling, "1.3"),
            (RoundMode::Floor, "1.2"),
            (RoundMode::ZeroFiveUp, "1.2"),
        ];
        for (mode, expected) in cases {
            let r = rescale(&d("1.25"), -1, mode);
            check(&r, 0, expected, 0);
        }
    }

    #[test]
    fn round_modes_on_half_negative() {
        let cases: [(RoundMode, &str); 8] = [
            (RoundMode::Down, "-1.2"),
            (RoundMode::Up, "-1.3"),
            (RoundMode::HalfUp, "-1.3"),
            (RoundMode::HalfDown, "-1.2"),
            (RoundMode::HalfEven, "-1.2"),
            (RoundMode::Ceiling, "-1.2"),
            (RoundMode::Floor, "-1.3"),
            (RoundMode::ZeroFiveUp, "-1.2"),
        ];
        for (mode, expected) in cases {
            let r = rescale(&d("-1.25"), -1, mode);
            check(&r, 0, expected, 0);
        }
    }
}

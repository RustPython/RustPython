//! Digit-wise, exponent and neighbour operations.
//!
//! Ported from `Lib/_pydecimal.py`.

use crate::bigops;
use crate::context::{Context, RoundMode};
use crate::dec::Decimal;
use crate::ops;
use crate::status;
use alloc::vec::Vec;
use core::cmp::Ordering;
use malachite_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

/// Above this many digits, a logical/rotate/shift operand is treated as
/// unallocatable rather than materialized, mirroring `ops::nines`'s guard for
/// absurd context precisions.
const MAX_LOGICAL_DIGITS: i64 = 100_000_000;

fn too_wide(prec: i64, status: &mut u32) -> Option<Decimal> {
    if prec > MAX_LOGICAL_DIGITS {
        *status |= status::MALLOC_ERROR;
        Some(Decimal::nan(0, BigUint::zero(), false))
    } else {
        None
    }
}

/// `Decimal._fill_logical`: right-align a digit string to exactly `prec`
/// digits, truncating extra leading digits or padding with leading zeros.
fn pad_or_truncate(digits: &[u8], prec: i64) -> Vec<u8> {
    let len = digits.len() as i64;
    if len >= prec {
        digits[(len - prec) as usize..].to_vec()
    } else {
        let mut v = alloc::vec![b'0'; (prec - len) as usize];
        v.extend_from_slice(digits);
        v
    }
}

/// The two operands of a digit-wise logical operation, right-aligned to
/// `context.prec` digits each (`Decimal._fill_logical`).
fn fill_logical(prec: i64, a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (pad_or_truncate(a, prec), pad_or_truncate(b, prec))
}

fn digit_wise(
    a: &Decimal,
    b: &Decimal,
    ctx: &Context,
    status: &mut u32,
    combine: impl Fn(u8, u8) -> u8,
) -> Decimal {
    if !ops::is_logical(a) || !ops::is_logical(b) {
        return ops::invalid_operation(status);
    }
    if let Some(nan) = too_wide(ctx.prec, status) {
        return nan;
    }
    let (da, db) = fill_logical(ctx.prec, &a.coeff_digits(), &b.coeff_digits());
    let result: Vec<u8> = da
        .iter()
        .zip(db.iter())
        .map(|(&x, &y)| combine(x, y))
        .collect();
    let coeff = bigops::from_ascii_digits(&result);
    Decimal::new_finite(0, coeff, 0)
}

/// `Decimal.logical_and`.
pub fn logical_and(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    digit_wise(a, b, ctx, status, |x, y| {
        if x == b'1' && y == b'1' { b'1' } else { b'0' }
    })
}

/// `Decimal.logical_or`.
pub fn logical_or(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    digit_wise(a, b, ctx, status, |x, y| {
        if x == b'1' || y == b'1' { b'1' } else { b'0' }
    })
}

/// `Decimal.logical_xor`.
pub fn logical_xor(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    digit_wise(a, b, ctx, status, |x, y| if x == y { b'0' } else { b'1' })
}

/// `Decimal.logical_invert`.
pub fn logical_invert(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = too_wide(ctx.prec, status) {
        return nan;
    }
    let ones_digits = alloc::vec![b'1'; ctx.prec.max(0) as usize];
    let ones = Decimal::new_finite(0, bigops::from_ascii_digits(&ones_digits), 0);
    logical_xor(a, &ones, ctx, status)
}

/// The value of a finite, non-special `other` operand with exponent 0, as
/// used by `rotate`, `shift` and `scaleb`'s `int(other)`. `None` if the
/// magnitude does not fit `i128` at all, which is always out of the caller's
/// valid range anyway.
fn decimal_int_value(d: &Decimal) -> Option<i128> {
    let mag = i128::try_from(d.coefficient().to_u128()?).ok()?;
    Some(if d.sign() != 0 { -mag } else { mag })
}

/// `Decimal.rotate`.
pub fn rotate(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }
    if b.is_infinite() || b.exponent() != 0 {
        return ops::invalid_operation(status);
    }
    let torot = match decimal_int_value(b) {
        Some(v) if v >= -i128::from(ctx.prec) && v <= i128::from(ctx.prec) => v,
        _ => return ops::invalid_operation(status),
    };
    if a.is_infinite() {
        return a.clone();
    }
    if let Some(nan) = too_wide(ctx.prec, status) {
        return nan;
    }
    let rotdig = pad_or_truncate(&a.coeff_digits(), ctx.prec);
    let width = i128::from(ctx.prec);
    let n = (((torot % width) + width) % width) as usize;
    let mut rotated = Vec::with_capacity(rotdig.len());
    rotated.extend_from_slice(&rotdig[n..]);
    rotated.extend_from_slice(&rotdig[..n]);
    let coeff = bigops::from_ascii_digits(&rotated);
    Decimal::new_finite(a.sign(), coeff, a.exponent())
}

/// `Decimal.shift`.
pub fn shift(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }
    if b.is_infinite() || b.exponent() != 0 {
        return ops::invalid_operation(status);
    }
    let torot = match decimal_int_value(b) {
        Some(v) if v >= -i128::from(ctx.prec) && v <= i128::from(ctx.prec) => v,
        _ => return ops::invalid_operation(status),
    };
    if a.is_infinite() {
        return a.clone();
    }
    if let Some(nan) = too_wide(ctx.prec, status) {
        return nan;
    }
    let rotdig = pad_or_truncate(&a.coeff_digits(), ctx.prec);
    let width = rotdig.len();
    let shifted: Vec<u8> = if torot < 0 {
        let cut = (-torot) as usize;
        rotdig[..width - cut].to_vec()
    } else {
        let mut v = rotdig;
        v.extend(core::iter::repeat_n(b'0', torot as usize));
        let vlen = v.len();
        let prec = ctx.prec as usize;
        v[vlen - prec..].to_vec()
    };
    let coeff = if shifted.is_empty() {
        BigUint::zero()
    } else {
        bigops::from_ascii_digits(&shifted)
    };
    Decimal::new_finite(a.sign(), coeff, a.exponent())
}

/// `Decimal.scaleb`.
pub fn scaleb(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }
    if b.is_infinite() || b.exponent() != 0 {
        return ops::invalid_operation(status);
    }
    let liminf = -2i128 * (i128::from(ctx.emax) + i128::from(ctx.prec));
    let limsup = 2i128 * (i128::from(ctx.emax) + i128::from(ctx.prec));
    let n = match decimal_int_value(b) {
        Some(v) if v >= liminf && v <= limsup => v,
        _ => return ops::invalid_operation(status),
    };
    if a.is_infinite() {
        return a.clone();
    }
    let new_exp =
        (i128::from(a.exponent()) + n).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    let d = Decimal::new_finite(a.sign(), a.coefficient().clone(), new_exp);
    ops::fix(&d, ctx, status)
}

/// One signed operand of `exact_add_signed`: a coefficient at a given
/// exponent, with its own sign.
struct SignedTerm<'a> {
    sign: u8,
    exp: i64,
    coeff: &'a BigUint,
}

/// Exact signed decimal addition `a + b`, with no context applied: the worker
/// inside `Decimal.__add__` before the final `_fix`. Used instead of calling
/// into `ops::arith` (off-limits here) for the exact single-ulp step in
/// `next_plus`/`next_minus`. `round` only matters to pick the sign of an
/// exact zero, per `__add__`'s `negativezero` rule.
fn exact_add_signed(round: RoundMode, prec: i64, a: SignedTerm<'_>, b: SignedTerm<'_>) -> Decimal {
    let exp = a.exp.min(b.exp);
    let negativezero = round == RoundMode::Floor && a.sign != b.sign;
    let a_zero = a.coeff.is_zero();
    let b_zero = b.coeff.is_zero();

    if a_zero && b_zero {
        let sign = if negativezero { 1 } else { a.sign.min(b.sign) };
        return Decimal::zero(sign, exp);
    }
    if a_zero {
        let target = exp.max(b.exp.saturating_sub(prec).saturating_sub(1));
        let shift = (b.exp - target) as u64;
        return Decimal::new_finite(b.sign, bigops::mul_pow10(b.coeff, shift), target);
    }
    if b_zero {
        let target = exp.max(a.exp.saturating_sub(prec).saturating_sub(1));
        let shift = (a.exp - target) as u64;
        return Decimal::new_finite(a.sign, bigops::mul_pow10(a.coeff, shift), target);
    }

    let a_scaled = bigops::mul_pow10(a.coeff, (a.exp - exp) as u64);
    let b_scaled = bigops::mul_pow10(b.coeff, (b.exp - exp) as u64);
    if a.sign == b.sign {
        return Decimal::new_finite(a.sign, &a_scaled + &b_scaled, exp);
    }
    match a_scaled.cmp(&b_scaled) {
        Ordering::Equal => Decimal::zero(u8::from(negativezero), exp),
        Ordering::Greater => Decimal::new_finite(a.sign, &a_scaled - &b_scaled, exp),
        Ordering::Less => Decimal::new_finite(b.sign, &b_scaled - &a_scaled, exp),
    }
}

/// `Decimal.next_minus`.
pub fn next_minus(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_infinite() {
        return if a.sign() != 0 {
            Decimal::infinity(1)
        } else {
            match ops::nines(ctx.prec) {
                Some(coeff) => Decimal::new_finite(0, coeff, ctx.etop()),
                None => {
                    *status |= status::MALLOC_ERROR;
                    Decimal::nan(0, BigUint::zero(), false)
                }
            }
        };
    }

    let mut scratch_ctx = *ctx;
    scratch_ctx.round = RoundMode::Floor;
    let mut scratch_status = 0u32;
    let new_a = ops::fix(a, &scratch_ctx, &mut scratch_status);
    if ops::cmp_values(&new_a, a) != Ordering::Equal {
        return new_a;
    }
    let unit_exp = scratch_ctx.etiny() - 1;
    let unit = BigUint::one();
    let raw = exact_add_signed(
        scratch_ctx.round,
        scratch_ctx.prec,
        SignedTerm {
            sign: a.sign(),
            exp: a.exponent(),
            coeff: a.coefficient(),
        },
        SignedTerm {
            sign: 1,
            exp: unit_exp,
            coeff: &unit,
        },
    );
    ops::fix(&raw, &scratch_ctx, &mut scratch_status)
}

/// `Decimal.next_plus`.
pub fn next_plus(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_infinite() {
        return if a.sign() == 0 {
            Decimal::infinity(0)
        } else {
            match ops::nines(ctx.prec) {
                Some(coeff) => Decimal::new_finite(1, coeff, ctx.etop()),
                None => {
                    *status |= status::MALLOC_ERROR;
                    Decimal::nan(0, BigUint::zero(), false)
                }
            }
        };
    }

    let mut scratch_ctx = *ctx;
    scratch_ctx.round = RoundMode::Ceiling;
    let mut scratch_status = 0u32;
    let new_a = ops::fix(a, &scratch_ctx, &mut scratch_status);
    if ops::cmp_values(&new_a, a) != Ordering::Equal {
        return new_a;
    }
    let unit_exp = scratch_ctx.etiny() - 1;
    let unit = BigUint::one();
    let raw = exact_add_signed(
        scratch_ctx.round,
        scratch_ctx.prec,
        SignedTerm {
            sign: a.sign(),
            exp: a.exponent(),
            coeff: a.coefficient(),
        },
        SignedTerm {
            sign: 0,
            exp: unit_exp,
            coeff: &unit,
        },
    );
    ops::fix(&raw, &scratch_ctx, &mut scratch_status)
}

/// `Decimal.next_toward`.
pub fn next_toward(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }
    let comparison = ops::cmp_values(a, b);
    if comparison == Ordering::Equal {
        return super::compare::copy_sign(a, b);
    }
    let ans = if comparison == Ordering::Less {
        next_plus(a, ctx, status)
    } else {
        next_minus(a, ctx, status)
    };

    if ans.is_infinite() {
        *status |= status::OVERFLOW | status::INEXACT | status::ROUNDED;
    } else if ans.adjusted() < ctx.emin {
        *status |= status::UNDERFLOW | status::SUBNORMAL | status::INEXACT | status::ROUNDED;
        if ans.is_zero() {
            *status |= status::CLAMPED;
        }
    }
    ans
}

/// `Decimal.logb`.
pub fn logb(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_infinite() {
        return Decimal::infinity(0);
    }
    if a.is_zero() {
        *status |= status::DIVISION_BY_ZERO;
        return Decimal::infinity(1);
    }
    let ans = Decimal::from_i64(a.adjusted());
    ops::fix(&ans, ctx, status)
}

/// `Decimal.number_class`.
pub fn number_class(a: &Decimal, ctx: &Context) -> &'static str {
    if a.is_snan() {
        return "sNaN";
    }
    if a.is_qnan() {
        return "NaN";
    }
    if a.is_infinite() {
        return if a.sign() != 0 {
            "-Infinity"
        } else {
            "+Infinity"
        };
    }
    if a.is_zero() {
        return if a.sign() != 0 { "-Zero" } else { "+Zero" };
    }
    if is_subnormal(a, ctx) {
        return if a.sign() != 0 {
            "-Subnormal"
        } else {
            "+Subnormal"
        };
    }
    if a.sign() != 0 { "-Normal" } else { "+Normal" }
}

/// `Decimal.is_normal`.
pub fn is_normal(a: &Decimal, ctx: &Context) -> bool {
    if a.is_special() || a.is_zero() {
        return false;
    }
    ctx.emin <= a.adjusted()
}

/// `Decimal.is_subnormal`.
pub fn is_subnormal(a: &Decimal, ctx: &Context) -> bool {
    if a.is_special() || a.is_zero() {
        return false;
    }
    a.adjusted() < ctx.emin
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status as st;

    fn ctx() -> Context {
        Context {
            prec: 9,
            ..Context::default()
        }
    }

    fn d(s: &str) -> Decimal {
        Decimal::parse_ascii(s.as_bytes()).unwrap()
    }

    fn same(a: &Decimal, b: &Decimal) -> bool {
        a.sign() == b.sign()
            && a.special() == b.special()
            && a.coefficient() == b.coefficient()
            && (a.special() != crate::dec::Special::Finite || a.exponent() == b.exponent())
    }

    #[test]
    fn logical_and_or_xor() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).logical_and(Decimal('1100'),Decimal('1010')) == Decimal('1000')
        assert!(same(
            &logical_and(&d("1100"), &d("1010"), &c, &mut st),
            &d("1000")
        ));
        assert!(same(
            &logical_or(&d("1100"), &d("1010"), &c, &mut st),
            &d("1110")
        ));
        assert!(same(
            &logical_xor(&d("1100"), &d("1010"), &c, &mut st),
            &d("0110")
        ));
        assert_eq!(st, 0);
    }

    #[test]
    fn logical_invert_basic() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).logical_invert(Decimal('0')) == Decimal('111111111')
        assert!(same(&logical_invert(&d("0"), &c, &mut st), &d("111111111")));
        assert!(same(
            &logical_invert(&d("101"), &c, &mut st),
            &d("111111010")
        ));
    }

    #[test]
    fn logical_rejects_non_logical_operand() {
        let mut st = 0u32;
        let c = ctx();
        let r = logical_and(&d("2"), &d("1"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st & st::INVALID_OPERATION, st::INVALID_OPERATION);

        let mut st2 = 0u32;
        let r2 = logical_and(&d("NaN"), &d("1"), &c, &mut st2);
        assert!(r2.is_qnan());
        assert_eq!(st2 & st::INVALID_OPERATION, st::INVALID_OPERATION);
    }

    #[test]
    fn rotate_basic() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).rotate(Decimal('123456789'), Decimal('2')) == Decimal('345678912')
        assert!(same(
            &rotate(&d("123456789"), &d("2"), &c, &mut st),
            &d("345678912")
        ));
        // Oracle: rotate(Decimal('123456789'), Decimal('-2')) == Decimal('891234567')
        assert!(same(
            &rotate(&d("123456789"), &d("-2"), &c, &mut st),
            &d("891234567")
        ));
        assert_eq!(st, 0);
    }

    #[test]
    fn rotate_rejects_bad_other() {
        let mut st = 0u32;
        let c = ctx();
        let r = rotate(&d("1"), &d("400"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st & st::INVALID_OPERATION, st::INVALID_OPERATION);

        let mut st2 = 0u32;
        let r2 = rotate(&d("1"), &d("1.5"), &c, &mut st2);
        assert!(r2.is_qnan());
    }

    #[test]
    fn shift_basic() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).shift(Decimal('123456789'), Decimal('2')) == Decimal('345678900')
        assert!(same(
            &shift(&d("123456789"), &d("2"), &c, &mut st),
            &d("345678900")
        ));
        // Oracle: shift(Decimal('123456789'), Decimal('-2')) == Decimal('1234567')
        assert!(same(
            &shift(&d("123456789"), &d("-2"), &c, &mut st),
            &d("1234567")
        ));
        assert_eq!(st, 0);
    }

    #[test]
    fn scaleb_basic() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).scaleb(Decimal('7.50'), Decimal('-2')) == Decimal('0.0750')
        assert!(same(
            &scaleb(&d("7.50"), &d("-2"), &c, &mut st),
            &d("0.0750")
        ));
        assert_eq!(st, 0);
    }

    #[test]
    fn scaleb_out_of_range() {
        let mut st = 0u32;
        let c = ctx();
        let r = scaleb(&d("1"), &d("999999999999999999"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st & st::INVALID_OPERATION, st::INVALID_OPERATION);
    }

    #[test]
    fn next_plus_minus_basic() {
        let mut st = 0u32;
        let c = ctx();
        // Oracle: Context(prec=9).next_plus(Decimal('1')) == Decimal('1.00000001')
        assert!(same(&next_plus(&d("1"), &c, &mut st), &d("1.00000001")));
        // Oracle: Context(prec=9).next_minus(Decimal('1')) == Decimal('0.999999999')
        assert!(same(&next_minus(&d("1"), &c, &mut st), &d("0.999999999")));
    }

    #[test]
    fn next_plus_minus_infinities() {
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&next_plus(&d("Inf"), &c, &mut st), &d("Inf")));
        assert!(same(&next_minus(&d("-Inf"), &c, &mut st), &d("-Inf")));
        // Oracle: Context(prec=9).next_plus(Decimal('-Infinity')) == Decimal('-9.99999999E+999999')
        let r = next_plus(&d("-Inf"), &c, &mut st);
        assert_eq!(r.sign(), 1);
        assert!(r.is_finite());
    }

    #[test]
    fn next_toward_directional() {
        // Oracle: Context(prec=9).next_toward(Decimal('1'), Decimal('2')) == Decimal('1.00000001')
        let mut st = 0u32;
        let c = ctx();
        assert!(same(
            &next_toward(&d("1"), &d("2"), &c, &mut st),
            &d("1.00000001")
        ));
        assert!(same(
            &next_toward(&d("1"), &d("0"), &c, &mut st),
            &d("0.999999999")
        ));
        // Numerically less than self: rounds down towards other too.
        assert!(same(
            &next_toward(&d("1"), &d("-1"), &c, &mut st),
            &d("0.999999999")
        ));
        // Equal operands: result is self with other's sign.
        assert!(same(&next_toward(&d("1"), &d("1"), &c, &mut st), &d("1")));
    }

    #[test]
    fn logb_basic() {
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&logb(&d("250"), &c, &mut st), &d("2")));
        assert_eq!(st, 0);
    }

    #[test]
    fn logb_zero_signals() {
        let mut st = 0u32;
        let c = ctx();
        let r = logb(&d("0"), &c, &mut st);
        assert!(r.is_infinite());
        assert_eq!(r.sign(), 1);
        assert_eq!(st & st::DIVISION_BY_ZERO, st::DIVISION_BY_ZERO);
    }

    #[test]
    fn number_class_cases() {
        let c = ctx();
        assert_eq!(number_class(&d("sNaN"), &c), "sNaN");
        assert_eq!(number_class(&d("NaN"), &c), "NaN");
        assert_eq!(number_class(&d("Inf"), &c), "+Infinity");
        assert_eq!(number_class(&d("-Inf"), &c), "-Infinity");
        assert_eq!(number_class(&d("0"), &c), "+Zero");
        assert_eq!(number_class(&d("-0"), &c), "-Zero");
        assert_eq!(number_class(&d("1"), &c), "+Normal");
        assert_eq!(number_class(&d("-1"), &c), "-Normal");
    }

    #[test]
    fn is_normal_subnormal() {
        let c = Context {
            prec: 9,
            emin: -10,
            emax: 10,
            ..Context::default()
        };
        assert!(is_normal(&d("1"), &c));
        assert!(!is_subnormal(&d("1"), &c));
        // adjusted() of 1E-12 is -12 < Emin(-10): subnormal.
        assert!(is_subnormal(&d("1E-12"), &c));
        assert!(!is_normal(&d("1E-12"), &c));
        assert!(!is_normal(&d("0"), &c));
        assert!(!is_subnormal(&d("0"), &c));
    }
}

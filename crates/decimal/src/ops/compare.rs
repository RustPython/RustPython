//! Comparison, selection and sign-copying operations.
//!
//! Ported from `Lib/_pydecimal.py`.

use crate::context::Context;
use crate::dec::{Decimal, Special};
use crate::ops;
use core::cmp::Ordering;

/// `Decimal._compare_check_nans`, used by the signaling comparisons
/// (`compare_signal`). Signaling NaNs take precedence over quiet NaNs;
/// returns the quieted NaN result when either operand is a NaN.
fn compare_check_nans(
    a: &Decimal,
    b: &Decimal,
    ctx: &Context,
    status: &mut u32,
) -> Option<Decimal> {
    if !a.is_special() && !b.is_special() {
        return None;
    }
    if a.is_snan() {
        return Some(ops::quiet_snan(a, ctx, status));
    }
    if b.is_snan() {
        return Some(ops::quiet_snan(b, ctx, status));
    }
    if a.is_qnan() {
        return Some(ops::quiet_snan(a, ctx, status));
    }
    if b.is_qnan() {
        return Some(ops::quiet_snan(b, ctx, status));
    }
    None
}

fn ordering_to_decimal(ord: Ordering) -> Decimal {
    Decimal::from_i64(match ord {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })
}

/// 0 for a number, 1 for a quiet NaN, 2 for a signaling NaN: `Decimal._isnan`.
fn nan_rank(d: &Decimal) -> u8 {
    if d.is_qnan() {
        1
    } else if d.is_snan() {
        2
    } else {
        0
    }
}

/// `Decimal.compare`.
pub fn compare(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }
    ordering_to_decimal(ops::cmp_values(a, b))
}

/// `Decimal.compare_signal`.
pub fn compare_signal(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = compare_check_nans(a, b, ctx, status) {
        return nan;
    }
    compare(a, b, ctx, status)
}

/// `Decimal.compare_total`, as an ordering.
pub fn compare_total(a: &Decimal, b: &Decimal) -> Ordering {
    // If one is negative and the other positive, that settles it.
    if a.sign() != 0 && b.sign() == 0 {
        return Ordering::Less;
    }
    if a.sign() == 0 && b.sign() != 0 {
        return Ordering::Greater;
    }
    let sign = a.sign();

    let a_nan = nan_rank(a);
    let b_nan = nan_rank(b);
    if a_nan != 0 || b_nan != 0 {
        if a_nan == b_nan {
            // Compare payloads as though they were integers: (length, digits).
            let a_key = (a.digits(), a.coefficient());
            let b_key = (b.digits(), b.coefficient());
            return match a_key.cmp(&b_key) {
                Ordering::Less => {
                    if sign != 0 {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                Ordering::Greater => {
                    if sign != 0 {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                Ordering::Equal => Ordering::Equal,
            };
        }
        if sign != 0 {
            if a_nan == 1 {
                return Ordering::Less;
            }
            if b_nan == 1 {
                return Ordering::Greater;
            }
            if a_nan == 2 {
                return Ordering::Less;
            }
            if b_nan == 2 {
                return Ordering::Greater;
            }
        } else {
            if a_nan == 1 {
                return Ordering::Greater;
            }
            if b_nan == 1 {
                return Ordering::Less;
            }
            if a_nan == 2 {
                return Ordering::Greater;
            }
            if b_nan == 2 {
                return Ordering::Less;
            }
        }
    }

    let ord = ops::cmp_values(a, b);
    if ord != Ordering::Equal {
        return ord;
    }

    if a.exponent() < b.exponent() {
        return if sign != 0 {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if a.exponent() > b.exponent() {
        return if sign != 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    Ordering::Equal
}

/// `Decimal.compare_total_mag`, as an ordering.
pub fn compare_total_mag(a: &Decimal, b: &Decimal) -> Ordering {
    compare_total(&copy_abs(a), &copy_abs(b))
}

/// `Decimal.max`.
pub fn max(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() || b.is_special() {
        let sn = nan_rank(a);
        let on = nan_rank(b);
        if sn != 0 || on != 0 {
            if on == 1 && sn == 0 {
                return ops::fix(a, ctx, status);
            }
            if sn == 1 && on == 0 {
                return ops::fix(b, ctx, status);
            }
            return ops::check_nans(a, Some(b), ctx, status).expect("one operand is a NaN");
        }
    }
    let mut c = ops::cmp_values(a, b);
    if c == Ordering::Equal {
        c = compare_total(a, b);
    }
    let ans = if c == Ordering::Less { b } else { a };
    ops::fix(ans, ctx, status)
}

/// `Decimal.min`.
pub fn min(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() || b.is_special() {
        let sn = nan_rank(a);
        let on = nan_rank(b);
        if sn != 0 || on != 0 {
            if on == 1 && sn == 0 {
                return ops::fix(a, ctx, status);
            }
            if sn == 1 && on == 0 {
                return ops::fix(b, ctx, status);
            }
            return ops::check_nans(a, Some(b), ctx, status).expect("one operand is a NaN");
        }
    }
    let mut c = ops::cmp_values(a, b);
    if c == Ordering::Equal {
        c = compare_total(a, b);
    }
    let ans = if c == Ordering::Less { a } else { b };
    ops::fix(ans, ctx, status)
}

/// `Decimal.max_mag`.
pub fn max_mag(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() || b.is_special() {
        let sn = nan_rank(a);
        let on = nan_rank(b);
        if sn != 0 || on != 0 {
            if on == 1 && sn == 0 {
                return ops::fix(a, ctx, status);
            }
            if sn == 1 && on == 0 {
                return ops::fix(b, ctx, status);
            }
            return ops::check_nans(a, Some(b), ctx, status).expect("one operand is a NaN");
        }
    }
    let mut c = ops::cmp_values(&copy_abs(a), &copy_abs(b));
    if c == Ordering::Equal {
        c = compare_total(a, b);
    }
    let ans = if c == Ordering::Less { b } else { a };
    ops::fix(ans, ctx, status)
}

/// `Decimal.min_mag`.
pub fn min_mag(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() || b.is_special() {
        let sn = nan_rank(a);
        let on = nan_rank(b);
        if sn != 0 || on != 0 {
            if on == 1 && sn == 0 {
                return ops::fix(a, ctx, status);
            }
            if sn == 1 && on == 0 {
                return ops::fix(b, ctx, status);
            }
            return ops::check_nans(a, Some(b), ctx, status).expect("one operand is a NaN");
        }
    }
    let mut c = ops::cmp_values(&copy_abs(a), &copy_abs(b));
    if c == Ordering::Equal {
        c = compare_total(a, b);
    }
    let ans = if c == Ordering::Less { a } else { b };
    ops::fix(ans, ctx, status)
}

/// `Decimal.same_quantum`.
pub fn same_quantum(a: &Decimal, b: &Decimal) -> bool {
    if a.is_special() || b.is_special() {
        return (a.is_nan() && b.is_nan()) || (a.is_infinite() && b.is_infinite());
    }
    a.exponent() == b.exponent()
}

/// Rebuilds `d` with a different sign, keeping its special kind, coefficient
/// (or NaN payload) and exponent untouched. Shared by `copy_abs`,
/// `copy_negate` and `copy_sign`.
fn with_sign(d: &Decimal, sign: u8) -> Decimal {
    match d.special() {
        Special::Finite => {
            Decimal::from_triple(sign, d.coefficient().clone(), d.digits(), d.exponent())
        }
        Special::Inf => Decimal::infinity(sign),
        Special::Nan => Decimal::nan(sign, d.coefficient().clone(), false),
        Special::Snan => Decimal::nan(sign, d.coefficient().clone(), true),
    }
}

/// `Decimal.copy_abs`.
pub fn copy_abs(a: &Decimal) -> Decimal {
    with_sign(a, 0)
}

/// `Decimal.copy_negate`.
pub fn copy_negate(a: &Decimal) -> Decimal {
    with_sign(a, 1 - a.sign())
}

/// `Decimal.copy_sign`.
pub fn copy_sign(a: &Decimal, b: &Decimal) -> Decimal {
    with_sign(a, b.sign())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bigops;
    use crate::status as st;
    use malachite_bigint::BigUint;

    fn ctx() -> Context {
        Context {
            prec: 9,
            ..Context::default()
        }
    }

    fn d(s: &str) -> Decimal {
        Decimal::parse_ascii(s.as_bytes()).unwrap()
    }

    /// Structural equality: `Decimal` has no `PartialEq` (NaN payloads and
    /// signed zeros make the natural relation non-total), so tests compare
    /// the sign/special-kind/coefficient/exponent tuple directly instead of
    /// numeric value.
    fn same(a: &Decimal, b: &Decimal) -> bool {
        a.sign() == b.sign()
            && a.special() == b.special()
            && a.coefficient() == b.coefficient()
            && (a.special() != Special::Finite || a.exponent() == b.exponent())
    }

    #[test]
    fn compare_basic() {
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&compare(&d("1"), &d("2"), &c, &mut st), &d("-1")));
        assert!(same(&compare(&d("2"), &d("1"), &c, &mut st), &d("1")));
        assert!(same(&compare(&d("2"), &d("2.0"), &c, &mut st), &d("0")));
        assert_eq!(st, 0);
    }

    #[test]
    fn compare_snan_signals() {
        let mut st = 0u32;
        let c = ctx();
        let r = compare(&d("sNaN"), &d("1"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st & st::INVALID_OPERATION, st::INVALID_OPERATION);
    }

    #[test]
    fn compare_qnan_no_flag() {
        let mut st = 0u32;
        let c = ctx();
        let r = compare(&d("NaN"), &d("1"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st, 0);
    }

    #[test]
    fn compare_signal_qnan_signals() {
        let mut st = 0u32;
        let c = ctx();
        let r = compare_signal(&d("NaN"), &d("1"), &c, &mut st);
        assert!(r.is_qnan());
        assert_eq!(st & st::INVALID_OPERATION, st::INVALID_OPERATION);
    }

    #[test]
    fn compare_total_sign_first() {
        assert_eq!(compare_total(&d("-1"), &d("1")), Ordering::Less);
        assert_eq!(compare_total(&d("1"), &d("-1")), Ordering::Greater);
    }

    #[test]
    fn compare_total_exponent_tiebreak() {
        // Oracle: Decimal('1.0').compare_total(Decimal('1')) == -1
        assert_eq!(compare_total(&d("1.0"), &d("1")), Ordering::Less);
        assert_eq!(compare_total(&d("1"), &d("1.0")), Ordering::Greater);
        assert_eq!(compare_total(&d("-1.0"), &d("-1")), Ordering::Greater);
    }

    #[test]
    fn compare_total_nan_payload_order() {
        // Oracle: Decimal('NaN1').compare_total(Decimal('NaN2')) == -1
        assert_eq!(compare_total(&d("NaN1"), &d("NaN2")), Ordering::Less);
        assert_eq!(compare_total(&d("NaN10"), &d("NaN2")), Ordering::Greater);
        assert_eq!(compare_total(&d("-NaN1"), &d("-NaN2")), Ordering::Greater);
    }

    #[test]
    fn compare_total_nan_kind_order() {
        // Oracle: Decimal('-sNaN').compare_total(Decimal('-NaN')) == 1
        // Oracle: Decimal('sNaN').compare_total(Decimal('NaN')) == -1
        // Oracle: Decimal('-Inf').compare_total(Decimal('-NaN')) == 1
        assert_eq!(compare_total(&d("-sNaN"), &d("-NaN")), Ordering::Greater);
        assert_eq!(compare_total(&d("sNaN"), &d("NaN")), Ordering::Less);
        assert_eq!(compare_total(&d("-Inf"), &d("-NaN")), Ordering::Greater);
    }

    #[test]
    fn compare_total_mag_ignores_sign() {
        assert_eq!(compare_total_mag(&d("-1"), &d("1")), Ordering::Equal);
        assert_eq!(compare_total_mag(&d("-1.0"), &d("1")), Ordering::Less);
    }

    #[test]
    fn min_max_pick_nonnan() {
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&max(&d("NaN"), &d("1"), &c, &mut st), &d("1")));
        assert!(same(&min(&d("1"), &d("NaN"), &c, &mut st), &d("1")));
    }

    #[test]
    fn max_uses_compare_total_tiebreak() {
        // Oracle: Context(prec=9).max(Decimal('1.0'), Decimal('1')) == Decimal('1')
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&max(&d("1.0"), &d("1"), &c, &mut st), &d("1")));
        assert!(same(&min(&d("1.0"), &d("1"), &c, &mut st), &d("1.0")));
    }

    #[test]
    fn max_mag_min_mag() {
        let mut st = 0u32;
        let c = ctx();
        assert!(same(&max_mag(&d("-3"), &d("2"), &c, &mut st), &d("-3")));
        assert!(same(&min_mag(&d("-3"), &d("2"), &c, &mut st), &d("2")));
    }

    #[test]
    fn same_quantum_cases() {
        assert!(same_quantum(&d("1.00"), &d("2.00")));
        assert!(!same_quantum(&d("1.0"), &d("1.00")));
        assert!(same_quantum(&d("NaN"), &d("NaN")));
        assert!(same_quantum(&d("Inf"), &d("-Inf")));
        assert!(!same_quantum(&d("Inf"), &d("NaN")));
    }

    #[test]
    fn copy_ops() {
        assert!(same(&copy_abs(&d("-2.5")), &d("2.5")));
        assert!(same(&copy_negate(&d("2.5")), &d("-2.5")));
        assert!(same(&copy_negate(&d("-2.5")), &d("2.5")));
        assert!(same(&copy_sign(&d("2.5"), &d("-1")), &d("-2.5")));
        assert!(same(&copy_sign(&d("-2.5"), &d("1")), &d("2.5")));
    }

    #[test]
    fn copy_abs_preserves_special() {
        assert!(same(&copy_abs(&d("-Inf")), &d("Inf")));
        let n = copy_abs(&d("-NaN123"));
        assert!(n.is_qnan());
        assert_eq!(n.coefficient(), &bigops::from_ascii_digits(b"123"));
        assert_eq!(n.sign(), 0);
    }

    #[test]
    fn copy_sign_ignores_snan_ness() {
        let r = copy_sign(&d("sNaN5"), &d("-1"));
        assert!(r.is_snan());
        assert_eq!(r.sign(), 1);
        assert_eq!(r.coefficient(), &BigUint::from(5u32));
    }
}

//! Correctly rounded square root, exponential, logarithms and power.
//!
//! Ported from `Lib/_pydecimal.py`, whose algorithms are the reference for the
//! results the specification leaves to the implementation. This engine never
//! raises: where `_pydecimal` calls `context._raise_error(Cond, msg)` this
//! module sets the matching `status` bit and returns the value
//! `_raise_error` would have produced (see the module-level porting table).
//!
//! Internal integer arithmetic is carried out on `BigInt`/`BigUint` from
//! `malachite_bigint`, mirroring `_pydecimal`'s use of unbounded Python
//! integers. Exponent-scale quantities use `i128` so that arithmetic on
//! context exponents (up to ±2e18) and precisions (up to 1e18) cannot
//! silently wrap in `i64`.

use crate::context::{Context, RoundMode};
use crate::dec::{Decimal, Special};
use crate::status;
use crate::{bigops, ops};
use alloc::string::String;
use alloc::string::ToString;
use core::cell::RefCell;
use core::cmp::Ordering;
use malachite_bigint::{BigInt, BigUint};
use num_integer::Integer as _;
use num_traits::{One, Signed, ToPrimitive, Zero};

// ---------------------------------------------------------------------
// Small numeric helpers shared by every algorithm below.
// ---------------------------------------------------------------------

/// Above this many digits a working precision is treated as unallocatable,
/// mirroring `ops::nines`'s guard against absurd context precisions.
const MAX_WORK_DIGITS: i128 = 100_000_000;

/// A quiet NaN tagged with `MALLOC_ERROR`, for a working precision that would
/// require materialising an unreasonable number of digits.
fn malloc_error_nan(status: &mut u32) -> Decimal {
    *status |= status::MALLOC_ERROR;
    Decimal::nan(0, BigUint::zero(), false)
}

/// Guards the working precision `p` used by the Newton/series loops below.
fn work_prec_guard(p: i128, status: &mut u32) -> Option<Decimal> {
    if !(0..=MAX_WORK_DIGITS).contains(&p) {
        Some(malloc_error_nan(status))
    } else {
        None
    }
}

/// `len(str(n))` for a signed integer: the number of decimal digits in `n`'s
/// magnitude.
fn intlen(n: &BigInt) -> i128 {
    bigops::digit_count(n.magnitude()) as i128
}

/// `len(str(n))` for a nonnegative `i128`.
fn digit_len_pos(n: i128) -> i128 {
    debug_assert!(n >= 0);
    i128::from(bigops::digit_count_u128(n as u128))
}

/// Ceiling division for positive `a`, `b`.
fn ceil_div(a: i128, b: i128) -> i128 {
    debug_assert!(a >= 0 && b > 0);
    (a + b - 1) / b
}

/// `10**e` as a `BigInt`, for `e >= 0`.
fn pow10_bi(e: i128) -> BigInt {
    debug_assert!(e >= 0);
    BigInt::from(bigops::pow10(e as u64))
}

/// `int(n)` truncated to `i64`, saturating instead of overflowing. Used only
/// where the caller has already established the value lies in a
/// representable range; saturation on the rare out-of-range case is a safe
/// fallback since the surrounding `_fix` will report Overflow/Underflow.
fn i128_to_i64_sat(n: i128) -> i64 {
    n.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// `Decimal._cmp`-equivalent equality test against the exact value 1.
fn equals_one(d: &Decimal) -> bool {
    !d.is_special() && ops::cmp_values(d, &Decimal::one()) == Ordering::Equal
}

/// `copy_negate`: flips the sign, preserving everything else.
fn copy_negate(d: &Decimal) -> Decimal {
    let sign = 1 - d.sign();
    match d.special() {
        Special::Finite => Decimal::new_finite(sign, d.coefficient().clone(), d.exponent()),
        Special::Inf => Decimal::infinity(sign),
        Special::Nan | Special::Snan => Decimal::nan(sign, d.coefficient().clone(), d.is_snan()),
    }
}

/// The exact integer value of an integer-valued, nonnegative `Decimal`, if it
/// fits in an `i128`.
fn integer_value_i128(d: &Decimal) -> Option<i128> {
    debug_assert!(d.is_integer_valued());
    let mag = if d.exponent() >= 0 {
        bigops::mul_pow10(d.coefficient(), d.exponent() as u64)
    } else {
        bigops::div_pow10(d.coefficient(), d.exponent().unsigned_abs())
    };
    mag.to_i128()
}

// ---------------------------------------------------------------------
// `_pydecimal`'s private integer-arithmetic helpers (`~5655-5995`).
// ---------------------------------------------------------------------

/// `_decimal_lshift_exact`: `n * 10**e` if that is an integer, else `None`.
fn decimal_lshift_exact(n: &BigInt, e: i128) -> Option<BigInt> {
    if n.is_zero() {
        return Some(BigInt::zero());
    }
    if e >= 0 {
        return Some(n * pow10_bi(e));
    }
    let val_n = bigops::trailing_zeros10(n.magnitude()).expect("n is nonzero") as i128;
    if val_n < -e {
        None
    } else {
        Some(n / pow10_bi(-e))
    }
}

/// `_sqrt_nearest`: the integer closest to `sqrt(n)`, Newton's method seeded
/// from `a`. Both arguments must be positive.
fn sqrt_nearest(n: &BigInt, a0: &BigInt) -> BigInt {
    debug_assert!(n.is_positive() && a0.is_positive());
    let mut a = a0.clone();
    let mut b = BigInt::zero();
    while a != b {
        b = a.clone();
        // a - (-n)//a >> 1, i.e. (a + ceil(n/a)) // 2.
        let neg_n = -n;
        let ceil_term = neg_n.div_floor(&a);
        a = (&a - &ceil_term).div_floor(&BigInt::from(2));
    }
    a
}

/// `_rshift_nearest`: closest integer to `x / 2**shift`, ties to even.
fn rshift_nearest(x: &BigInt, shift: i128) -> BigInt {
    debug_assert!(shift >= 0);
    let two_pow = BigInt::from(2).pow(shift as u32);
    let q = x.div_floor(&two_pow);
    let r = x.mod_floor(&two_pow);
    let q_odd = q.mod_floor(&BigInt::from(2));
    if &(&r * 2) + &q_odd > two_pow {
        q + 1
    } else {
        q
    }
}

/// `_div_nearest`: closest integer to `a / b`, ties to even. `b` must be
/// positive.
fn div_nearest(a: &BigInt, b: &BigInt) -> BigInt {
    debug_assert!(b.is_positive());
    let q = a.div_floor(b);
    let r = a.mod_floor(b);
    let q_odd = q.mod_floor(&BigInt::from(2));
    if &(&r * 2) + &q_odd > *b { q + 1 } else { q }
}

/// `_ilog`: integer approximation to `M*log(x/M)`. `x` and `M` must be
/// positive.
fn ilog(x: &BigInt, m: &BigInt) -> BigInt {
    const L: i128 = 8;
    let mut y = x - m;
    let mut r: i128 = 0;
    loop {
        let abs_y = y.abs();
        let cond = if r <= L {
            (&abs_y << (L - r) as u32) >= *m
        } else {
            (&abs_y >> (r - L) as u32) >= *m
        };
        if !cond {
            break;
        }
        let shifted = rshift_nearest(&y, r);
        let inner = m + shifted;
        let sq = sqrt_nearest(&(m * &inner), m);
        y = div_nearest(&((m * &y) * 2), &(m + &sq));
        r += 1;
    }

    let m_len = intlen(m);
    let t = ceil_div(10 * m_len, 3 * L);
    let yshift = rshift_nearest(&y, r);
    let mut w = div_nearest(m, &BigInt::from(t));
    let mut k = t - 1;
    while k > 0 {
        w = div_nearest(m, &BigInt::from(k)) - div_nearest(&(&yshift * &w), m);
        k -= 1;
    }
    div_nearest(&(&w * &y), m)
}

thread_local! {
    /// Digits of `log(10) = 2.302585...`, grown on demand. Mirrors
    /// `_Log10Memoize`.
    static LOG10_DIGITS: RefCell<String> =
        RefCell::new("23025850929940456840179914546843642076011014886".to_string());
}

/// `_Log10Memoize.getdigits`: `floor(10**p * log(10))`.
fn log10_digits(p: i128) -> BigInt {
    debug_assert!(p >= 0);
    LOG10_DIGITS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if (p as usize) >= cache.len() {
            let mut extra: i128 = 3;
            loop {
                let m = pow10_bi(p + extra + 2);
                let ten_m = &m * 10;
                let val = div_nearest(&ilog(&ten_m, &m), &BigInt::from(100));
                let digits = val.to_str_radix(10);
                let tail_start = digits.len() - extra as usize;
                if digits.as_bytes()[tail_start..].iter().any(|&b| b != b'0') {
                    let trimmed = digits.trim_end_matches('0');
                    *cache = trimmed[..trimmed.len() - 1].to_string();
                    break;
                }
                extra += 3;
            }
        }
        let s = &cache[..=(p as usize)];
        BigInt::parse_bytes(s.as_bytes(), 10).expect("digit string")
    })
}

/// `_dlog10`: integer approximation to `10**p * log10(c*10**e)`. `c` must be
/// positive.
fn dlog10(c: &BigInt, e: i128, p: i128) -> BigInt {
    let p = p + 2;
    let l = intlen(c);
    let f = e + l - i128::from(e + l >= 1);

    let (log_d, log_tenpower) = if p > 0 {
        let m = pow10_bi(p);
        let k = e + p - f;
        let c = if k >= 0 {
            c * pow10_bi(k)
        } else {
            div_nearest(c, &pow10_bi(-k))
        };
        let log_d = ilog(&c, &m);
        let log_10 = log10_digits(p);
        let log_d = div_nearest(&(&log_d * &m), &log_10);
        (log_d, BigInt::from(f) * m)
    } else {
        (BigInt::zero(), div_nearest(&BigInt::from(f), &pow10_bi(-p)))
    };

    div_nearest(&(log_tenpower + log_d), &BigInt::from(100))
}

/// `_dlog`: integer approximation to `10**p * log(c*10**e)`. `c` must be
/// positive.
fn dlog(c: &BigInt, e: i128, p: i128) -> BigInt {
    let p = p + 2;
    let l = intlen(c);
    let f = e + l - i128::from(e + l >= 1);

    let log_d = if p > 0 {
        let k = e + p - f;
        let c = if k >= 0 {
            c * pow10_bi(k)
        } else {
            div_nearest(c, &pow10_bi(-k))
        };
        ilog(&c, &pow10_bi(p))
    } else {
        BigInt::zero()
    };

    let f_log_ten = if f != 0 {
        let extra = digit_len_pos(f.unsigned_abs() as i128) - 1;
        if p + extra >= 0 {
            div_nearest(
                &(BigInt::from(f) * log10_digits(p + extra)),
                &pow10_bi(extra),
            )
        } else {
            BigInt::zero()
        }
    } else {
        BigInt::zero()
    };

    div_nearest(&(f_log_ten + log_d), &BigInt::from(100))
}

/// `_iexp`: integer approximation to `M*exp(x/M)`. `x` and `M` must be
/// positive, with `x/M` small.
fn iexp(x: &BigInt, m: &BigInt) -> BigInt {
    const L: i128 = 8;
    let r = (x << (L as u32)).div_floor(m).magnitude().bits() as i128;
    let m_len = intlen(m);
    let t = ceil_div(10 * m_len, 3 * L);

    let mut y = div_nearest(x, &BigInt::from(t));
    let mshift = m << (r as u32);
    let mut k = t - 1;
    while k > 0 {
        y = div_nearest(&(x * (&mshift + &y)), &(&mshift * k));
        k -= 1;
    }

    let mut k = r - 1;
    while k >= 0 {
        let mshift = m << ((k + 2) as u32);
        y = div_nearest(&(&y * (&y + &mshift)), &mshift);
        k -= 1;
    }
    m + y
}

/// `_dexp`: `(d, f)` such that `d*10**f` approximates `exp(c*10**e)` to `p`
/// digits, with error in `d` of at most 1.
fn dexp(c: &BigInt, e: i128, p: i128) -> (BigInt, i128) {
    let p = p + 2;
    let extra = 0.max(e + intlen(c) - 1);
    let q = p + extra;
    let shift = e + q;
    let cshift = if shift >= 0 {
        c * pow10_bi(shift)
    } else {
        c.div_floor(&pow10_bi(-shift))
    };
    let log10 = log10_digits(q);
    let quot = cshift.div_floor(&log10);
    let rem = cshift.mod_floor(&log10);
    let rem = div_nearest(&rem, &pow10_bi(extra));

    let coeff = div_nearest(&iexp(&rem, &pow10_bi(p)), &BigInt::from(1000));
    (coeff, quot.to_i128().expect("bounded quotient") - p + 3)
}

/// `_dpower`: `(c, e)` such that `c*10**e` approximates `x**y` to `p` digits,
/// where `x = xc*10**xe` is positive and not 1, and `y = yc*10**ye` (`yc`
/// carrying `y`'s sign) is nonzero.
fn dpower(xc: &BigInt, xe: i128, yc: &BigInt, ye: i128, p: i128) -> (BigInt, i128) {
    let b = intlen(yc) + ye;
    let lxc = dlog(xc, xe, p + b + 1);
    let shift = ye - b;
    let pc = if shift >= 0 {
        &lxc * yc * pow10_bi(shift)
    } else {
        div_nearest(&(&lxc * yc), &pow10_bi(-shift))
    };

    if pc.is_zero() {
        let x_gt_one = intlen(xc) + xe >= 1;
        let y_gt_zero = yc.is_positive();
        if x_gt_one == y_gt_zero {
            (pow10_bi(p - 1) + 1, 1 - p)
        } else {
            (pow10_bi(p) - 1, -p)
        }
    } else {
        let (coeff, exp) = dexp(&pc, -(p + 1), p + 1);
        (div_nearest(&coeff, &BigInt::from(10)), exp + 1)
    }
}

/// `_log10_lb`: a lower bound for `100*log10(c)`, `c` a positive integer.
fn log10_lb(c: &BigUint) -> i128 {
    const CORRECTION: [i128; 10] = [0, 100, 70, 53, 40, 31, 23, 16, 10, 5];
    debug_assert!(!c.is_zero());
    let len = bigops::digit_count(c) as i128;
    let first = bigops::to_digits(c)[0];
    100 * len - CORRECTION[(first - b'0') as usize]
}

// ---------------------------------------------------------------------
// `Decimal._ln_exp_bound` / `Decimal._log10_exp_bound` (~3132, ~3207).
// ---------------------------------------------------------------------

/// `Decimal._ln_exp_bound`. `a` must be finite, positive and not equal to 1.
fn ln_exp_bound(a: &Decimal) -> i128 {
    let adj = i128::from(a.adjusted());
    if adj >= 1 {
        return digit_len_pos(adj * 23 / 10) - 1;
    }
    if adj <= -2 {
        return digit_len_pos((-1 - adj) * 23 / 10) - 1;
    }
    let c = BigInt::from(a.coefficient().clone());
    let e = i128::from(a.exponent());
    if adj == 0 {
        let num = (&c - pow10_bi(-e)).to_str_radix(10);
        let den = c.to_str_radix(10);
        let lt = i128::from(num < den);
        return num.len() as i128 - den.len() as i128 - lt;
    }
    // adj == -1
    let num = (pow10_bi(-e) - &c).to_str_radix(10);
    e + num.len() as i128 - 1
}

/// `Decimal._log10_exp_bound`. `a` must be finite, positive and not equal to
/// 1.
fn log10_exp_bound(a: &Decimal) -> i128 {
    let adj = i128::from(a.adjusted());
    if adj >= 1 {
        return digit_len_pos(adj) - 1;
    }
    if adj <= -2 {
        return digit_len_pos(-1 - adj) - 1;
    }
    let c = BigInt::from(a.coefficient().clone());
    let e = i128::from(a.exponent());
    if adj == 0 {
        let num = (&c - pow10_bi(-e)).to_str_radix(10);
        let den = (&c * BigInt::from(231)).to_str_radix(10);
        let lt = i128::from(num < den);
        return num.len() as i128 - den.len() as i128 - lt + 2;
    }
    // adj == -1
    let num = (pow10_bi(-e) - &c).to_str_radix(10);
    let lt = i128::from(num.as_str() < "231");
    num.len() as i128 + e - lt - 1
}

// ---------------------------------------------------------------------
// Public transcendental operations.
// ---------------------------------------------------------------------

/// `Decimal.sqrt`.
pub fn sqrt(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if a.is_special() {
        if let Some(nan) = ops::check_nans(a, None, ctx, status) {
            return nan;
        }
        if a.is_infinite() && a.sign() == 0 {
            return a.clone();
        }
    }

    if a.is_zero() {
        let exp = num_integer::Integer::div_floor(&i128::from(a.exponent()), &2);
        let ans = Decimal::zero(a.sign(), i128_to_i64_sat(exp));
        return ops::fix(&ans, ctx, status);
    }

    if a.sign() == 1 {
        return ops::invalid_operation(status);
    }

    let prec = i128::from(ctx.prec) + 1;
    if let Some(nan) = work_prec_guard(prec, status) {
        return nan;
    }

    let mut e = num_integer::Integer::div_floor(&i128::from(a.exponent()), &2);
    let odd = a.exponent() & 1 != 0;
    let mut c = if odd {
        BigUint::from(10u32) * a.coefficient()
    } else {
        a.coefficient().clone()
    };
    let l = if odd {
        (a.digits() as i128 >> 1) + 1
    } else {
        (a.digits() as i128 + 1) >> 1
    };

    let shift = prec - l;
    let mut exact = if shift >= 0 {
        c = bigops::mul_pow10(&c, 2 * shift as u64);
        true
    } else {
        let (q, r) = bigops::split_pow10(&c, 2 * (-shift) as u64);
        c = q;
        r.is_zero()
    };
    e -= shift;

    let mut n = bigops::pow10(prec as u64);
    loop {
        let q = &c / &n;
        if n <= q {
            break;
        }
        n = (&n + &q) >> 1u32;
    }
    exact = exact && &n * &n == c;

    if exact {
        if shift >= 0 {
            n = bigops::div_pow10(&n, shift as u64);
        } else {
            n = bigops::mul_pow10(&n, (-shift) as u64);
        }
        e += shift;
    } else if (&n % BigUint::from(5u32)).is_zero() {
        n += BigUint::one();
    }

    let ans = Decimal::new_finite(0, n, i128_to_i64_sat(e));

    let mut ctx2 = *ctx;
    ctx2.round = RoundMode::HalfEven;
    ops::fix(&ans, &ctx2, status)
}

/// `Decimal.exp`.
pub fn exp(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_infinite() {
        return if a.sign() == 1 {
            Decimal::zero(0, 0)
        } else {
            a.clone()
        };
    }
    if a.is_zero() {
        return Decimal::one();
    }

    let p = i128::from(ctx.prec);
    if let Some(nan) = work_prec_guard(p, status) {
        return nan;
    }
    let adj = i128::from(a.adjusted());

    let ans = if a.sign() == 0 && adj > digit_len_pos((i128::from(ctx.emax) + 1) * 3) {
        Decimal::new_finite(0, BigUint::one(), ctx.emax + 1)
    } else if a.sign() == 1 && adj > digit_len_pos((-i128::from(ctx.etiny()) + 1) * 3) {
        Decimal::new_finite(0, BigUint::one(), ctx.etiny() - 1)
    } else if a.sign() == 0 && adj < -p {
        // 1 followed by (p-1) zeros then a 1: 10**p + 1.
        let coeff = bigops::pow10(p as u64) + BigUint::one();
        Decimal::new_finite(0, coeff, i128_to_i64_sat(-p))
    } else if a.sign() == 1 && adj < -p - 1 {
        let coeff = ops::nines(i128_to_i64_sat(p + 1)).unwrap_or_else(BigUint::zero);
        Decimal::new_finite(0, coeff, i128_to_i64_sat(-p - 1))
    } else {
        let e = i128::from(a.exponent());
        let mut c = BigInt::from(a.coefficient().clone());
        if a.sign() == 1 {
            c = -c;
        }
        let mut extra: i128 = 3;
        let (coeff, exp) = loop {
            let (coeff, exp) = dexp(&c, e, p + extra);
            let clen = intlen(&coeff);
            let modulus = BigInt::from(5) * pow10_bi(clen - p - 1);
            if !(&coeff % &modulus).is_zero() {
                break (coeff, exp);
            }
            extra += 3;
        };
        Decimal::new_finite(
            0,
            coeff.to_biguint().expect("dexp coefficient is positive"),
            i128_to_i64_sat(exp),
        )
    };

    let mut ctx2 = *ctx;
    ctx2.round = RoundMode::HalfEven;
    ops::fix(&ans, &ctx2, status)
}

/// `Decimal.ln`.
pub fn ln(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_zero() {
        return Decimal::infinity(1);
    }
    if a.is_infinite() && a.sign() == 0 {
        return Decimal::infinity(0);
    }
    if equals_one(a) {
        return Decimal::zero(0, 0);
    }
    if a.sign() == 1 {
        return ops::invalid_operation(status);
    }

    let p = i128::from(ctx.prec);
    if let Some(nan) = work_prec_guard(p, status) {
        return nan;
    }
    let c = BigInt::from(a.coefficient().clone());
    let e = i128::from(a.exponent());

    let mut places = p - ln_exp_bound(a) + 2;
    let coeff = loop {
        let coeff = dlog(&c, e, places);
        let clen = intlen(&coeff);
        let modulus = BigInt::from(5) * pow10_bi(clen - p - 1);
        if !coeff.mod_floor(&modulus).is_zero() {
            break coeff;
        }
        places += 3;
    };

    let sign = u8::from(coeff.is_negative());
    let ans = Decimal::new_finite(sign, coeff.magnitude().clone(), i128_to_i64_sat(-places));

    let mut ctx2 = *ctx;
    ctx2.round = RoundMode::HalfEven;
    ops::fix(&ans, &ctx2, status)
}

/// `Decimal.log10`.
pub fn log10(a: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, None, ctx, status) {
        return nan;
    }
    if a.is_zero() {
        return Decimal::infinity(1);
    }
    if a.is_infinite() && a.sign() == 0 {
        return Decimal::infinity(0);
    }
    if a.sign() == 1 {
        return ops::invalid_operation(status);
    }

    let p = i128::from(ctx.prec);
    if let Some(nan) = work_prec_guard(p, status) {
        return nan;
    }

    let digits = bigops::to_digits(a.coefficient());
    let is_pow10 = digits[0] == b'1' && digits[1..].iter().all(|&d| d == b'0');

    let ans = if is_pow10 {
        let n = i128::from(a.exponent()) + a.digits() as i128 - 1;
        let sign = u8::from(n < 0);
        Decimal::new_finite(sign, BigUint::from(n.unsigned_abs()), 0)
    } else {
        let c = BigInt::from(a.coefficient().clone());
        let e = i128::from(a.exponent());
        let mut places = p - log10_exp_bound(a) + 2;
        let coeff = loop {
            let coeff = dlog10(&c, e, places);
            let clen = intlen(&coeff);
            let modulus = BigInt::from(5) * pow10_bi(clen - p - 1);
            if !coeff.mod_floor(&modulus).is_zero() {
                break coeff;
            }
            places += 3;
        };
        let sign = u8::from(coeff.is_negative());
        Decimal::new_finite(sign, coeff.magnitude().clone(), i128_to_i64_sat(-places))
    };

    let mut ctx2 = *ctx;
    ctx2.round = RoundMode::HalfEven;
    ops::fix(&ans, &ctx2, status)
}

/// `Decimal._power_exact`. `a` must be finite, positive and not numerically
/// equal to 1; `b` must be finite and nonzero. Returns `None` when `a**b` is
/// not exactly representable in `p` digits.
fn power_exact(a: &Decimal, b: &Decimal, p: i128) -> Option<Decimal> {
    // `p` is only ever used as "how many digits may the result have", and the
    // caller may pass the unbounded context's precision. Clamp it to what can
    // actually be held in memory so the size checks below still bite; a result
    // needing more digits than that is left to the caller's series path, which
    // reports it as a memory error the way libmpdec does.
    let p = p.min(MAX_WORK_DIGITS);
    let mut xc = BigInt::from(a.coefficient().clone());
    let mut xe = i128::from(a.exponent());
    while (&xc % 10u32).is_zero() {
        xc /= 10;
        xe += 1;
    }

    let y_sign = b.sign();
    let mut yc = BigInt::from(b.coefficient().clone());
    let mut ye = i128::from(b.exponent());
    while (&yc % 10u32).is_zero() {
        yc /= 10;
        ye += 1;
    }

    let b_is_nonneg_int = b.is_integer_valued() && b.sign() == 0;

    if xc == BigInt::one() {
        // x = 10**xe exactly.
        let mut exponent = BigInt::from(xe) * &yc;
        // exponent is xe*yc*10**ye; strip trailing zeros from xe*yc first
        // (matching `while xe % 10 == 0: xe //= 10; ye += 1`).
        while (&exponent % 10u32).is_zero() && !exponent.is_zero() {
            exponent /= 10;
            ye += 1;
        }
        if ye < 0 {
            return None;
        }
        let mut exponent = exponent * pow10_bi(ye);
        if y_sign == 1 {
            exponent = -exponent;
        }
        let zeros = if b_is_nonneg_int {
            let ideal_exponent = BigInt::from(a.exponent()) * BigInt::from(integer_value_i128(b)?);
            (&exponent - &ideal_exponent).min(BigInt::from(p - 1))
        } else {
            BigInt::zero()
        };
        let coeff = pow10_bi(zeros.clone().max(BigInt::zero()).to_i128()?);
        let final_exp = (&exponent - &zeros).to_i128()?;
        return Some(Decimal::new_finite(
            0,
            coeff.to_biguint().expect("nonnegative"),
            i128_to_i64_sat(final_exp),
        ));
    }

    if y_sign == 1 {
        let last_digit = (&xc % 10u32).to_u32().expect("single digit");
        let mut e;
        if matches!(last_digit, 2 | 4 | 6 | 8) {
            // xc must be a power of two.
            let m = xc.magnitude();
            if !(m & (m - 1u32)).is_zero() {
                return None;
            }
            e = BigInt::from(m.bits() - 1);
            let emax = p * 93 / 65;
            if ye >= digit_len_pos(emax) {
                return None;
            }
            e = decimal_lshift_exact(&(&e * &yc), ye)?;
            xe = decimal_lshift_exact(&(BigInt::from(xe) * &yc), ye)?.to_i128()?;
            if e > BigInt::from(emax) {
                return None;
            }
            xc = BigInt::from(5).pow(e.to_u32()?);
        } else if last_digit == 5 {
            e = BigInt::from(xc.magnitude().bits() as i128 * 28 / 65);
            let five_e = BigInt::from(5).pow(e.to_u32()?);
            let (q, r) = five_e.div_mod_floor(&xc);
            if !r.is_zero() {
                return None;
            }
            let mut xc2 = q;
            while (&xc2 % 5u32).is_zero() {
                xc2 /= 5;
                e -= 1;
            }
            let emax = p * 10 / 3;
            if ye >= digit_len_pos(emax) {
                return None;
            }
            e = decimal_lshift_exact(&(&e * &yc), ye)?;
            xe = decimal_lshift_exact(&(BigInt::from(xe) * &yc), ye)?.to_i128()?;
            if e > BigInt::from(emax) {
                return None;
            }
            xc = BigInt::from(2).pow(e.to_u32()?);
        } else {
            return None;
        }

        let strxc = xc.to_str_radix(10);
        if strxc.len() as i128 > p {
            return None;
        }
        let final_exp = -&e - BigInt::from(xe);
        return Some(Decimal::new_finite(
            0,
            xc.to_biguint().expect("nonnegative"),
            i128_to_i64_sat(final_exp.to_i128()?),
        ));
    }

    // y is positive.
    let xc_bits;
    let (m, mut n) = if ye >= 0 {
        xc_bits = 0; // unused on this path
        (&yc * pow10_bi(ye), BigInt::one())
    } else {
        if xe != 0 && (yc.clone() * xe).to_str_radix(10).len() as i128 <= -ye {
            return None;
        }
        let bits = xc.magnitude().bits() as i128;
        xc_bits = bits;
        if (yc.abs() * bits).to_str_radix(10).len() as i128 <= -ye {
            return None;
        }
        let mut m = yc.clone();
        let mut n = pow10_bi(-ye);
        while (&m % 2u32).is_zero() && (&n % 2u32).is_zero() {
            m /= 2;
            n /= 2;
        }
        while (&m % 5u32).is_zero() && (&n % 5u32).is_zero() {
            m /= 5;
            n /= 5;
        }
        (m, n)
    };

    if n > BigInt::one() {
        let bits = if xc_bits != 0 {
            xc_bits
        } else {
            xc.magnitude().bits() as i128
        };
        let n_i128 = n.to_i128()?;
        if bits <= n_i128 {
            return None;
        }
        let (q, r) = xe.div_mod_floor(&n_i128);
        if r != 0 {
            return None;
        }
        xe = q;

        // Newton's method for the integer nth root of xc.
        let n_u32 = u32::try_from(n_i128).ok()?;
        let ceil_bits = ceil_div(bits, n_i128);
        let mut a = BigInt::one() << (ceil_bits as u32);
        loop {
            let (q, r) = xc.div_mod_floor(&a.pow(n_u32 - 1));
            if a <= q {
                if a == q && r.is_zero() {
                    xc = a;
                    break;
                }
                return None;
            }
            a = (&a * i128::from(n_u32 - 1) + &q).div_floor(&BigInt::from(n_i128));
        }
        n = BigInt::one();
        let _ = n; // n no longer used past this point except via m below.
    }

    let m_i128 = m.to_i128()?;
    if xc > BigInt::one() && m_i128 > p * 100 / log10_lb(xc.magnitude()) {
        return None;
    }
    let m_u32 = u32::try_from(m_i128.unsigned_abs()).ok()?;
    xc = xc.pow(m_u32);
    xe *= m_i128;
    let str_xc = xc.to_str_radix(10);
    if str_xc.len() as i128 > p {
        return None;
    }

    let zeros = if b_is_nonneg_int {
        let ideal_exponent = BigInt::from(a.exponent()) * BigInt::from(integer_value_i128(b)?);
        (BigInt::from(xe) - &ideal_exponent).min(BigInt::from(p - str_xc.len() as i128))
    } else {
        BigInt::zero()
    };
    let extra_zeros = zeros.clone().max(BigInt::zero()).to_i128()?;
    let coeff = &xc * pow10_bi(extra_zeros);
    let final_exp = (BigInt::from(xe) - &zeros).to_i128()?;
    Some(Decimal::new_finite(
        0,
        coeff.to_biguint().expect("nonnegative"),
        i128_to_i64_sat(final_exp),
    ))
}

/// `Decimal.__pow__` without a modulus.
pub fn power(a: &Decimal, b: &Decimal, ctx: &Context, status: &mut u32) -> Decimal {
    if let Some(nan) = ops::check_nans(a, Some(b), ctx, status) {
        return nan;
    }

    if !b.is_nonzero() {
        return if !a.is_nonzero() {
            ops::invalid_operation(status)
        } else {
            Decimal::one()
        };
    }

    let mut result_sign = 0u8;
    let mut a = a.clone();
    if a.sign() == 1 {
        if b.is_integer_valued() {
            if !b.is_even_integer() {
                result_sign = 1;
            }
        } else if a.is_nonzero() {
            return ops::invalid_operation(status);
        }
        a = copy_negate(&a);
    }

    if !a.is_nonzero() {
        return if b.sign() == 0 {
            Decimal::zero(result_sign, 0)
        } else {
            Decimal::infinity(result_sign)
        };
    }

    if a.is_infinite() {
        return if b.sign() == 0 {
            Decimal::infinity(result_sign)
        } else {
            Decimal::zero(result_sign, 0)
        };
    }

    if equals_one(&a) {
        let multiplier: i128 = if b.is_integer_valued() {
            if b.sign() == 1 {
                0
            } else if ops::cmp_values(b, &Decimal::from_i64(ctx.prec)) == Ordering::Greater {
                i128::from(ctx.prec)
            } else {
                integer_value_i128(b).unwrap_or_else(|| i128::from(ctx.prec))
            }
        } else {
            *status |= status::INEXACT | status::ROUNDED;
            i128::MIN // sentinel: forces the "non-integer" exponent branch below
        };

        let mut exp = i128::from(a.exponent());
        let exp = if multiplier == i128::MIN {
            1 - i128::from(ctx.prec)
        } else {
            exp = exp.saturating_mul(multiplier);
            if exp < 1 - i128::from(ctx.prec) {
                *status |= status::ROUNDED;
                1 - i128::from(ctx.prec)
            } else {
                exp
            }
        };
        let zeros = if exp < 0 { (-exp) as u64 } else { 0 };
        return Decimal::new_finite(result_sign, bigops::pow10(zeros), i128_to_i64_sat(exp));
    }

    let self_adj = i128::from(a.adjusted());

    if b.is_infinite() {
        return if (b.sign() == 0) == (self_adj < 0) {
            Decimal::zero(result_sign, 0)
        } else {
            Decimal::infinity(result_sign)
        };
    }

    // No working-precision guard here: an integer exponent can have an exact
    // result that is cheap to materialise even under the unbounded context
    // `_pylong` uses (`prec = MAX_PREC`), which is exactly how `int(str)`
    // converts a huge decimal string. Only the series fallback below needs a
    // precision it can actually allocate.
    let p = i128::from(ctx.prec);

    let bound = log10_exp_bound(&a) + i128::from(b.adjusted());
    let mut ans = None;
    if (self_adj >= 0) == (b.sign() == 0) {
        if bound >= digit_len_pos(i128::from(ctx.emax)) {
            ans = Some(Decimal::new_finite(
                result_sign,
                BigUint::one(),
                ctx.emax + 1,
            ));
        }
    } else {
        let etiny = ctx.etiny();
        if bound >= digit_len_pos(i128::from(-etiny)) {
            ans = Some(Decimal::new_finite(result_sign, BigUint::one(), etiny - 1));
        }
    }

    let mut exact = false;
    if ans.is_none() {
        ans = power_exact(&a, b, p + 1).map(|pe| {
            exact = true;
            if result_sign == 1 {
                copy_negate(&pe)
            } else {
                pe
            }
        });
    }

    let ans = match ans {
        Some(ans) => ans,
        None => {
            if let Some(nan) = work_prec_guard(p, status) {
                return nan;
            }
            let xc = BigInt::from(a.coefficient().clone());
            let xe = i128::from(a.exponent());
            let mut yc = BigInt::from(b.coefficient().clone());
            let ye = i128::from(b.exponent());
            if b.sign() == 1 {
                yc = -yc;
            }
            let mut extra: i128 = 3;
            let (coeff, exp) = loop {
                let (coeff, exp) = dpower(&xc, xe, &yc, ye, p + extra);
                let clen = intlen(&coeff);
                let modulus = BigInt::from(5) * pow10_bi(clen - p - 1);
                if !(&coeff % &modulus).is_zero() {
                    break (coeff, exp);
                }
                extra += 3;
            };
            Decimal::new_finite(
                result_sign,
                coeff.to_biguint().expect("dpower coefficient is positive"),
                i128_to_i64_sat(exp),
            )
        }
    };

    if exact && !b.is_integer_valued() {
        let mut ans = ans;
        if ans.digits() <= ctx.prec {
            let expdiff = ctx.prec + 1 - ans.digits();
            let coeff = bigops::mul_pow10(ans.coefficient(), expdiff as u64);
            ans = Decimal::new_finite(ans.sign(), coeff, ans.exponent() - expdiff);
        }
        let fixed = ops::fix(&ans, ctx, status);
        *status |= status::INEXACT;
        if *status & status::SUBNORMAL != 0 {
            *status |= status::UNDERFLOW;
        }
        fixed
    } else {
        ops::fix(&ans, ctx, status)
    }
}

/// Binary modular exponentiation: `base**exp mod modulo`, all nonnegative,
/// `modulo` positive.
fn modpow(base: &BigUint, exp: &BigUint, modulo: &BigUint) -> BigUint {
    if modulo.is_one() {
        return BigUint::zero();
    }
    let mut result = BigUint::one();
    let mut base = base % modulo;
    let mut exp = exp.clone();
    let two = BigUint::from(2u32);
    while !exp.is_zero() {
        if (&exp % &two).is_one() {
            result = (&result * &base) % modulo;
        }
        exp /= &two;
        base = (&base * &base) % modulo;
    }
    result
}

/// `modpow` with an `i128` exponent, for exponents too small to warrant a
/// `BigUint`.
fn modpow_small(base: &BigUint, exp: i128, modulo: &BigUint) -> BigUint {
    debug_assert!(exp >= 0);
    modpow(base, &BigUint::from(exp as u128), modulo)
}

/// `Decimal.__pow__` with a modulus, i.e. `Decimal._power_modulo`.
pub fn power_modulo(
    a: &Decimal,
    b: &Decimal,
    modulo: &Decimal,
    ctx: &Context,
    status: &mut u32,
) -> Decimal {
    if a.is_snan() {
        return ops::quiet_snan(a, ctx, status);
    }
    if b.is_snan() {
        return ops::quiet_snan(b, ctx, status);
    }
    if modulo.is_snan() {
        return ops::quiet_snan(modulo, ctx, status);
    }
    if a.is_nan() {
        return ops::fix_nan(a, ctx);
    }
    if b.is_nan() {
        return ops::fix_nan(b, ctx);
    }
    if modulo.is_nan() {
        return ops::fix_nan(modulo, ctx);
    }

    if !(a.is_integer_valued() && b.is_integer_valued() && modulo.is_integer_valued()) {
        return ops::invalid_operation(status);
    }
    if ops::cmp_values(b, &Decimal::zero(0, 0)) == Ordering::Less {
        return ops::invalid_operation(status);
    }
    if modulo.is_zero() {
        return ops::invalid_operation(status);
    }
    if i128::from(modulo.adjusted()) >= i128::from(ctx.prec) {
        return ops::invalid_operation(status);
    }
    if !b.is_nonzero() && !a.is_nonzero() {
        return ops::invalid_operation(status);
    }

    let sign = if b.is_even_integer() { 0 } else { a.sign() };

    let modulo_int = if modulo.exponent() >= 0 {
        bigops::mul_pow10(modulo.coefficient(), modulo.exponent() as u64)
    } else {
        bigops::div_pow10(modulo.coefficient(), modulo.exponent().unsigned_abs())
    };

    let (base_coeff, base_exp): (BigUint, i128) = if a.exponent() >= 0 {
        (a.coefficient().clone(), i128::from(a.exponent()))
    } else {
        (
            bigops::div_pow10(a.coefficient(), a.exponent().unsigned_abs()),
            0,
        )
    };
    let (other_coeff, other_exp): (BigUint, i128) = if b.exponent() >= 0 {
        (b.coefficient().clone(), i128::from(b.exponent()))
    } else {
        (
            bigops::div_pow10(b.coefficient(), b.exponent().unsigned_abs()),
            0,
        )
    };

    if other_exp > 10_000_000 {
        return malloc_error_nan(status);
    }

    let ten_pow_base_exp = modpow_small(&BigUint::from(10u32), base_exp, &modulo_int);
    let mut base_val = (&base_coeff % &modulo_int * ten_pow_base_exp) % &modulo_int;

    let mut i = other_exp;
    while i > 0 {
        base_val = modpow(&base_val, &BigUint::from(10u32), &modulo_int);
        i -= 1;
    }
    base_val = modpow(&base_val, &other_coeff, &modulo_int);

    Decimal::new_finite(sign, base_val, 0)
}

#[cfg(test)]
// Each test is named after the function it exercises, so the `test_` prefix is
// what keeps the two apart.
#[allow(clippy::redundant_test_prefix)]
mod tests {
    use super::*;
    use crate::Context;

    // The expected values below were captured once from CPython 3.14's
    // `_decimal` (the C-accelerated module, the reference for the public
    // `sqrt`/`ln`/`log10`/`exp`/`power` API) and `_pydecimal` (the pure-Python
    // reference for the internal helpers `_sqrt_nearest`, `_rshift_nearest`,
    // etc.), then hard-coded here so the test suite is hermetic:
    //
    //   python3.14 -c "import _decimal as D; c=D.Context(prec=9); print(c.sqrt(D.Decimal('2')))"
    //   python3.14 -c "import _pydecimal as P; print(P._sqrt_nearest(2, 1))"
    //
    // `cargo test` never shells out to an interpreter or depends on any
    // particular machine having Python installed.

    fn ctx(prec: i64) -> Context {
        Context {
            prec,
            ..Context::default()
        }
    }

    fn d(s: &str) -> Decimal {
        Decimal::parse_ascii(s.as_bytes()).unwrap()
    }

    #[test]
    fn test_decimal_lshift_exact() {
        assert_eq!(
            decimal_lshift_exact(&BigInt::from(3), 4),
            Some(BigInt::from(30000))
        );
        assert_eq!(decimal_lshift_exact(&BigInt::from(300), -999_999_999), None);
        assert_eq!(
            decimal_lshift_exact(&BigInt::from(500), -2),
            Some(BigInt::from(5))
        );
        assert_eq!(
            decimal_lshift_exact(&BigInt::from(0), -2),
            Some(BigInt::zero())
        );
    }

    #[test]
    fn test_sqrt_nearest() {
        for (n, want) in [
            (1u64, 1i128),
            (2, 1),
            (3, 2),
            (4, 2),
            (99, 10),
            (100, 10),
            (101, 10),
            (123456789, 11111),
        ] {
            let got = sqrt_nearest(&BigInt::from(n), &BigInt::from(1));
            assert_eq!(got, BigInt::from(want), "n={n}");
        }
    }

    #[test]
    fn test_rshift_nearest() {
        for (x, shift, want) in [
            (17i64, 2, 4i128),
            (-17, 2, -4),
            (100, 3, 12),
            (-100, 3, -12),
            (5, 1, 2),
            (-5, 1, -2),
        ] {
            let got = rshift_nearest(&BigInt::from(x), shift);
            assert_eq!(got, BigInt::from(want), "x={x} shift={shift}");
        }
    }

    #[test]
    fn test_div_nearest() {
        for (a, b, want) in [
            (17i64, 5, 3i128),
            (-17, 5, -3),
            (15, 2, 8),
            (-15, 2, -8),
            (0, 7, 0),
        ] {
            let got = div_nearest(&BigInt::from(a), &BigInt::from(b));
            assert_eq!(got, BigInt::from(want), "a={a} b={b}");
        }
    }

    #[test]
    fn test_ilog() {
        for (x, m, want) in [
            (105i64, 100, 5i128),
            (95, 100, -5),
            (1000, 100, 231),
            (11, 10, 1),
        ] {
            let got = ilog(&BigInt::from(x), &BigInt::from(m));
            assert_eq!(got, BigInt::from(want), "x={x} m={m}");
        }
    }

    #[test]
    fn test_dlog10() {
        for (c, e, p, want) in [
            (123i64, -2, 20, 8_990_511_143_939_793_180i128),
            (5, 0, 15, 698_970_004_336_019),
            (999_999, -6, 30, -434_294_699_050_637_544_212_918),
        ] {
            let got = dlog10(&BigInt::from(c), e, p);
            assert_eq!(got, BigInt::from(want), "c={c} e={e} p={p}");
        }
    }

    #[test]
    fn test_dlog() {
        for (c, e, p, want) in [
            (123i64, -2, 20, 20_701_416_938_432_612_723i128),
            (5, 0, 15, 1_609_437_912_434_100),
            (999_999, -6, 30, -1_000_000_500_000_333_333_583_334),
        ] {
            let got = dlog(&BigInt::from(c), e, p);
            assert_eq!(got, BigInt::from(want), "c={c} e={e} p={p}");
        }
    }

    #[test]
    fn test_log10_digits() {
        for (p, want) in [
            (0i128, "2"),
            (1, "23"),
            (3, "2302"),
            (10, "23025850929"),
            (47, "230258509299404568401799145468436420760110148862"),
            (48, "2302585092994045684017991454684364207601101488628"),
            (49, "23025850929940456840179914546843642076011014886287"),
            (50, "230258509299404568401799145468436420760110148862877"),
            (
                100,
                "23025850929940456840179914546843642076011014886287729760333279009675726096773524802359972050895982983",
            ),
        ] {
            let got = log10_digits(p);
            assert_eq!(got.to_str_radix(10), want, "p={p}");
        }
    }

    #[test]
    fn test_iexp_dexp() {
        for (c, e, p, want_coeff, want_exp) in [
            (1i64, 0, 20, 27_182_818_284_590_452_354i128, -19i128),
            (5, -1, 15, 164_872_127_070_013, -14),
            (123, -2, 30, 342_122_953_628_967_357_379_015_235_145, -29),
        ] {
            let (coeff, exp) = dexp(&BigInt::from(c), e, p);
            assert_eq!(coeff, BigInt::from(want_coeff), "c={c} e={e} p={p}");
            assert_eq!(exp, want_exp, "c={c} e={e} p={p}");
        }
    }

    #[test]
    fn test_log10_lb() {
        for (c, want) in [
            (1u64, 0i128),
            (2, 30),
            (3, 47),
            (5, 69),
            (9, 95),
            (10, 100),
            (99, 195),
            (100, 200),
            (999_999, 595),
        ] {
            let got = log10_lb(&BigUint::from(c));
            assert_eq!(got, want, "c={c}");
        }
    }

    // -----------------------------------------------------------------
    // Public functions, checked against captured `_decimal` output.
    // -----------------------------------------------------------------

    fn check_unary(
        name: &str,
        prec: i64,
        round: RoundMode,
        input: &str,
        expected: &str,
        f: impl Fn(&Decimal, &Context, &mut u32) -> Decimal,
    ) {
        let mut c = ctx(prec);
        c.round = round;
        let mut status = 0u32;
        let got = f(&d(input), &c, &mut status);
        assert_eq!(
            got.to_sci_string(true),
            expected,
            "{name}({input}) prec={prec} round={round:?}"
        );
    }

    #[test]
    fn test_sqrt_basic() {
        for (prec, input, want) in [
            (9, "2", "1.41421356"),
            (9, "0", "0"),
            (9, "-0", "-0"),
            (28, "4", "2"),
            (28, "2", "1.414213562373095048801688724"),
            (10, "1E+10", "1E+5"),
            (10, "1.44", "1.2"),
            (5, "123456", "351.36"),
            (9, "9999999999999999", "100000000"),
        ] {
            check_unary("sqrt", prec, RoundMode::HalfEven, input, want, sqrt);
        }
    }

    #[test]
    fn test_sqrt_rounding_modes() {
        // The correctly-rounded square root of 2 lands the same way under
        // every rounding mode at these two precisions.
        for round in RoundMode::ALL {
            check_unary("sqrt", 7, round, "2", "1.414214", sqrt);
            check_unary("sqrt", 3, round, "2", "1.41", sqrt);
        }
    }

    #[test]
    fn test_sqrt_negative_and_specials() {
        let c = ctx(9);
        let mut status = 0u32;
        let got = sqrt(&d("-4"), &c, &mut status);
        assert!(got.is_qnan());
        assert_ne!(status & status::INVALID_OPERATION, 0);

        let mut status = 0u32;
        let got = sqrt(&Decimal::infinity(0), &c, &mut status);
        assert!(got.is_infinite());

        let mut status = 0u32;
        let got = sqrt(&Decimal::infinity(1), &c, &mut status);
        assert!(got.is_qnan());
    }

    #[test]
    fn test_exp_basic() {
        for (prec, input, want) in [
            (9, "0", "1"),
            (9, "1", "2.71828183"),
            (28, "1", "2.718281828459045235360287471"),
            (9, "-1", "0.367879441"),
            (9, "10", "22026.4658"),
            (9, "-10", "0.0000453999298"),
            (9, "0.0000001", "1.00000010"),
            (12, "2.302585092994046", "10.0000000000"),
        ] {
            check_unary("exp", prec, RoundMode::HalfEven, input, want, exp);
        }
    }

    #[test]
    fn test_exp_specials() {
        let c = ctx(9);
        let mut status = 0u32;
        assert!(exp(&Decimal::infinity(1), &c, &mut status).is_zero());
        let mut status = 0u32;
        assert!(exp(&Decimal::infinity(0), &c, &mut status).is_infinite());
    }

    #[test]
    fn test_ln_basic() {
        for (prec, input, want) in [
            (9, "1", "0"),
            (28, "1", "0"),
            (9, "2", "0.693147181"),
            (9, "10", "2.30258509"),
            (9, "0.5", "-0.693147181"),
            (12, "2.718281828459045", "1.00000000000"),
            (9, "1000000", "13.8155106"),
        ] {
            check_unary("ln", prec, RoundMode::HalfEven, input, want, ln);
        }
    }

    #[test]
    fn test_ln_specials() {
        let c = ctx(9);
        let mut status = 0u32;
        assert!(ln(&Decimal::zero(0, 0), &c, &mut status).is_infinite());
        let mut status = 0u32;
        let got = ln(&d("-1"), &c, &mut status);
        assert!(got.is_qnan());
    }

    #[test]
    fn test_ln_rounding_modes() {
        // Same result under every rounding mode at these precisions.
        for round in RoundMode::ALL {
            check_unary("ln", 7, round, "2", "0.6931472", ln);
            check_unary("ln", 9, round, "0.001", "-6.90775528", ln);
        }
    }

    #[test]
    fn test_ln_extreme_operands() {
        for (prec, input, want) in [
            (12, "1E+300", "690.775527898"),
            (12, "1E-300", "-690.775527898"),
            (9, "9.999999999E+999999", "2302585.09"),
            (9, "1E-999998", "-2302580.49"),
        ] {
            check_unary("ln", prec, RoundMode::HalfEven, input, want, ln);
        }
    }

    #[test]
    fn test_log10_basic() {
        for (prec, input, want) in [
            (9, "1", "0"),
            (9, "10", "1"),
            (9, "100", "2"),
            (9, "1000", "3"),
            (28, "1E+10", "10"),
            (9, "0.1", "-1"),
            (9, "0.01", "-2"),
            (9, "2", "0.301029996"),
            (9, "3.14159", "0.497149506"),
            (9, "0.001", "-3"),
            (12, "1E+300", "300"),
            (12, "1E-300", "-300"),
        ] {
            check_unary("log10", prec, RoundMode::HalfEven, input, want, log10);
        }
    }

    #[test]
    fn test_log10_rounding_modes() {
        // Same result under every rounding mode at these precisions.
        for round in RoundMode::ALL {
            check_unary("log10", 7, round, "2", "0.3010300", log10);
            check_unary("log10", 5, round, "12345", "4.0915", log10);
        }
    }

    #[test]
    fn test_log10_specials() {
        let c = ctx(9);
        let mut status = 0u32;
        assert!(log10(&Decimal::zero(0, 0), &c, &mut status).is_infinite());
        let mut status = 0u32;
        let got = log10(&d("-5"), &c, &mut status);
        assert!(got.is_qnan());
        let mut status = 0u32;
        let snan_input = Decimal::nan(0, BigUint::zero(), true);
        let got = log10(&snan_input, &c, &mut status);
        assert!(got.is_qnan());
        assert_ne!(status & status::INVALID_OPERATION, 0);
    }

    #[test]
    fn test_exp_rounding_modes() {
        // Same result under every rounding mode at these precisions.
        for round in RoundMode::ALL {
            check_unary("exp", 7, round, "1", "2.718282", exp);
            check_unary("exp", 5, round, "-1.5", "0.22313", exp);
        }
    }

    #[test]
    fn test_exp_subnormal_underflow() {
        // With a tight Emin, exp of a large negative value underflows to a
        // subnormal or clamped zero.
        let mut c = ctx(9);
        c.emin = -10;
        c.emax = 10;
        let mut status = 0u32;
        let got = exp(&d("-50"), &c, &mut status);
        assert!(got.is_zero() || got.is_finite());
        assert_ne!(
            status & (status::UNDERFLOW | status::SUBNORMAL | status::CLAMPED),
            0
        );
    }

    #[test]
    fn test_sqrt_huge_and_tiny() {
        for (prec, input, want) in [
            (12, "1E+300", "1E+150"),
            (12, "1E-300", "1E-150"),
            (9, "9.999999999E+999999", "1.00000000E+500000"),
            (30, "2E+1", "4.47213595499957939281834733746"),
        ] {
            check_unary("sqrt", prec, RoundMode::HalfEven, input, want, sqrt);
        }
    }

    #[test]
    fn test_power_extreme_exponents() {
        // Overflow: base > 1 raised to a huge exponent.
        let mut status = 0u32;
        let got = power(&d("2"), &d("1E+50"), &ctx(9), &mut status);
        assert!(got.is_infinite() || (got.is_finite() && status & status::OVERFLOW != 0));

        // Underflow: base > 1 raised to a huge negative exponent.
        let mut status = 0u32;
        let got = power(&d("2"), &d("-1E+50"), &ctx(9), &mut status);
        assert!(got.is_zero() || status & (status::UNDERFLOW | status::CLAMPED) != 0);
    }

    fn check_pow(prec: i64, round: RoundMode, base: &str, exponent: &str, expected: &str) {
        let mut c = ctx(prec);
        c.round = round;
        let mut status = 0u32;
        let got = power(&d(base), &d(exponent), &c, &mut status);
        assert_eq!(
            got.to_sci_string(true),
            expected,
            "pow({base}, {exponent}) prec={prec} round={round:?}"
        );
    }

    #[test]
    fn test_power_basic() {
        for (prec, base, exp, want) in [
            (9, "2", "3", "8"),
            (9, "2", "0.5", "1.41421356"),
            (9, "2", "-3", "0.125"),
            (9, "0", "0", "NaN"),
            (9, "0", "5", "0"),
            (9, "0", "-5", "Infinity"),
            (9, "-2", "3", "-8"),
            (9, "-2", "4", "16"),
            (9, "-2", "0.5", "NaN"),
            (9, "10", "3", "1000"),
            (9, "1", "1000000", "1"),
            (9, "1", "0.5", "1.00000000"),
            (28, "2", "10", "1024"),
            (9, "3", "3.5", "46.7653718"),
        ] {
            check_pow(prec, RoundMode::HalfEven, base, exp, want);
        }
    }

    #[test]
    fn test_power_rounding_modes() {
        for (round, want_a, want_b) in [
            (RoundMode::Down, "1.414213", "58.257"),
            (RoundMode::HalfUp, "1.414214", "58.257"),
            (RoundMode::HalfEven, "1.414214", "58.257"),
            (RoundMode::Ceiling, "1.414214", "58.258"),
            (RoundMode::Floor, "1.414213", "58.257"),
            (RoundMode::Up, "1.414214", "58.258"),
            (RoundMode::HalfDown, "1.414214", "58.257"),
            (RoundMode::ZeroFiveUp, "1.414213", "58.257"),
        ] {
            check_pow(7, round, "2", "0.5", want_a);
            check_pow(5, round, "3", "3.7", want_b);
        }
    }

    #[test]
    fn test_power_modulo_basic() {
        let c = ctx(20);
        let mut status = 0u32;
        let got = power_modulo(&d("3"), &d("5"), &d("7"), &c, &mut status);
        assert_eq!(got.to_sci_string(true), "5"); // 3**5 % 7 = 243 % 7 = 5

        let mut status = 0u32;
        let got = power_modulo(&d("-3"), &d("4"), &d("7"), &c, &mut status);
        assert_eq!(got.to_sci_string(true), "4"); // (-3)**4 % 7 = 81%7=4, sign 0 (even exp)

        let mut status = 0u32;
        let got = power_modulo(&d("2"), &d("-1"), &d("7"), &c, &mut status);
        assert!(got.is_qnan());
    }

    #[test]
    fn test_power_modulo_more() {
        let c = ctx(20);

        // 0 ** 0 % m is invalid.
        let mut status = 0u32;
        let got = power_modulo(&d("0"), &d("0"), &d("5"), &c, &mut status);
        assert!(got.is_qnan());

        // modulo of 0 is invalid.
        let mut status = 0u32;
        let got = power_modulo(&d("2"), &d("3"), &d("0"), &c, &mut status);
        assert!(got.is_qnan());

        // non-integer operand is invalid.
        let mut status = 0u32;
        let got = power_modulo(&d("2.5"), &d("3"), &d("5"), &c, &mut status);
        assert!(got.is_qnan());

        // exponent with a positive decimal exponent field (e.g. 2E+2 = 200).
        let mut status = 0u32;
        let got = power_modulo(&d("3"), &d("2E+1"), &d("100"), &c, &mut status);
        // 3**20 mod 100
        let want: u64 = {
            let mut r: u64 = 1;
            for _ in 0..20 {
                r = (r * 3) % 100;
            }
            r
        };
        assert_eq!(got.to_sci_string(true), want.to_string());

        // base with a positive decimal exponent field.
        let mut status2 = 0u32;
        let got = power_modulo(&d("2E+1"), &d("3"), &d("1000"), &c, &mut status2);
        // 20**3 mod 1000 = 8000 mod 1000 = 0
        assert_eq!(got.to_sci_string(true), "0");
    }
}

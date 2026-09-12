//! Decimal-oriented helpers on top of arbitrary-precision integers.
//!
//! `Lib/_pydecimal.py` keeps a coefficient as a string of digits and leans on
//! `len(str(n))`, slicing and `int(...)` round-trips. Here the coefficient is a
//! `BigUint`, so the equivalent operations are expressed as divisions by powers
//! of ten; these helpers provide them together with a cache of the powers.

use alloc::vec::Vec;
use core::cell::RefCell;
use malachite_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

/// `10.pow(e)` for every `e` a `u128` can hold exactly.
const SMALL_POW10: [u128; 39] = {
    let mut table = [1u128; 39];
    let mut i = 1;
    while i < 39 {
        table[i] = table[i - 1] * 10;
        i += 1;
    }
    table
};

thread_local! {
    /// `10.pow(1 << k)` for the `k`s reached so far, used to build large powers
    /// of ten by squaring instead of by multiplying ten in a loop.
    static POW10_SQUARES: RefCell<Vec<BigUint>> = const { RefCell::new(Vec::new()) };
}

/// `10.pow(e)`.
///
/// # Panics
///
/// Panics if the result cannot be allocated, i.e. for exponents beyond what the
/// caller's precision limits should already have excluded.
pub fn pow10(e: u64) -> BigUint {
    if let Some(small) = SMALL_POW10.get(e as usize) {
        return BigUint::from(*small);
    }
    POW10_SQUARES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.is_empty() {
            cache.push(BigUint::from(10u32));
        }
        let mut result = BigUint::one();
        let mut rest = e;
        let mut k = 0usize;
        while rest != 0 {
            if k == cache.len() {
                let next = &cache[k - 1] * &cache[k - 1];
                cache.push(next);
            }
            if rest & 1 != 0 {
                result *= &cache[k];
            }
            rest >>= 1;
            k += 1;
        }
        result
    })
}

/// Number of decimal digits in `n`, counting `0` as one digit.
pub fn digit_count(n: &BigUint) -> i64 {
    if let Some(small) = n.to_u128() {
        return digit_count_u128(small);
    }
    // `n.bits() * log10(2)` under-estimates `floor(log10(n))` by at most one.
    let mut d = ((u128::from(n.bits()) * 1233) >> 12) as u64;
    while *n < pow10(d) {
        d -= 1;
    }
    while *n >= pow10(d + 1) {
        d += 1;
    }
    d as i64 + 1
}

/// Number of decimal digits in `n`, counting `0` as one digit.
pub fn digit_count_u128(n: u128) -> i64 {
    match SMALL_POW10.binary_search(&n) {
        Ok(i) => i as i64 + 1,
        Err(i) => i as i64,
    }
    .max(1)
}

/// `n * 10.pow(e)`.
pub fn mul_pow10(n: &BigUint, e: u64) -> BigUint {
    if n.is_zero() || e == 0 {
        return n.clone();
    }
    n * pow10(e)
}

/// `n // 10.pow(e)`, i.e. the digits of `n` with the last `e` dropped.
pub fn div_pow10(n: &BigUint, e: u64) -> BigUint {
    if n.is_zero() || e == 0 {
        return n.clone();
    }
    if dwarfs(n, e) {
        return BigUint::zero();
    }
    n / pow10(e)
}

/// Whether `10.pow(e)` is certainly larger than `n`, decided without building
/// it.
///
/// A decimal exponent can be as far from a coefficient's size as `1e-999999999`
/// is from `1`, and materialising `10.pow(e)` for such an `e` costs hundreds of
/// megabytes and seconds of multiplication for an answer that is a foregone
/// conclusion. `10.pow(e) >= 2.pow(e)`, so `e` at least the bit length settles
/// it; the bound is loose by the ratio between the two logarithms, which only
/// costs an exact comparison in a narrow band.
fn dwarfs(n: &BigUint, e: u64) -> bool {
    e >= n.bits()
}

/// `(n // 10.pow(e), n % 10.pow(e))`.
pub fn split_pow10(n: &BigUint, e: u64) -> (BigUint, BigUint) {
    if e == 0 {
        return (n.clone(), BigUint::zero());
    }
    if n.is_zero() {
        return (BigUint::zero(), BigUint::zero());
    }
    if dwarfs(n, e) {
        return (BigUint::zero(), n.clone());
    }
    let p = pow10(e);
    let q = n / &p;
    let r = n - &q * &p;
    (q, r)
}

/// `n % 10.pow(e)`, i.e. the last `e` digits of `n`.
pub fn rem_pow10(n: &BigUint, e: u64) -> BigUint {
    split_pow10(n, e).1
}

/// The number of times ten divides `n`, or `None` when `n` is zero.
pub fn trailing_zeros10(n: &BigUint) -> Option<u64> {
    if n.is_zero() {
        return None;
    }
    // A power of ten dividing `n` divides both two and five out of it, so the
    // count is bounded by the number of trailing binary zeros.
    let bound = n.trailing_zeros().unwrap_or(0);
    let mut lo = 0u64;
    let mut hi = bound + 1;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if rem_pow10(n, mid).is_zero() {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some(lo)
}

/// The decimal digits of `n`, most significant first, without leading zeros
/// (`0` renders as a single `0` digit).
pub fn to_digits(n: &BigUint) -> Vec<u8> {
    n.to_str_radix(10).into_bytes()
}

/// Builds an integer from ASCII decimal digits.
pub fn from_ascii_digits(digits: &[u8]) -> BigUint {
    if digits.is_empty() {
        return BigUint::zero();
    }
    BigUint::parse_bytes(digits, 10).expect("caller must pass ASCII decimal digits")
}

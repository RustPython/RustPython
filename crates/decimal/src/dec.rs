//! The decimal number itself.
//!
//! A finite value is `(-1)**sign * coeff * 10**exp`. As in `_pydecimal`, the
//! coefficient keeps its trailing zeros (they carry the value's quantum) while
//! leading zeros are always stripped, so `digits` is the length that
//! `_pydecimal` would get from `len(self._int)`.

use crate::bigops;
use crate::{MAX_EMAX, MAX_PREC, MIN_ETINY};
use alloc::string::String;
use alloc::vec::Vec;
use malachite_bigint::BigUint;
use num_traits::Zero;

/// Which of the specification's four kinds of value this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Special {
    Finite,
    /// `Infinity`; `_pydecimal` spells this exponent `'F'`.
    Inf,
    /// Quiet `NaN`; `_pydecimal` spells this exponent `'n'`.
    Nan,
    /// Signaling `NaN`; `_pydecimal` spells this exponent `'N'`.
    Snan,
}

#[derive(Clone, Debug)]
pub struct Decimal {
    pub(crate) sign: u8,
    pub(crate) special: Special,
    /// Coefficient for a finite value, diagnostic payload for a NaN, zero for
    /// an infinity.
    pub(crate) coeff: BigUint,
    /// `len(_int)`: at least one for a finite value or an infinity, and zero
    /// for a NaN whose payload is empty.
    pub(crate) digits: i64,
    /// Meaningful only for finite values.
    pub(crate) exp: i64,
}

/// Why a string is not a valid decimal literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// The string does not match the numeric-string grammar at all.
    Syntax,
    /// The string is well formed but names a value outside the range the
    /// module can represent exactly.
    Range,
}

impl Decimal {
    pub fn new_finite(sign: u8, coeff: BigUint, exp: i64) -> Self {
        let digits = bigops::digit_count(&coeff);
        Self {
            sign,
            special: Special::Finite,
            coeff,
            digits,
            exp,
        }
    }

    /// Builds a finite value whose digit count is already known, as
    /// `_dec_from_triple` does when the caller has just laid the digits out.
    pub(crate) fn from_triple(sign: u8, coeff: BigUint, digits: i64, exp: i64) -> Self {
        debug_assert_eq!(digits, bigops::digit_count(&coeff));
        Self {
            sign,
            special: Special::Finite,
            coeff,
            digits,
            exp,
        }
    }

    pub fn zero(sign: u8, exp: i64) -> Self {
        Self {
            sign,
            special: Special::Finite,
            coeff: BigUint::zero(),
            digits: 1,
            exp,
        }
    }

    pub fn infinity(sign: u8) -> Self {
        Self {
            sign,
            special: Special::Inf,
            coeff: BigUint::zero(),
            digits: 1,
            exp: 0,
        }
    }

    pub fn nan(sign: u8, payload: BigUint, signaling: bool) -> Self {
        let digits = if payload.is_zero() {
            0
        } else {
            bigops::digit_count(&payload)
        };
        Self {
            sign,
            special: if signaling {
                Special::Snan
            } else {
                Special::Nan
            },
            coeff: payload,
            digits,
            exp: 0,
        }
    }

    pub fn from_i64(value: i64) -> Self {
        let sign = u8::from(value < 0);
        Self::new_finite(sign, BigUint::from(value.unsigned_abs()), 0)
    }

    pub fn one() -> Self {
        Self::from_i64(1)
    }

    #[inline]
    pub fn sign(&self) -> u8 {
        self.sign
    }

    #[inline]
    pub fn special(&self) -> Special {
        self.special
    }

    #[inline]
    pub fn coefficient(&self) -> &BigUint {
        &self.coeff
    }

    #[inline]
    pub fn digits(&self) -> i64 {
        self.digits
    }

    #[inline]
    pub fn exponent(&self) -> i64 {
        self.exp
    }

    #[inline]
    pub fn is_special(&self) -> bool {
        !matches!(self.special, Special::Finite)
    }

    #[inline]
    pub fn is_finite(&self) -> bool {
        matches!(self.special, Special::Finite)
    }

    #[inline]
    pub fn is_nan(&self) -> bool {
        matches!(self.special, Special::Nan | Special::Snan)
    }

    #[inline]
    pub fn is_qnan(&self) -> bool {
        matches!(self.special, Special::Nan)
    }

    #[inline]
    pub fn is_snan(&self) -> bool {
        matches!(self.special, Special::Snan)
    }

    #[inline]
    pub fn is_infinite(&self) -> bool {
        matches!(self.special, Special::Inf)
    }

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.is_finite() && self.coeff.is_zero()
    }

    /// True when the value is nonzero, matching `Decimal.__bool__`.
    #[inline]
    pub fn is_nonzero(&self) -> bool {
        self.is_special() || !self.coeff.is_zero()
    }

    /// `self.adjusted()`: the exponent the value would have written with a
    /// single digit before the point. Specials report zero, as in `_pydecimal`.
    #[inline]
    pub fn adjusted(&self) -> i64 {
        if self.is_special() {
            0
        } else {
            // A tuple-built value can sit at the bottom of the exponent range,
            // where the sum would not fit.
            self.exp.saturating_add(self.digits - 1)
        }
    }

    /// True for a finite value that is an exact integer.
    pub fn is_integer_valued(&self) -> bool {
        if self.is_special() {
            return false;
        }
        if self.exp >= 0 || self.coeff.is_zero() {
            return true;
        }
        let drop = self.exp.unsigned_abs();
        bigops::rem_pow10(&self.coeff, drop).is_zero()
    }

    /// True for a finite value that is an even integer.
    pub fn is_even_integer(&self) -> bool {
        if !self.is_integer_valued() {
            return false;
        }
        if self.coeff.is_zero() {
            return true;
        }
        if self.exp > 0 {
            // A positive exponent leaves a trailing zero, so the value is even.
            return true;
        }
        let scaled = bigops::div_pow10(&self.coeff, self.exp.unsigned_abs());
        (scaled % BigUint::from(2u32)).is_zero()
    }

    /// The coefficient rendered as ASCII digits, i.e. `_pydecimal`'s `_int`.
    pub fn coeff_digits(&self) -> Vec<u8> {
        if self.is_nan() && self.coeff.is_zero() {
            return Vec::new();
        }
        bigops::to_digits(&self.coeff)
    }

    /// `Decimal.__str__`, i.e. `to-scientific-string`.
    pub fn to_sci_string(&self, capitals: bool) -> String {
        self.format_plain(false, capitals)
    }

    /// `Decimal.to_eng_string`.
    pub fn to_eng_string(&self, capitals: bool) -> String {
        self.format_plain(true, capitals)
    }

    fn format_plain(&self, eng: bool, capitals: bool) -> String {
        let mut out = String::new();
        if self.sign != 0 {
            out.push('-');
        }
        match self.special {
            Special::Inf => {
                out.push_str("Infinity");
                return out;
            }
            Special::Nan | Special::Snan => {
                out.push_str(if self.special == Special::Snan {
                    "sNaN"
                } else {
                    "NaN"
                });
                if !self.coeff.is_zero() {
                    out.push_str(&self.coeff.to_str_radix(10));
                }
                return out;
            }
            Special::Finite => {}
        }

        let digits = bigops::to_digits(&self.coeff);
        let ndigits = digits.len() as i64;
        let leftdigits = self.exp + ndigits;
        let dotplace = if self.exp <= 0 && leftdigits > -6 {
            leftdigits
        } else if !eng {
            1
        } else if self.coeff.is_zero() {
            (leftdigits + 1).rem_euclid(3) - 1
        } else {
            (leftdigits - 1).rem_euclid(3) + 1
        };

        if dotplace <= 0 {
            out.push_str("0.");
            for _ in 0..-dotplace {
                out.push('0');
            }
            out.push_str(core::str::from_utf8(&digits).expect("ASCII digits"));
        } else if dotplace >= ndigits {
            out.push_str(core::str::from_utf8(&digits).expect("ASCII digits"));
            for _ in 0..dotplace - ndigits {
                out.push('0');
            }
        } else {
            let split = dotplace as usize;
            out.push_str(core::str::from_utf8(&digits[..split]).expect("ASCII digits"));
            out.push('.');
            out.push_str(core::str::from_utf8(&digits[split..]).expect("ASCII digits"));
        }
        if leftdigits != dotplace {
            out.push(if capitals { 'E' } else { 'e' });
            let exp = leftdigits - dotplace;
            out.push(if exp < 0 { '-' } else { '+' });
            out.push_str(&itoa_abs(exp));
        }
        out
    }

    /// Parses the specification's numeric-string grammar.
    ///
    /// The caller is responsible for the Unicode-level preprocessing CPython
    /// does first: stripping surrounding whitespace, dropping underscores and
    /// folding non-ASCII decimal digits to ASCII.
    pub fn parse_ascii(s: &[u8]) -> Result<Self, ParseError> {
        let value = Self::parse_ascii_raw(s)?;
        if value.is_exactly_representable() {
            Ok(value)
        } else {
            Err(ParseError::Range)
        }
    }

    /// Parses without demanding that the value be exactly representable, for
    /// `Context.create_decimal`, which rounds into the context instead.
    pub fn parse_ascii_raw(s: &[u8]) -> Result<Self, ParseError> {
        let (sign, rest) = match s.first() {
            Some(b'-') => (1u8, &s[1..]),
            Some(b'+') => (0u8, &s[1..]),
            _ => (0u8, s),
        };
        if rest.is_empty() {
            return Err(ParseError::Syntax);
        }

        // Infinity / NaN.
        if !rest[0].is_ascii_digit() && rest[0] != b'.' {
            let lower: Vec<u8> = rest.to_ascii_lowercase();
            if lower == b"inf" || lower == b"infinity" {
                return Ok(Self::infinity(sign));
            }
            let (signaling, body) = if lower.starts_with(b"s") {
                (true, &lower[1..])
            } else {
                (false, &lower[..])
            };
            if !body.starts_with(b"nan") {
                return Err(ParseError::Syntax);
            }
            let diag = &body[3..];
            if !diag.iter().all(u8::is_ascii_digit) {
                return Err(ParseError::Syntax);
            }
            let payload = bigops::from_ascii_digits(diag);
            return Ok(Self::nan(sign, payload, signaling));
        }

        // Finite: digits, optional fraction, optional exponent.
        let mut i = 0usize;
        let int_start = i;
        while i < rest.len() && rest[i].is_ascii_digit() {
            i += 1;
        }
        let int_part = &rest[int_start..i];
        let frac_part: &[u8] = if i < rest.len() && rest[i] == b'.' {
            i += 1;
            let frac_start = i;
            while i < rest.len() && rest[i].is_ascii_digit() {
                i += 1;
            }
            &rest[frac_start..i]
        } else {
            b""
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(ParseError::Syntax);
        }
        let exp: i64 = if i < rest.len() {
            if rest[i] != b'e' && rest[i] != b'E' {
                return Err(ParseError::Syntax);
            }
            i += 1;
            let neg = match rest.get(i) {
                Some(b'-') => {
                    i += 1;
                    true
                }
                Some(b'+') => {
                    i += 1;
                    false
                }
                _ => false,
            };
            let digits = &rest[i..];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Err(ParseError::Syntax);
            }
            i = rest.len();
            let mut acc: i64 = 0;
            let mut overflowed = false;
            for &d in digits {
                if overflowed {
                    continue;
                }
                match acc
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(i64::from(d - b'0')))
                {
                    Some(v) => acc = v,
                    None => overflowed = true,
                }
            }
            if overflowed {
                return Err(ParseError::Range);
            }
            if neg { -acc } else { acc }
        } else {
            0
        };
        if i != rest.len() {
            return Err(ParseError::Syntax);
        }

        let mut all = Vec::with_capacity(int_part.len() + frac_part.len());
        all.extend_from_slice(int_part);
        all.extend_from_slice(frac_part);
        let coeff = bigops::from_ascii_digits(&all);
        let exp = match exp.checked_sub(frac_part.len() as i64) {
            Some(v) => v,
            None => return Err(ParseError::Range),
        };
        Ok(Self::new_finite(sign, coeff, exp))
    }

    /// Whether the value fits the widest context the module supports, which is
    /// what an exact (context-free) conversion demands.
    pub fn is_exactly_representable(&self) -> bool {
        if self.is_special() {
            return self.digits <= MAX_PREC;
        }
        if self.digits > MAX_PREC || self.exp < MIN_ETINY || self.exp > MAX_EMAX {
            return false;
        }
        self.coeff.is_zero() || self.adjusted() <= MAX_EMAX
    }
}

fn itoa_abs(value: i64) -> String {
    let mut buf = String::new();
    let mut n = value.unsigned_abs();
    if n == 0 {
        buf.push('0');
        return buf;
    }
    let mut tmp = [0u8; 20];
    let mut len = 0;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        buf.push(tmp[i] as char);
    }
    buf
}

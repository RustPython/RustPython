use std::{
    cmp::Ordering,
    ops::{
        Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Div,
        DivAssign, Mul, MulAssign, Neg, Not, Rem, RemAssign, Sub, SubAssign,
    },
};

use bstr::ByteSlice;
use derive_more::{
    Binary, Display, From, FromStr, LowerHex, Octal, Product, Shl, ShlAssign, Shr, ShrAssign, Sum,
    UpperHex,
};
use malachite::{
    num::{
        arithmetic::traits::{Abs, FloorSqrt, Mod, ModPow, Parity, Sign},
        conversion::traits::{Digits, FromStringBase, OverflowingInto, RoundingInto},
        logic::traits::{CountOnes, SignificantBits},
    },
    rounding_modes::RoundingMode,
    Integer, Natural, Rational,
};
use num_traits::{Num, One, Pow, Signed, ToPrimitive, Zero};

#[repr(transparent)]
#[derive(
    Debug,
    Hash,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Clone,
    FromStr,
    Display,
    Binary,
    Octal,
    LowerHex,
    UpperHex,
    From,
    Shr,
    Shl,
    Sum,
    Product,
    ShlAssign,
    ShrAssign,
)]
#[from(forward)]
pub struct BigInt(Integer);

macro_rules! to_primitive_int {
    ($fn:ident, $ret:ty) => {
        fn $fn(&self) -> Option<$ret> {
            match self.inner().overflowing_into() {
                (val, false) => Some(val),
                _ => None,
            }
        }
    };
}

macro_rules! to_primitive_float {
    ($fn:ident, $ret:ty) => {
        fn $fn(&self) -> Option<$ret> {
            let val: $ret = self.inner().rounding_into(RoundingMode::Floor);
            val.is_finite().then_some(val)
        }
    };
}

impl ToPrimitive for BigInt {
    to_primitive_int!(to_isize, isize);
    to_primitive_int!(to_i8, i8);
    to_primitive_int!(to_i16, i16);
    to_primitive_int!(to_i32, i32);
    to_primitive_int!(to_i64, i64);
    to_primitive_int!(to_i128, i128);
    to_primitive_int!(to_usize, usize);
    to_primitive_int!(to_u8, u8);
    to_primitive_int!(to_u16, u16);
    to_primitive_int!(to_u32, u32);
    to_primitive_int!(to_u64, u64);
    to_primitive_int!(to_u128, u128);

    to_primitive_float!(to_f32, f32);
    to_primitive_float!(to_f64, f64);
}

macro_rules! try_to_primitive {
    ($fn:ident, $ret:ty) => {
        impl TryFrom<&BigInt> for $ret {
            type Error = ();

            fn try_from(value: &BigInt) -> Result<Self, Self::Error> {
                value.$fn().ok_or(())
            }
        }
        impl TryFrom<BigInt> for $ret {
            type Error = ();

            fn try_from(value: BigInt) -> Result<Self, Self::Error> {
                value.$fn().ok_or(())
            }
        }
    };
}

try_to_primitive!(to_isize, isize);
try_to_primitive!(to_i8, i8);
try_to_primitive!(to_i16, i16);
try_to_primitive!(to_i32, i32);
try_to_primitive!(to_i64, i64);
try_to_primitive!(to_i128, i128);
try_to_primitive!(to_usize, usize);
try_to_primitive!(to_u8, u8);
try_to_primitive!(to_u16, u16);
try_to_primitive!(to_u32, u32);
try_to_primitive!(to_u64, u64);
try_to_primitive!(to_u128, u128);

macro_rules! impl_unary_op {
    ($trait:tt, $fn:ident) => {
        impl $trait for BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self) -> BigInt {
                BigInt(self.0.$fn())
            }
        }
        impl $trait for &BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self) -> BigInt {
                BigInt(self.inner().$fn())
            }
        }
    };
}

impl_unary_op!(Neg, neg);
impl_unary_op!(Not, not);

macro_rules! impl_binary_op {
    ($trait:tt, $fn:ident) => {
        impl_binary_op!($trait, $fn, $trait::$fn);
    };
    ($trait:tt, $fn:ident, $innerfn:path) => {
        impl $trait<BigInt> for BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self, rhs: BigInt) -> BigInt {
                BigInt($innerfn(self.0, rhs.0))
            }
        }
        impl $trait<&BigInt> for BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self, rhs: &BigInt) -> BigInt {
                BigInt($innerfn(self.0, rhs.inner()))
            }
        }
        impl $trait<BigInt> for &BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self, rhs: BigInt) -> BigInt {
                BigInt($innerfn(self.inner(), rhs.0))
            }
        }
        impl $trait<&BigInt> for &BigInt {
            type Output = BigInt;
            #[inline(always)]
            fn $fn(self, rhs: &BigInt) -> BigInt {
                BigInt($innerfn(self.inner(), rhs.inner()))
            }
        }
    };
}

impl_binary_op!(Add, add);
impl_binary_op!(Sub, sub);
impl_binary_op!(BitAnd, bitand);
impl_binary_op!(BitOr, bitor);
impl_binary_op!(BitXor, bitxor);
impl_binary_op!(Mul, mul);
impl_binary_op!(Div, div);
impl_binary_op!(Rem, rem);

macro_rules! impl_assign_op {
    ($trait:tt, $fn:ident) => {
        impl $trait for BigInt {
            fn $fn(&mut self, rhs: BigInt) {
                self.0.$fn(rhs.0)
            }
        }
        impl $trait<&BigInt> for BigInt {
            fn $fn(&mut self, rhs: &BigInt) {
                self.0.$fn(rhs.inner())
            }
        }
    };
}

impl_assign_op!(AddAssign, add_assign);
impl_assign_op!(SubAssign, sub_assign);
impl_assign_op!(BitAndAssign, bitand_assign);
impl_assign_op!(BitOrAssign, bitor_assign);
impl_assign_op!(BitXorAssign, bitxor_assign);
impl_assign_op!(MulAssign, mul_assign);
impl_assign_op!(DivAssign, div_assign);
impl_assign_op!(RemAssign, rem_assign);

impl Zero for BigInt {
    fn zero() -> Self {
        Self(<Integer as malachite::num::basic::traits::Zero>::ZERO)
    }

    fn is_zero(&self) -> bool {
        *self == Self::zero()
    }
}

impl One for BigInt {
    fn one() -> Self {
        Self(<Integer as malachite::num::basic::traits::One>::ONE)
    }
}

impl Num for BigInt {
    type FromStrRadixErr = ();

    fn from_str_radix(str: &str, radix: u32) -> Result<Self, Self::FromStrRadixErr> {
        debug_assert!(radix <= 16);
        Integer::from_string_base(radix as u8, str)
            .map(|x| Self(x))
            .ok_or(())
    }
}

impl Signed for BigInt {
    fn abs(&self) -> Self {
        Self(self.inner().abs())
    }

    fn abs_sub(&self, other: &Self) -> Self {
        if self <= other {
            Self::zero()
        } else {
            self - other
        }
    }

    fn signum(&self) -> Self {
        match self.0.sign() {
            Ordering::Less => Self::negative_one(),
            Ordering::Equal => Self::zero(),
            Ordering::Greater => Self::one(),
        }
    }

    fn is_positive(&self) -> bool {
        self.0.sign() == Ordering::Greater
    }

    fn is_negative(&self) -> bool {
        self.0.sign() == Ordering::Less
    }
}

impl Pow<u64> for BigInt {
    type Output = BigInt;

    fn pow(self, rhs: u64) -> Self::Output {
        BigInt(<Integer as malachite::num::arithmetic::traits::Pow<u64>>::pow(self.0, rhs))
    }
}
impl Pow<u64> for &BigInt {
    type Output = BigInt;

    fn pow(self, rhs: u64) -> Self::Output {
        BigInt(<&Integer as malachite::num::arithmetic::traits::Pow<u64>>::pow(self.inner(), rhs))
    }
}

impl num_integer::Integer for BigInt {
    fn div_floor(&self, other: &Self) -> Self {
        Self(self.inner().div(other.inner()))
    }

    fn mod_floor(&self, other: &Self) -> Self {
        Self(self.inner().mod_op(other.inner()))
    }

    fn gcd(&self, other: &Self) -> Self {
        todo!()
    }

    fn lcm(&self, other: &Self) -> Self {
        todo!()
    }

    fn divides(&self, other: &Self) -> bool {
        todo!()
    }

    fn is_multiple_of(&self, other: &Self) -> bool {
        todo!()
    }

    fn is_even(&self) -> bool {
        self.0.even()
    }

    fn is_odd(&self) -> bool {
        self.0.odd()
    }

    fn div_rem(&self, other: &Self) -> (Self, Self) {
        todo!()
    }

    fn div_ceil(&self, other: &Self) -> Self {
        let (q, r) = self.div_mod_floor(other);
        if r.is_zero() {
            q
        } else {
            q + Self::one()
        }
    }

    fn gcd_lcm(&self, other: &Self) -> (Self, Self) {
        (self.gcd(other), self.lcm(other))
    }

    fn extended_gcd(&self, other: &Self) -> num_integer::ExtendedGcd<Self>
    where
        Self: Clone,
    {
        let mut s = (Self::zero(), Self::one());
        let mut t = (Self::one(), Self::zero());
        let mut r = (other.clone(), self.clone());

        while !r.0.is_zero() {
            let q = r.1.clone() / r.0.clone();
            let f = |mut r: (Self, Self)| {
                std::mem::swap(&mut r.0, &mut r.1);
                r.0 = r.0 - q.clone() * r.1.clone();
                r
            };
            r = f(r);
            s = f(s);
            t = f(t);
        }

        if r.1 >= Self::zero() {
            num_integer::ExtendedGcd {
                gcd: r.1,
                x: s.1,
                y: t.1,
            }
        } else {
            num_integer::ExtendedGcd {
                gcd: Self::zero() - r.1,
                x: Self::zero() - s.1,
                y: Self::zero() - t.1,
            }
        }
    }

    fn extended_gcd_lcm(&self, other: &Self) -> (num_integer::ExtendedGcd<Self>, Self)
    where
        Self: Clone + Signed,
    {
        (self.extended_gcd(other), self.lcm(other))
    }

    fn div_mod_floor(&self, other: &Self) -> (Self, Self) {
        (self.div_floor(other), self.mod_floor(other))
    }

    fn next_multiple_of(&self, other: &Self) -> Self
    where
        Self: Clone,
    {
        let m = self.mod_floor(other);
        self.clone()
            + if m.is_zero() {
                Self::zero()
            } else {
                other.clone() - m
            }
    }

    fn prev_multiple_of(&self, other: &Self) -> Self
    where
        Self: Clone,
    {
        self.clone() - self.mod_floor(other)
    }
}

impl BigInt {
    fn inner(&self) -> &Integer {
        &self.0
    }

    pub fn negative_one() -> Self {
        Self(<Integer as malachite::num::basic::traits::NegativeOne>::NEGATIVE_ONE)
    }

    pub fn true_div(&self, other: &Self) -> f64 {
        assert!(!other.is_zero());
        let rational = malachite::Rational::from_integers_ref(self.inner(), other.inner());
        rational.rounding_into(RoundingMode::Down)
    }

    /// Generates the base-2 logarithm of a BigInt `x`
    pub fn int_log2(&self) -> f64 {
        // log2(x) = log2(2^n * 2^-n * x) = n + log2(x/2^n)
        // If we set 2^n to be the greatest power of 2 below x, then x/2^n is in [1, 2), and can
        // thus be converted into a float.
        let n = self.bits() as u32 - 1;
        let frac = Rational::from_integers_ref(self.inner(), Self::from(2).pow(n.into()).inner());
        let float: f64 = frac.rounding_into(RoundingMode::Down);
        f64::from(n) + float.log2()
    }

    pub fn getrandbits<F>(mut k: usize, mut f: F) -> Self
    where
        F: FnMut() -> u64,
    {
        let words = (k - 1) / 32 + 1;
        let wordarray = (0..words)
            .map(|_| {
                let mut word = f();
                if k < 64 {
                    word >>= 64 - k;
                }
                k = k.wrapping_sub(64);
                word
            })
            .collect::<Vec<_>>();
    
        let abs = Natural::from_owned_limbs_asc(wordarray);
        Integer::from_sign_and_abs(true, abs).into()
    }

    pub fn into_limbs_asc(self) -> Vec<u64> {
        self.0.into_twos_complement_limbs_asc()
    }

    pub fn sqrt(self) -> Self {
        Self(self.0.floor_sqrt())
    }

    pub fn rounding_from(value: f64) -> Option<Self> {
        (value.is_finite() && !value.is_nan())
            .then(|| Self(value.rounding_into(RoundingMode::Down)))
    }

    pub fn rational_of(value: f64) -> Option<(Self, Self)> {
        let rational = malachite::Rational::try_from(value).ok()?;
        let (numerator, denominator) = rational.into_numerator_and_denominator();
        Some((Self(numerator.into()), Self(denominator.into())))
    }

    pub fn modpow(&self, exponent: &Self, modulus: &Self) -> Self {
        assert!(
            !exponent.is_negative(),
            "negative exponentiation is not supported!"
        );
        assert!(
            !modulus.is_zero(),
            "attempt to calculate with zero modulus!"
        );

        let mut abs = self.inner().unsigned_abs_ref().mod_pow(
            exponent.inner().unsigned_abs_ref(),
            modulus.inner().unsigned_abs_ref(),
        );

        if abs == <Natural as malachite::num::basic::traits::Zero>::ZERO {
            return Self::zero();
        }

        let sign = modulus.is_positive();

        if self.is_negative() && exponent.inner().odd() != !sign {
            abs = modulus.inner().unsigned_abs_ref() - abs;
        }

        Self(Integer::from_sign_and_abs(sign, abs))
    }

    pub fn bits(&self) -> u64 {
        self.inner().significant_bits()
    }

    pub fn count_ones(&self) -> u64 {
        self.inner().unsigned_abs_ref().count_ones()
    }

    pub fn sign(&self) -> Ordering {
        self.inner().sign()
    }

    pub fn to_bytes_be(&self) -> (Ordering, Vec<u8>) {
        (
            self.sign(),
            self.inner().unsigned_abs_ref().to_digits_desc(&u8::MAX),
        )
    }

    pub fn to_bytes_le(&self) -> (Ordering, Vec<u8>) {
        (
            self.sign(),
            self.inner().unsigned_abs_ref().to_digits_asc(&u8::MAX),
        )
    }

    pub fn to_signed_bytes_be(&self) -> Vec<u8> {
        todo!()
    }

    pub fn to_signed_bytes_le(&self) -> Vec<u8> {
        todo!()
    }

    pub fn from_signed_bytes_be(digits: &[u8]) -> Self {
        todo!()
    }

    pub fn from_signed_bytes_le(digits: &[u8]) -> Self {
        todo!()
    }

    pub fn from_bytes_be(sign: bool, digits: &[u8]) -> Self {
        todo!()
    }

    pub fn from_bytes_le(sign: bool, digits: &[u8]) -> Self {
        todo!()
    }

    pub fn to_str_radix(&self, radix: u32) -> String {
        todo!()
    }
}

pub fn bytes_to_int(lit: &[u8], mut base: u8) -> Option<BigInt> {
    // split sign
    let mut lit = lit.trim();
    let sign = match lit.first()? {
        b'+' => {
            lit = &lit[1..];
            true
        }
        b'-' => {
            lit = &lit[1..];
            false
        }
        _ => true,
    };

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
        let s = unsafe { std::str::from_utf8_unchecked(lit) };
        let uint = Natural::from_string_base(base, s)?;
        BigInt(Integer::from_sign_and_abs(sign, uint))
    };
    Some(number)
}

#[inline]
pub fn detect_base(c: &u8) -> Option<u8> {
    let base = match c {
        b'x' | b'X' => 16,
        b'b' | b'B' => 2,
        b'o' | b'O' => 8,
        _ => return None,
    };
    Some(base)
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

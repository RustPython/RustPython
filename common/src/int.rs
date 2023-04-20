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
        arithmetic::traits::{
            Abs, DivMod, DivRem, DivisibleBy, ExtendedGcd, FloorSqrt, Gcd, Lcm, Mod, ModPow,
            Parity, Sign,
        },
        conversion::traits::{
            FromStringBase, OverflowingInto, PowerOf2Digits, RoundingInto, ToStringBase,
        },
        logic::traits::{CountOnes, SignificantBits},
    },
    rounding_modes::RoundingMode,
    Integer, Natural, Rational,
};
use num_traits::{Num, One, Pow, Signed, ToPrimitive, Zero};

#[repr(transparent)]
#[derive(
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

impl std::fmt::Debug for BigInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

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
            let val: $ret = self.inner().rounding_into(RoundingMode::Down);
            if val == <$ret>::MAX || val == <$ret>::MIN {
                (self.inner() == &val).then_some(val)
            } else {
                Some(val)
            }
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
        Integer::from_string_base(radix as u8, str)
            .map(Self)
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
        let abs = self
            .inner()
            .unsigned_abs_ref()
            .gcd(other.inner().unsigned_abs_ref());
        Integer::from_sign_and_abs(true, abs).into()
    }

    fn lcm(&self, other: &Self) -> Self {
        let abs = self
            .inner()
            .unsigned_abs_ref()
            .lcm(other.inner().unsigned_abs_ref());
        Integer::from_sign_and_abs(true, abs).into()
    }

    fn divides(&self, other: &Self) -> bool {
        self.is_multiple_of(other)
    }

    fn is_multiple_of(&self, other: &Self) -> bool {
        self.inner().divisible_by(other.inner())
    }

    fn is_even(&self) -> bool {
        self.0.even()
    }

    fn is_odd(&self) -> bool {
        self.0.odd()
    }

    fn div_rem(&self, other: &Self) -> (Self, Self) {
        let (div, rem) = self.inner().div_rem(other.inner());
        (div.into(), rem.into())
    }

    fn extended_gcd(&self, other: &Self) -> num_integer::ExtendedGcd<Self>
    where
        Self: Clone,
    {
        let (gcd, x, y) = self.inner().extended_gcd(other.inner());
        let gcd = Integer::from_sign_and_abs(true, gcd).into();
        num_integer::ExtendedGcd {
            gcd,
            x: x.into(),
            y: y.into(),
        }
    }

    fn div_mod_floor(&self, other: &Self) -> (Self, Self) {
        let (div, m) = self.inner().div_mod(other.inner());
        (div.into(), m.into())
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
        debug_assert!(!other.is_zero());
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
        F: FnMut() -> u32,
    {
        // fast path
        if k <= 32 {
            return Integer::from_sign_and_abs(true, Natural::from(f() >> (32 - k))).into();
        }

        let words = (k - 1) / 32 + 1;
        let mut wordarray = (0..words)
            .map(|_| {
                let mut word = f();
                if k < 32 {
                    word >>= 32 - k;
                }
                k = k.wrapping_sub(32);
                word
            })
            .collect::<Vec<_>>();
        // padding
        if words.odd() {
            wordarray.push(0);
        }
        let (_, slice, _) = unsafe { wordarray.align_to::<u64>() };

        let abs = Natural::from_limbs_asc(slice);
        Integer::from_sign_and_abs(true, abs).into()
    }

    pub fn into_limbs_asc(self) -> Vec<u64> {
        self.0.into_twos_complement_limbs_asc()
    }

    pub fn to_u32_digits(&self) -> (Ordering, Vec<u32>) {
        let abs = self.inner().unsigned_abs_ref();
        let digits: Vec<u32> = abs.to_power_of_2_digits_asc(32);
        (self.sign(), digits)
    }

    pub fn sqrt(self) -> Self {
        Self(self.0.floor_sqrt())
    }

    pub fn rounding_from(value: f64) -> Option<Self> {
        (value.is_finite() && !value.is_nan())
            .then(|| Self(value.rounding_into(RoundingMode::Down)))
    }

    pub fn rational_of(value: f64) -> Option<(Self, Self)> {
        let sign = value >= 0.0;
        let rational = malachite::Rational::try_from(value).ok()?;
        let (numerator, denominator) = rational.into_numerator_and_denominator();
        Some((
            Self(Integer::from_sign_and_abs(sign, numerator)),
            Self(denominator.into()),
        ))
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
            self.inner().unsigned_abs_ref().to_power_of_2_digits_desc(8),
        )
    }

    pub fn to_bytes_le(&self) -> (Ordering, Vec<u8>) {
        (
            self.sign(),
            self.inner().unsigned_abs_ref().to_power_of_2_digits_asc(8),
        )
    }

    pub fn from_bytes_be(sign: bool, digits: &[u8]) -> Self {
        // SAFETY: &[u8] cannot have any digit greater than 2^8
        let abs = unsafe {
            Natural::from_power_of_2_digits_desc(8, digits.iter().cloned()).unwrap_unchecked()
        };
        Integer::from_sign_and_abs(sign, abs).into()
    }

    pub fn from_bytes_le(sign: bool, digits: &[u8]) -> Self {
        // SAFETY: &[u8] cannot have any digit greater than 2^8
        let abs = unsafe {
            Natural::from_power_of_2_digits_asc(8, digits.iter().cloned()).unwrap_unchecked()
        };
        Integer::from_sign_and_abs(sign, abs).into()
    }

    pub fn to_signed_bytes_be(&self) -> Vec<u8> {
        let limbs = self.inner().to_twos_complement_limbs_asc();
        let uint = Natural::from_owned_limbs_asc(limbs);
        uint.to_power_of_2_digits_desc(8)
    }

    pub fn to_signed_bytes_le(&self) -> Vec<u8> {
        let limbs = self.inner().to_twos_complement_limbs_asc();
        let uint = Natural::from_owned_limbs_asc(limbs);
        uint.to_power_of_2_digits_asc(8)
    }

    pub fn from_signed_bytes_be(digits: &[u8]) -> Self {
        // SAFETY: &[u8] cannot have any digit greater than 2^8
        let uint = unsafe {
            Natural::from_power_of_2_digits_desc(8, digits.iter().cloned()).unwrap_unchecked()
        };
        let limbs = uint.into_limbs_asc();
        Integer::from_owned_twos_complement_limbs_asc(limbs).into()
    }

    pub fn from_signed_bytes_le(digits: &[u8]) -> Self {
        // SAFETY: &[u8] cannot have any digit greater than 2^8
        let uint = unsafe {
            Natural::from_power_of_2_digits_asc(8, digits.iter().cloned()).unwrap_unchecked()
        };
        let limbs = uint.into_limbs_asc();
        Integer::from_owned_twos_complement_limbs_asc(limbs).into()
    }

    pub fn to_str_radix(&self, radix: u32) -> String {
        self.inner().to_string_base(radix as u8)
    }
}

pub fn bytes_to_int(lit: &[u8], mut radix: u8) -> Option<BigInt> {
    let mut lit = lit.trim();

    let mut first = *lit.first()?;

    let sign = match first {
        b'+' | b'-' => {
            let sign = first == b'+';
            lit = &lit[1..];
            first = *lit.first()?;
            sign
        }
        _ => true,
    };

    let detected = || -> Option<u8> {
        if first != b'0' || lit.len() < 3 {
            return None;
        }
        match unsafe { lit.get_unchecked(1) } {
            b'x' | b'X' => Some(16),
            b'b' | b'B' => Some(2),
            b'o' | b'O' => Some(8),
            _ => None,
        }
    }();

    let mut leading_zero = false;

    if let Some(detected) = detected {
        lit = &lit[2..];
        if radix == 0 {
            radix = detected;
        } else if radix != detected {
            return None;
        }
    } else {
        if radix == 0 {
            radix = 10;
            leading_zero = first == b'0';
        }
        // start with underscore
        if first == b'_' {
            return None;
        }
    }

    let mut v: Vec<u8>;
    if lit.iter().any(|&x| x == b'_') {
        // erase underscores
        v = Vec::with_capacity(lit.len());
        let mut is_prev_underscore = false;
        for &x in lit {
            let is_underscore = x == b'_';
            match (is_underscore, is_prev_underscore) {
                // multiple underscores
                (true, true) => return None,
                (true, false) => (),
                (false, _) => v.push(x),
            }
            is_prev_underscore = is_underscore;
        }
        // end with underscore
        if is_prev_underscore {
            return None;
        }
        lit = v.as_slice();
    }

    let s = std::str::from_utf8(lit).ok()?;
    let abs = Natural::from_string_base(radix, s)?;

    if abs != <Natural as malachite::num::basic::traits::Zero>::ZERO && leading_zero {
        return None;
    }

    Some(Integer::from_sign_and_abs(sign, abs).into())
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

#[test]
fn test_bytes_convertions() {
    let uint = Natural::from(258u32);
    let bytes: Vec<u8> = uint.to_power_of_2_digits_asc(8);
    assert_eq!(bytes, vec![2, 1]);
    let bytes: Vec<u8> = uint.to_power_of_2_digits_desc(8);
    assert_eq!(bytes, vec![1, 2]);
}

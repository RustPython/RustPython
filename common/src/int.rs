use bstr::ByteSlice;
use derive_more::{
    Add, AddAssign, Binary, BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign,
    Display, Div, DivAssign, From, LowerHex, Mul, MulAssign, Neg, Not, Octal, Product, Rem,
    RemAssign, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign, Sum, UpperHex,
};
use malachite::{
    num::conversion::traits::{FromStringBase, OverflowingInto, RoundingInto},
    Integer, Natural, rounding_modes::RoundingMode,
};
use num_traits::{ToPrimitive, Zero};

#[repr(transparent)]
#[derive(
    Debug,
    PartialEq,
    PartialOrd,
    Clone,
    Display,
    Binary,
    Octal,
    LowerHex,
    UpperHex,
    From,
    Not,
    Neg,
    Add,
    Sub,
    BitAnd,
    BitOr,
    BitXor,
    Mul,
    Div,
    Rem,
    Shr,
    Shl,
    Sum,
    Product,
    AddAssign,
    SubAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    MulAssign,
    DivAssign,
    RemAssign,
    ShlAssign,
    ShrAssign,
)]
#[from(forward)]
#[mul(forward)]
#[div(forward)]
#[rem(forward)]
#[mul_assign(forward)]
#[div_assign(forward)]
#[rem_assign(forward)]
pub struct BigInt(Integer);

macro_rules! to_primitive_int {
    ($fn:ident, $ret:ty) => {
        fn $fn(&self) -> Option<$ret> {
            match (&self.0).overflowing_into() {
                (val, false) => Some(val),
                _ => None,
            }
        }
    };
}

macro_rules! to_primitive_float {
    ($fn:ident, $ret:ty) => {
        fn $fn(&self) -> Option<$ret> {
            let val: $ret = (&self.0).rounding_into(RoundingMode::Floor);
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
    #[cfg(has_i128)]
    to_primitive_int!(to_i128, i128);
    to_primitive_int!(to_usize, usize);
    to_primitive_int!(to_u8, u8);
    to_primitive_int!(to_u16, u16);
    to_primitive_int!(to_u32, u32);
    to_primitive_int!(to_u64, u64);
    #[cfg(has_u128)]
    to_primitive_int!(to_u128, u128);

    to_primitive_float!(to_f32, f32);
    to_primitive_float!(to_f64, f64);
}

// impl TryFrom<f32> for BigInt {
//     type Error = ();

//     fn try_from(value: f32) -> Result<Self, Self::Error> {
//         Ok(value.into())
//     }
// }
impl TryFrom<&BigInt> for i8 {
    type Error = ();

    fn try_from(value: &BigInt) -> Result<Self, Self::Error> {
        value.to_i8().ok_or(())
    }
}

macro_rules! try_to_primitive {
    ($fn:ident, $ret:ty) => {
        impl TryFrom<&BigInt> for $ret {
            type Error = ();

            fn try_from(value: &BigInt) -> Result<Self, Self::Error> {
                value.$fn().ok_or(())
            }
        }
    };
}

try_to_primitive!(to_isize, isize);
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

impl Zero for BigInt {
    fn zero() -> Self {
        Self(<Integer as malachite::num::basic::traits::Zero>::ZERO)
    }

    fn is_zero(&self) -> bool {
        self.eq(&Self::zero())
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

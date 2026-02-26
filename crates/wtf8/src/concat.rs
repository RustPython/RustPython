use alloc::borrow::{Cow, ToOwned};
use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;
use fmt::Write;

use crate::{CodePoint, Wtf8, Wtf8Buf};

impl fmt::Write for Wtf8Buf {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

/// Trait for types that can be appended to a [`Wtf8Buf`], preserving surrogates.
pub trait Wtf8Concat {
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf);
}

impl Wtf8Concat for Wtf8 {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push_wtf8(self);
    }
}

impl Wtf8Concat for Wtf8Buf {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push_wtf8(self);
    }
}

impl Wtf8Concat for str {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push_str(self);
    }
}

impl Wtf8Concat for String {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push_str(self);
    }
}

impl Wtf8Concat for char {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push_char(*self);
    }
}

impl Wtf8Concat for CodePoint {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        buf.push(*self);
    }
}

/// Wrapper that appends a [`fmt::Display`] value to a [`Wtf8Buf`].
///
/// Note: This goes through UTF-8 formatting, so lone surrogates in the
/// display output will be replaced with U+FFFD. Use direct [`Wtf8Concat`]
/// impls for surrogate-preserving concatenation.
#[allow(dead_code)]
pub struct DisplayAsWtf8<T>(pub T);

impl<T: fmt::Display> Wtf8Concat for DisplayAsWtf8<T> {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        write!(buf, "{}", self.0).unwrap();
    }
}

macro_rules! impl_wtf8_concat_for_int {
    ($($t:ty),*) => {
        $(impl Wtf8Concat for $t {
            #[inline]
            fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
                write!(buf, "{}", self).unwrap();
            }
        })*
    };
}

impl_wtf8_concat_for_int!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);

impl<T: Wtf8Concat + ?Sized> Wtf8Concat for &T {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        (**self).fmt_wtf8(buf);
    }
}

impl<T: Wtf8Concat + ?Sized> Wtf8Concat for &mut T {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        (**self).fmt_wtf8(buf);
    }
}

impl<T: Wtf8Concat + ?Sized> Wtf8Concat for Box<T> {
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        (**self).fmt_wtf8(buf);
    }
}

impl<T: Wtf8Concat + ?Sized> Wtf8Concat for Cow<'_, T>
where
    T: ToOwned,
{
    #[inline]
    fn fmt_wtf8(&self, buf: &mut Wtf8Buf) {
        (**self).fmt_wtf8(buf);
    }
}

/// Concatenate values into a [`Wtf8Buf`], preserving surrogates.
///
/// Each argument must implement [`Wtf8Concat`]. String literals (`&str`),
/// [`Wtf8`], [`Wtf8Buf`], [`char`], and [`CodePoint`] are all supported.
///
/// ```
/// use rustpython_wtf8::Wtf8Buf;
/// let name = "world";
/// let result = rustpython_wtf8::wtf8_concat!("hello, ", name, "!");
/// assert_eq!(result, Wtf8Buf::from("hello, world!"));
/// ```
#[macro_export]
macro_rules! wtf8_concat {
    ($($arg:expr),* $(,)?) => {{
        let mut buf = $crate::Wtf8Buf::new();
        $($crate::Wtf8Concat::fmt_wtf8(&$arg, &mut buf);)*
        buf
    }};
}

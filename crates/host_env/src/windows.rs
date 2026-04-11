use rustpython_wtf8::Wtf8;
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
};

/// _MAX_ENV from Windows CRT stdlib.h - maximum environment variable size
pub const _MAX_ENV: usize = 32767;

pub trait ToWideString {
    fn to_wide(&self) -> Vec<u16>;
    fn to_wide_with_nul(&self) -> Vec<u16>;
}

impl<T> ToWideString for T
where
    T: AsRef<OsStr>,
{
    fn to_wide(&self) -> Vec<u16> {
        self.as_ref().encode_wide().collect()
    }
    fn to_wide_with_nul(&self) -> Vec<u16> {
        self.as_ref().encode_wide().chain(Some(0)).collect()
    }
}

impl ToWideString for OsStr {
    fn to_wide(&self) -> Vec<u16> {
        self.encode_wide().collect()
    }
    fn to_wide_with_nul(&self) -> Vec<u16> {
        self.encode_wide().chain(Some(0)).collect()
    }
}

impl ToWideString for Wtf8 {
    fn to_wide(&self) -> Vec<u16> {
        self.encode_wide().collect()
    }
    fn to_wide_with_nul(&self) -> Vec<u16> {
        self.encode_wide().chain(Some(0)).collect()
    }
}

pub trait FromWideString
where
    Self: Sized,
{
    fn from_wides_until_nul(wide: &[u16]) -> Self;
}
impl FromWideString for OsString {
    fn from_wides_until_nul(wide: &[u16]) -> OsString {
        let len = wide.iter().take_while(|&&c| c != 0).count();
        OsString::from_wide(&wide[..len])
    }
}

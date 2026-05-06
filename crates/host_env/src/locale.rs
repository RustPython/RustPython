use alloc::vec::Vec;
use core::{ffi::CStr, ptr};

#[cfg(windows)]
#[repr(C)]
struct RawLconv {
    decimal_point: *mut libc::c_char,
    thousands_sep: *mut libc::c_char,
    grouping: *mut libc::c_char,
    int_curr_symbol: *mut libc::c_char,
    currency_symbol: *mut libc::c_char,
    mon_decimal_point: *mut libc::c_char,
    mon_thousands_sep: *mut libc::c_char,
    mon_grouping: *mut libc::c_char,
    positive_sign: *mut libc::c_char,
    negative_sign: *mut libc::c_char,
    int_frac_digits: libc::c_char,
    frac_digits: libc::c_char,
    p_cs_precedes: libc::c_char,
    p_sep_by_space: libc::c_char,
    n_cs_precedes: libc::c_char,
    n_sep_by_space: libc::c_char,
    p_sign_posn: libc::c_char,
    n_sign_posn: libc::c_char,
}

#[cfg(windows)]
unsafe extern "C" {
    fn localeconv() -> *mut RawLconv;
}

#[cfg(unix)]
use libc::localeconv;

#[derive(Debug, Clone)]
pub struct LocaleConv {
    pub decimal_point: Vec<u8>,
    pub thousands_sep: Vec<u8>,
    pub grouping: Vec<libc::c_char>,
    pub int_curr_symbol: Vec<u8>,
    pub currency_symbol: Vec<u8>,
    pub mon_decimal_point: Vec<u8>,
    pub mon_thousands_sep: Vec<u8>,
    pub mon_grouping: Vec<libc::c_char>,
    pub positive_sign: Vec<u8>,
    pub negative_sign: Vec<u8>,
    pub int_frac_digits: libc::c_char,
    pub frac_digits: libc::c_char,
    pub p_cs_precedes: libc::c_char,
    pub p_sep_by_space: libc::c_char,
    pub n_cs_precedes: libc::c_char,
    pub n_sep_by_space: libc::c_char,
    pub p_sign_posn: libc::c_char,
    pub n_sign_posn: libc::c_char,
}

fn copy_cstr(ptr: *const libc::c_char) -> Vec<u8> {
    if ptr.is_null() {
        Vec::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec()
    }
}

fn copy_grouping(ptr: *const libc::c_char) -> Vec<libc::c_char> {
    if ptr.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = ptr;
    unsafe {
        while ![0, libc::c_char::MAX].contains(&*cur) {
            out.push(*cur);
            cur = cur.add(1);
        }
    }
    out
}

pub fn localeconv_data() -> LocaleConv {
    let lc = unsafe { localeconv() };
    unsafe {
        LocaleConv {
            decimal_point: copy_cstr((*lc).decimal_point),
            thousands_sep: copy_cstr((*lc).thousands_sep),
            grouping: copy_grouping((*lc).grouping),
            int_curr_symbol: copy_cstr((*lc).int_curr_symbol),
            currency_symbol: copy_cstr((*lc).currency_symbol),
            mon_decimal_point: copy_cstr((*lc).mon_decimal_point),
            mon_thousands_sep: copy_cstr((*lc).mon_thousands_sep),
            mon_grouping: copy_grouping((*lc).mon_grouping),
            positive_sign: copy_cstr((*lc).positive_sign),
            negative_sign: copy_cstr((*lc).negative_sign),
            int_frac_digits: (*lc).int_frac_digits,
            frac_digits: (*lc).frac_digits,
            p_cs_precedes: (*lc).p_cs_precedes,
            p_sep_by_space: (*lc).p_sep_by_space,
            n_cs_precedes: (*lc).n_cs_precedes,
            n_sep_by_space: (*lc).n_sep_by_space,
            p_sign_posn: (*lc).p_sign_posn,
            n_sign_posn: (*lc).n_sign_posn,
        }
    }
}

pub fn strcoll(string1: &CStr, string2: &CStr) -> libc::c_int {
    unsafe { libc::strcoll(string1.as_ptr(), string2.as_ptr()) }
}

pub fn strxfrm(string: &CStr, _initial_len: usize) -> Vec<u8> {
    let len = unsafe { libc::strxfrm(ptr::null_mut(), string.as_ptr(), 0) };
    let mut buff = vec![0u8; len + 1];
    unsafe {
        libc::strxfrm(buff.as_mut_ptr() as _, string.as_ptr(), buff.len());
    }
    buff.truncate(len);
    buff
}

pub fn setlocale(category: i32, locale: Option<&CStr>) -> Option<Vec<u8>> {
    let result = unsafe {
        match locale {
            None => libc::setlocale(category, ptr::null()),
            Some(locale) => libc::setlocale(category, locale.as_ptr()),
        }
    };
    (!result.is_null()).then(|| unsafe { CStr::from_ptr(result) }.to_bytes().to_vec())
}

#[cfg(windows)]
pub fn acp() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetACP() }
}

#[cfg(windows)]
pub fn decode_ansi_bytes(bytes: &[u8]) -> Option<String> {
    use core::ptr;
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};

    if bytes.is_empty() {
        return Some(String::new());
    }
    let len_i32 = i32::try_from(bytes.len()).ok()?;

    let len =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len_i32, ptr::null_mut(), 0) };
    if len <= 0 {
        return None;
    }
    let mut wide = vec![0u16; len as usize];
    unsafe {
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len_i32, wide.as_mut_ptr(), len);
    }
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(all(
    unix,
    not(any(target_os = "ios", target_os = "android", target_os = "redox"))
))]
pub fn nl_langinfo_codeset() -> Option<Vec<u8>> {
    let codeset = unsafe { libc::nl_langinfo(libc::CODESET) };
    (!codeset.is_null()).then(|| unsafe { CStr::from_ptr(codeset) }.to_bytes().to_vec())
}

use alloc::vec::Vec;
use core::{ffi::CStr, ptr};

pub use libc::{LC_ALL, LC_COLLATE, LC_CTYPE, LC_MONETARY, LC_NUMERIC, LC_TIME};

// CPython uses the wide-character APIs after btowc() on Apple platforms.
// Darwin's wint_t is int; libc does not expose these declarations.
#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn btowc(ch: libc::c_int) -> libc::c_int;
    fn iswalnum(ch: libc::c_int) -> libc::c_int;
    fn towlower(ch: libc::c_int) -> libc::c_int;
    fn towupper(ch: libc::c_int) -> libc::c_int;
}

/// Classify a byte using the current LC_CTYPE locale.
pub fn byte_isalnum(ch: u8) -> bool {
    // ctype functions require EOF or an unsigned-char value, never a signed byte.
    #[cfg(not(target_vendor = "apple"))]
    unsafe {
        libc::isalnum(ch.into()) != 0
    }
    #[cfg(target_vendor = "apple")]
    unsafe {
        iswalnum(btowc(ch.into())) != 0
    }
}

/// Lowercase a byte using the current LC_CTYPE locale.
pub fn byte_tolower(ch: u8) -> u32 {
    #[cfg(not(target_vendor = "apple"))]
    unsafe {
        libc::tolower(ch.into()) as u32
    }
    #[cfg(target_vendor = "apple")]
    unsafe {
        towlower(btowc(ch.into())) as u32
    }
}

/// Uppercase a byte using the current LC_CTYPE locale.
pub fn byte_toupper(ch: u8) -> u32 {
    #[cfg(not(target_vendor = "apple"))]
    unsafe {
        libc::toupper(ch.into()) as u32
    }
    #[cfg(target_vendor = "apple")]
    unsafe {
        towupper(btowc(ch.into())) as u32
    }
}

#[cfg(all(unix, not(any(target_os = "ios", target_os = "redox"))))]
pub use libc::LC_MESSAGES;

#[cfg(all(
    unix,
    not(any(target_os = "ios", target_os = "android", target_os = "redox"))
))]
pub use libc::{
    ABDAY_1, ABDAY_2, ABDAY_3, ABDAY_4, ABDAY_5, ABDAY_6, ABDAY_7, ABMON_1, ABMON_2, ABMON_3,
    ABMON_4, ABMON_5, ABMON_6, ABMON_7, ABMON_8, ABMON_9, ABMON_10, ABMON_11, ABMON_12, ALT_DIGITS,
    AM_STR, CODESET, CRNCYSTR, D_FMT, D_T_FMT, DAY_1, DAY_2, DAY_3, DAY_4, DAY_5, DAY_6, DAY_7,
    ERA, ERA_D_FMT, ERA_D_T_FMT, ERA_T_FMT, MON_1, MON_2, MON_3, MON_4, MON_5, MON_6, MON_7, MON_8,
    MON_9, MON_10, MON_11, MON_12, NOEXPR, PM_STR, RADIXCHAR, T_FMT, T_FMT_AMPM, THOUSEP, YESEXPR,
};

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
        while *cur != 0 {
            let value = *cur;
            out.push(value);
            if value == libc::c_char::MAX {
                break;
            }
            cur = cur.add(1);
        }
    }
    out
}

/// Every byte of a NUL-terminated C string, `CHAR_MAX` included.
///
/// `localeconv().grouping` spells a "repeat last group" / "stop" terminator
/// as `CHAR_MAX`. Stopping at that value drops the distinction; this reader
/// keeps it, the way a NUL-only C-string walk does.
///
/// # Safety
///
/// `ptr` must be null or point to a NUL-terminated C string that remains valid
/// for the duration of the call.
pub unsafe fn charp2bytes(ptr: *const libc::c_char) -> Vec<u8> {
    let mut out = Vec::new();
    if !ptr.is_null() {
        let mut cur = ptr;
        unsafe {
            while *cur != 0 {
                out.push(*cur as u8);
                cur = cur.add(1);
            }
        }
    }
    out
}

/// Decimal point, thousands separator and grouping of the current locale,
/// as the raw bytes `localeconv()` reports. Grouping keeps a `CHAR_MAX`
/// terminator when the C locale spells one.
pub fn localeconv_numeric() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let lc = unsafe { localeconv() };
    if lc.is_null() {
        return (b".".to_vec(), Vec::new(), Vec::new());
    }
    unsafe {
        (
            charp2bytes((*lc).decimal_point),
            charp2bytes((*lc).thousands_sep),
            charp2bytes((*lc).grouping),
        )
    }
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

#[cfg(target_os = "linux")]
pub use wide_collation::{CollationString, CollationTransformError};

#[cfg(target_os = "linux")]
mod wide_collation {
    use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf};
    use std::io;

    unsafe extern "C" {
        fn wcscoll(first: *const libc::wchar_t, second: *const libc::wchar_t) -> libc::c_int;
        fn wcsxfrm(
            destination: *mut libc::wchar_t,
            source: *const libc::wchar_t,
            capacity: usize,
        ) -> usize;
    }

    /// A NUL-terminated wide string for the locale collation APIs.
    pub struct CollationString(Vec<libc::wchar_t>);

    pub enum CollationTransformError {
        Memory,
        Os(io::Error),
    }

    impl CollationString {
        /// Preserve Python's lone surrogates and reject embedded NULs.
        pub fn new(string: &Wtf8) -> Option<Self> {
            let mut wide = Vec::with_capacity(string.len() + 1);
            for point in string.code_points() {
                let value = point.to_u32();
                if value == 0 {
                    return None;
                }
                wide.push(value as libc::wchar_t);
            }
            wide.push(0);
            Some(Self(wide))
        }

        pub fn compare(&self, other: &Self) -> libc::c_int {
            // Both buffers retain their terminator and cannot contain interior NULs.
            unsafe { wcscoll(self.0.as_ptr(), other.0.as_ptr()) }
        }

        pub fn transform(&self) -> Result<Wtf8Buf, CollationTransformError> {
            let mut dummy = [0];
            crate::os::clear_errno();
            let needed = unsafe { wcsxfrm(dummy.as_mut_ptr(), self.0.as_ptr(), 1) };
            let errno = crate::os::get_errno();
            if errno != 0 && errno != libc::ERANGE {
                return Err(CollationTransformError::Os(io::Error::from_raw_os_error(
                    errno,
                )));
            }
            let capacity = needed
                .checked_add(1)
                .ok_or(CollationTransformError::Memory)?;
            let mut wide = Vec::new();
            wide.try_reserve_exact(capacity)
                .map_err(|_| CollationTransformError::Memory)?;
            wide.resize(capacity, 0);
            crate::os::clear_errno();
            let written = unsafe { wcsxfrm(wide.as_mut_ptr(), self.0.as_ptr(), wide.len()) };
            let errno = crate::os::get_errno();
            if errno != 0 {
                return Err(CollationTransformError::Os(io::Error::from_raw_os_error(
                    errno,
                )));
            }
            // A concurrent locale change can invalidate the size query. Never
            // interpret a truncated or unterminated result as a collation key.
            if written >= wide.len() {
                return Err(CollationTransformError::Os(io::Error::from_raw_os_error(
                    libc::ERANGE,
                )));
            }
            wide.truncate(written);

            // wcsxfrm emits integer weights, not necessarily Unicode. CPython
            // 3.15 splits weights above 0x10000 into two ordered code points.
            // Keep individual surrogates rather than decoding as UTF-16.
            let mut key = Wtf8Buf::new();
            for weight in wide {
                let value = weight as u32;
                if value > 0x10000 {
                    key.push(CodePoint::from_u32((value >> 16) | 0x10000).unwrap());
                    key.push(CodePoint::from_u32(value & 0xffff).unwrap());
                } else {
                    key.push(CodePoint::from_u32(value).unwrap());
                }
            }
            Ok(key)
        }
    }
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
pub fn user_default_lcid() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetUserDefaultLCID() }
}

#[cfg(windows)]
pub const LOCALE_SISO639LANGNAME: u32 = 0x0000_0059;
#[cfg(windows)]
pub const LOCALE_SISO3166CTRYNAME: u32 = 0x0000_005A;

/// `GetLocaleInfoW` into a 16-unit buffer, the size `_getdefaultlocale` uses.
#[cfg(windows)]
pub fn locale_info(lcid: u32, lctype: u32) -> Option<String> {
    use windows_sys::Win32::Globalization::GetLocaleInfoW;

    let mut buffer = [0u16; 16];
    let len = unsafe { GetLocaleInfoW(lcid, lctype, buffer.as_mut_ptr(), buffer.len() as i32) };
    if len <= 1 {
        None
    } else {
        Some(String::from_utf16_lossy(&buffer[..len as usize - 1]))
    }
}

#[cfg(windows)]
fn wide_cstr(units: &[u16]) -> alloc::borrow::Cow<'_, [u16]> {
    if units.last() == Some(&0) {
        alloc::borrow::Cow::Borrowed(units)
    } else {
        let mut owned = units.to_vec();
        owned.push(0);
        alloc::borrow::Cow::Owned(owned)
    }
}

#[cfg(windows)]
pub fn wcscoll(s1: &[u16], s2: &[u16]) -> i32 {
    unsafe extern "C" {
        fn wcscoll(s1: *const u16, s2: *const u16) -> i32;
    }
    let s1 = wide_cstr(s1);
    let s2 = wide_cstr(s2);
    unsafe { wcscoll(s1.as_ptr(), s2.as_ptr()) }
}

#[cfg(windows)]
pub fn wcsxfrm(src: &[u16]) -> Vec<u16> {
    unsafe extern "C" {
        fn wcsxfrm(dst: *mut u16, src: *const u16, count: usize) -> usize;
    }
    let src = wide_cstr(src);
    let needed = unsafe { wcsxfrm(core::ptr::null_mut(), src.as_ptr(), 0) };
    let mut dst = vec![0u16; needed + 1];
    let written = unsafe { wcsxfrm(dst.as_mut_ptr(), src.as_ptr(), dst.len()) };
    dst.truncate(written);
    dst
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

/// Initialize the command-line interpreter's locale from its environment.
///
/// Ordinary startup changes only LC_CTYPE. PEP 538 coercion additionally
/// updates LC_CTYPE in the environment and refreshes the other categories,
/// just as CPython's _coerce_default_locale_settings does. Embedders should
/// preserve their application's locale and use a snapshot instead.
///
/// # Safety
///
/// Call only during single-threaded process startup, before other threads or
/// libraries can read the environment or use the process-global locale.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub unsafe fn initialize_ctype(ignore_environment: bool) {
    use std::io::Write;

    // An invalid environment locale is not a fatal initialization error.
    let _ = setlocale(LC_CTYPE, Some(c""));
    let control = (!ignore_environment)
        .then(|| std::env::var_os("PYTHONCOERCECLOCALE"))
        .flatten();
    let warn = control.as_deref() == Some(std::ffi::OsStr::new("warn"));
    let coerce = control.as_deref() != Some(std::ffi::OsStr::new("0"));
    let legacy = |locale: &[u8]| locale == b"C" || locale == b"POSIX";
    let Some(original) = setlocale(LC_CTYPE, None) else {
        return;
    };
    if !legacy(&original) {
        return;
    }

    let overridden = std::env::var_os("LC_ALL").is_some_and(|value| !value.is_empty());
    if coerce && !overridden {
        for candidate in [c"C.UTF-8", c"C.utf8", c"UTF-8"] {
            if setlocale(LC_CTYPE, Some(candidate)).is_none() {
                continue;
            }
            if nl_langinfo_codeset().is_none_or(|value| value.is_empty()) {
                let _ = setlocale(LC_CTYPE, Some(c""));
                continue;
            }
            let _ = setlocale(LC_ALL, Some(c""));
            // SAFETY: the caller guarantees exclusive access to environment
            // and global locale state for the entire initialization operation.
            if unsafe { libc::setenv(c"LC_CTYPE".as_ptr(), candidate.as_ptr(), 1) } != 0 {
                let _ = writeln!(
                    std::io::stderr(),
                    "Error setting LC_CTYPE, skipping C locale coercion"
                );
                break;
            }
            if warn {
                let _ = writeln!(
                    std::io::stderr(),
                    "Python detected LC_CTYPE=C: LC_CTYPE coerced to {} (set another locale or PYTHONCOERCECLOCALE=0 to disable this locale coercion behavior).",
                    candidate.to_string_lossy()
                );
            }
            let _ = setlocale(LC_ALL, Some(c""));
            return;
        }
        // setlocale's returned string contains no NUL bytes.
        if let Ok(original) = alloc::ffi::CString::new(original) {
            let _ = setlocale(LC_CTYPE, Some(&original));
        }
    }
    if warn {
        let _ = writeln!(
            std::io::stderr(),
            "Python runtime initialized with LC_CTYPE=C (a locale with default ASCII encoding), which may cause Unicode compatibility problems. Using C.UTF-8, C.utf8, or UTF-8 (if available) as alternative Unicode-compatible locales is recommended."
        );
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub use snapshot::{LocaleEncodeError, LocaleSnapshot};

// libc exposes mbstate_t on GNU Linux. Do not guess an opaque C conversion-
// state layout on platforms where the crate does not expose this type.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod snapshot {
    use core::{ffi::CStr, marker::PhantomData, ptr};
    use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf};
    use std::io;

    unsafe extern "C" {
        fn mbrtowc(
            out: *mut libc::wchar_t,
            input: *const libc::c_char,
            count: usize,
            state: *mut libc::mbstate_t,
        ) -> usize;
        fn wcrtomb(
            out: *mut libc::c_char,
            input: libc::wchar_t,
            state: *mut libc::mbstate_t,
        ) -> usize;
        fn __ctype_get_mb_cur_max() -> usize;
    }

    /// A conversion failure with enough information for UnicodeEncodeError.
    #[derive(Debug)]
    pub enum LocaleEncodeError {
        Native(io::Error),
        Character { index: usize, reason: &'static str },
    }

    impl From<io::Error> for LocaleEncodeError {
        fn from(error: io::Error) -> Self {
            Self::Native(error)
        }
    }

    /// An owned copy of the startup thread's locale for filesystem bootstrap.
    /// Later process-global setlocale calls cannot change this conversion state.
    pub struct LocaleSnapshot {
        locale: libc::locale_t,
        encoding: String,
    }

    // The owned locale is never modified after duplocale. uselocale installs
    // it only in the calling thread; every conversion has its own mbstate_t.
    // A borrowed guard keeps the locale alive until that thread restores its
    // previous locale, so sharing an immutable snapshot is safe.
    unsafe impl Send for LocaleSnapshot {}
    unsafe impl Sync for LocaleSnapshot {}

    impl Drop for LocaleSnapshot {
        fn drop(&mut self) {
            // SAFETY: capture owns a non-null duplicate, never LC_GLOBAL_LOCALE.
            unsafe { libc::freelocale(self.locale) };
        }
    }

    struct ThreadLocale<'a> {
        previous: libc::locale_t,
        _snapshot: &'a LocaleSnapshot,
        // The guard must be restored on the same thread that installed it.
        _not_send: PhantomData<*mut ()>,
    }

    impl Drop for ThreadLocale<'_> {
        fn drop(&mut self) {
            // SAFETY: previous is the valid locale returned by uselocale, and
            // the non-Send guard cannot migrate to a different thread.
            unsafe { libc::uselocale(self.previous) };
        }
    }

    impl LocaleSnapshot {
        pub fn capture() -> io::Result<Self> {
            // A null argument queries the caller's locale without changing it.
            let current = unsafe { libc::uselocale(ptr::null_mut()) };
            if current.is_null() {
                return Err(io::Error::last_os_error());
            }
            // duplocale also accepts the LC_GLOBAL_LOCALE sentinel.
            let locale = unsafe { libc::duplocale(current) };
            if locale.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut snapshot = Self {
                locale,
                encoding: String::new(),
            };
            let codeset = unsafe { libc::nl_langinfo_l(libc::CODESET, locale) };
            if codeset.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing locale encoding",
                ));
            }
            let encoding = unsafe { CStr::from_ptr(codeset) }
                .to_str()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if encoding.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "empty locale encoding",
                ));
            }
            encoding.clone_into(&mut snapshot.encoding);
            Ok(snapshot)
        }

        pub fn encoding(&self) -> &str {
            &self.encoding
        }

        fn enter(&self) -> io::Result<ThreadLocale<'_>> {
            let previous = unsafe { libc::uselocale(self.locale) };
            if previous.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(ThreadLocale {
                previous,
                _snapshot: self,
                _not_send: PhantomData,
            })
        }

        /// Decode with CPython's locale bootstrap surrogateescape behavior.
        pub fn decode(&self, input: &[u8]) -> io::Result<Wtf8Buf> {
            let _locale = self.enter()?;
            let mut output = Wtf8Buf::with_capacity(input.len());
            let mut input = input;
            // C specifies an all-zero mbstate_t as the initial conversion state.
            let mut state: libc::mbstate_t = unsafe { core::mem::zeroed() };
            while !input.is_empty() {
                let mut value: libc::wchar_t = 0;
                let count =
                    unsafe { mbrtowc(&mut value, input.as_ptr().cast(), input.len(), &mut state) };
                let point = u32::try_from(value).ok().and_then(char::from_u32);
                if count >= usize::MAX - 1 || point.is_none() {
                    // Invalid, incomplete, surrogate, and out-of-Unicode-range
                    // results all escape one byte and restart the shift state.
                    output.push(CodePoint::from(0xdc00u16 + u16::from(input[0])));
                    input = &input[1..];
                    state = unsafe { core::mem::zeroed() };
                } else {
                    if let Some(point) = point {
                        output.push_char(point);
                    }
                    // mbrtowc returns zero for an embedded NUL, consuming one
                    // byte. Our slice API retains NULs for later path validation.
                    input = &input[count.max(1)..];
                }
            }
            Ok(output)
        }

        /// Encode with the locale captured at startup, preserving escaped bytes.
        pub fn encode(&self, input: &Wtf8) -> Result<Vec<u8>, LocaleEncodeError> {
            let _locale = self.enter()?;
            let max_bytes = unsafe { __ctype_get_mb_cur_max() };
            if max_bytes == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid locale character size",
                )
                .into());
            }
            let mut buffer = vec![0u8; max_bytes];
            let mut output = Vec::with_capacity(input.len());
            for (index, point) in input.code_points().enumerate() {
                let value = point.to_u32();
                if (0xdc80..=0xdcff).contains(&value) {
                    output.push((value - 0xdc00) as u8);
                    continue;
                }
                if char::from_u32(value).is_none() {
                    return Err(LocaleEncodeError::Character {
                        index,
                        reason: "surrogates not allowed",
                    });
                }
                // CPython's bootstrap encoder converts each character from the
                // initial shift state (wcstombs on a one-character string).
                let mut state: libc::mbstate_t = unsafe { core::mem::zeroed() };
                let count = unsafe {
                    wcrtomb(
                        buffer.as_mut_ptr().cast(),
                        value as libc::wchar_t,
                        &mut state,
                    )
                };
                if count == usize::MAX {
                    return Err(LocaleEncodeError::Character {
                        index,
                        reason: "encoding error",
                    });
                }
                output.extend_from_slice(&buffer[..count]);
                // Preserve a stateful codec's shift-back bytes, excluding the
                // terminator that wcstombs would not count in its result.
                let count = unsafe { wcrtomb(buffer.as_mut_ptr().cast(), 0, &mut state) };
                if count == usize::MAX {
                    return Err(io::Error::last_os_error().into());
                }
                output.extend_from_slice(&buffer[..count.saturating_sub(1)]);
            }
            Ok(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::copy_grouping;

    #[test]
    fn grouping_keeps_char_max_terminator() {
        let grouping = [3, libc::c_char::MAX, 2, 0];
        assert_eq!(copy_grouping(grouping.as_ptr()), [3, libc::c_char::MAX]);
    }
}

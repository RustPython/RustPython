use alloc::borrow::Cow;
use core::ffi::{
    CStr, c_char, c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint,
    c_ulong, c_ulonglong, c_ushort, c_void,
};
#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
use libffi::middle::Type;
#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
use libffi::{
    low,
    middle::{Arg, Cif, Closure, CodePtr},
};
#[cfg(any(unix, windows))]
use libloading::Library;
#[cfg(unix)]
use libloading::os::unix::Library as UnixLibrary;
#[cfg(any(unix, windows))]
use parking_lot::{Mutex, RwLock};
use rustpython_wtf8::Wtf8;
use rustpython_wtf8::Wtf8Buf;
#[cfg(any(unix, windows))]
use std::{collections::HashMap, ffi::OsStr, sync::OnceLock};

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub type FfiType = Type;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub type FfiArg<'a> = Arg<'a>;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub type FfiCodePtr = CodePtr;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub type FfiCif = low::ffi_cif;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
type CallbackIntResult = low::ffi_arg;

#[cfg(not(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
)))]
type CallbackIntResult = c_int;

#[cfg(any(unix, windows, target_os = "wasi"))]
pub type WChar = libc::wchar_t;
#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub type WChar = u32;

#[cfg(any(unix, windows, target_os = "wasi"))]
type TimeT = libc::time_t;
#[cfg(not(any(unix, windows, target_os = "wasi")))]
type TimeT = i64;

std::thread_local! {
    /// Thread-local ctypes errno, separate from the platform errno.
    #[allow(clippy::missing_const_for_thread_local)]
    static CTYPES_LOCAL_ERRNO: core::cell::Cell<i32> = const { core::cell::Cell::new(0) };
}

pub fn get_errno() -> i32 {
    CTYPES_LOCAL_ERRNO.with(|e| e.get())
}

pub fn set_errno(value: i32) -> i32 {
    CTYPES_LOCAL_ERRNO.with(|e| {
        let old = e.get();
        e.set(value);
        old
    })
}

#[cfg(not(windows))]
pub fn with_swapped_errno<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let saved_errno = crate::os::get_errno();
    let saved_ctypes_errno = CTYPES_LOCAL_ERRNO.with(|e| e.get());
    crate::os::set_errno(saved_ctypes_errno);

    let result = f();

    let new_error = crate::os::get_errno();
    CTYPES_LOCAL_ERRNO.with(|e| e.set(new_error));
    crate::os::set_errno(saved_errno);

    result
}

pub fn with_callback_errno_preserved<F, R>(use_errno: bool, f: F) -> R
where
    F: FnOnce() -> R,
{
    if !use_errno {
        return f();
    }

    let saved = crate::os::get_errno();
    let result = f();
    let _current = crate::os::get_errno();
    crate::os::set_errno(saved);
    result
}

#[cfg(windows)]
std::thread_local! {
    /// Thread-local ctypes last_error, separate from the Windows last error.
    static CTYPES_LOCAL_LAST_ERROR: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

#[cfg(windows)]
pub fn get_last_error() -> u32 {
    CTYPES_LOCAL_LAST_ERROR.with(|e| e.get())
}

#[cfg(windows)]
pub fn set_last_error(value: u32) -> u32 {
    CTYPES_LOCAL_LAST_ERROR.with(|e| {
        let old = e.get();
        e.set(value);
        old
    })
}

#[cfg(windows)]
pub fn with_swapped_last_error<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let saved_last_error = crate::windows::get_last_error();
    let saved_ctypes_last_error = CTYPES_LOCAL_LAST_ERROR.with(|e| e.get());
    crate::windows::set_last_error(saved_ctypes_last_error);

    let result = f();

    let new_error = crate::windows::get_last_error();
    CTYPES_LOCAL_LAST_ERROR.with(|e| e.set(new_error));
    crate::windows::set_last_error(saved_last_error);

    result
}

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    not(target_os = "windows")
))]
const LONG_DOUBLE_SIZE: usize = core::mem::size_of::<u128>();

#[cfg(target_os = "windows")]
const LONG_DOUBLE_SIZE: usize = core::mem::size_of::<c_double>();

#[cfg(not(any(
    all(
        any(target_arch = "x86_64", target_arch = "aarch64"),
        not(target_os = "windows")
    ),
    target_os = "windows"
)))]
const LONG_DOUBLE_SIZE: usize = core::mem::size_of::<c_double>();

pub fn simple_type_size(ty: &str) -> Option<usize> {
    match ty {
        "c" | "b" => Some(core::mem::size_of::<c_schar>()),
        "u" => Some(core::mem::size_of::<WChar>()),
        "B" | "?" => Some(core::mem::size_of::<c_uchar>()),
        "h" | "v" => Some(core::mem::size_of::<c_short>()),
        "H" => Some(core::mem::size_of::<c_ushort>()),
        "i" => Some(core::mem::size_of::<c_int>()),
        "I" => Some(core::mem::size_of::<c_uint>()),
        "l" => Some(core::mem::size_of::<c_long>()),
        "L" => Some(core::mem::size_of::<c_ulong>()),
        "q" => Some(core::mem::size_of::<c_longlong>()),
        "Q" => Some(core::mem::size_of::<c_ulonglong>()),
        "f" => Some(core::mem::size_of::<c_float>()),
        "d" => Some(core::mem::size_of::<c_double>()),
        "g" => Some(LONG_DOUBLE_SIZE),
        "z" | "Z" | "P" | "X" | "O" => Some(core::mem::size_of::<usize>()),
        "void" => Some(0),
        _ => None,
    }
}

pub fn simple_type_align(ty: &str) -> Option<usize> {
    match ty {
        "c" | "b" => Some(core::mem::align_of::<c_schar>()),
        "u" => Some(core::mem::align_of::<WChar>()),
        "B" | "?" => Some(core::mem::align_of::<c_uchar>()),
        "h" | "v" => Some(core::mem::align_of::<c_short>()),
        "H" => Some(core::mem::align_of::<c_ushort>()),
        "i" => Some(core::mem::align_of::<c_int>()),
        "I" => Some(core::mem::align_of::<c_uint>()),
        "l" => Some(core::mem::align_of::<c_long>()),
        "L" => Some(core::mem::align_of::<c_ulong>()),
        "q" => Some(core::mem::align_of::<c_longlong>()),
        "Q" => Some(core::mem::align_of::<c_ulonglong>()),
        "f" => Some(core::mem::align_of::<c_float>()),
        "d" => Some(core::mem::align_of::<c_double>()),
        "g" => {
            #[cfg(all(
                any(target_arch = "x86_64", target_arch = "aarch64"),
                not(target_os = "windows")
            ))]
            {
                Some(core::mem::align_of::<u128>())
            }
            #[cfg(not(all(
                any(target_arch = "x86_64", target_arch = "aarch64"),
                not(target_os = "windows")
            )))]
            {
                Some(core::mem::align_of::<c_double>())
            }
        }
        "z" | "Z" | "P" | "X" | "O" => Some(core::mem::align_of::<usize>()),
        "void" => Some(0),
        _ => None,
    }
}

pub fn c_long_bytes_endian(value: i128, swapped: bool) -> Vec<u8> {
    let value = value as c_long;
    int_to_sized_bytes_endian(value as i64, core::mem::size_of::<c_long>(), swapped)
}

pub fn c_ulong_bytes_endian(value: i128, swapped: bool) -> Vec<u8> {
    let value = value as c_ulong;
    uint_to_sized_bytes_endian(value as u64, core::mem::size_of::<c_ulong>(), swapped)
}

pub fn simple_type_pep3118_code(code: char) -> char {
    match code {
        'i' if core::mem::size_of::<c_int>() == 2 => 'h',
        'i' if core::mem::size_of::<c_int>() == 4 => 'i',
        'i' if core::mem::size_of::<c_int>() == 8 => 'q',
        'I' if core::mem::size_of::<c_int>() == 2 => 'H',
        'I' if core::mem::size_of::<c_int>() == 4 => 'I',
        'I' if core::mem::size_of::<c_int>() == 8 => 'Q',
        'l' if core::mem::size_of::<c_long>() == 4 => 'l',
        'l' if core::mem::size_of::<c_long>() == 8 => 'q',
        'L' if core::mem::size_of::<c_long>() == 4 => 'L',
        'L' if core::mem::size_of::<c_long>() == 8 => 'Q',
        '?' if core::mem::size_of::<bool>() == 1 => '?',
        '?' if core::mem::size_of::<bool>() == 2 => 'H',
        '?' if core::mem::size_of::<bool>() == 4 => 'L',
        '?' if core::mem::size_of::<bool>() == 8 => 'Q',
        _ => code,
    }
}

pub enum StringAtError {
    NullPointer,
    TooLong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawMemoryViewError {
    NullPointer,
    NegativeSize,
}

#[derive(Debug, Clone, Copy)]
pub struct RawMemoryView {
    ptr: usize,
    size: usize,
    readonly: bool,
}

impl RawMemoryView {
    pub fn new(ptr: usize, size: isize, readonly: bool) -> Result<Self, RawMemoryViewError> {
        if ptr == 0 {
            return Err(RawMemoryViewError::NullPointer);
        }
        if size < 0 {
            return Err(RawMemoryViewError::NegativeSize);
        }
        Ok(Self {
            ptr,
            size: size as usize,
            readonly,
        })
    }

    pub fn size(self) -> usize {
        self.size
    }

    pub fn readonly(self) -> bool {
        self.readonly
    }

    /// # Safety
    ///
    /// The stored pointer must remain valid for `self.size` bytes.
    pub unsafe fn bytes(self) -> &'static [u8] {
        unsafe { borrow_memory(self.ptr as *const u8, self.size) }
    }

    /// # Safety
    ///
    /// The stored pointer must remain valid and uniquely writable for
    /// `self.size` bytes.
    pub unsafe fn bytes_mut(self) -> &'static mut [u8] {
        unsafe { borrow_memory_mut(self.ptr as *mut u8, self.size) }
    }
}

// These match the current RustPython _ctypes surface exactly.
pub const RTLD_LOCAL: i32 = 0;
pub const RTLD_GLOBAL: i32 = 0;
pub const SIZEOF_TIME_T: usize = core::mem::size_of::<TimeT>();

#[cfg(all(unix, not(target_os = "wasi")))]
pub fn dlopen_mode(load_flags: Option<i32>) -> i32 {
    load_flags.unwrap_or(libc::RTLD_NOW | libc::RTLD_LOCAL) | libc::RTLD_NOW
}

#[cfg(not(all(unix, not(target_os = "wasi"))))]
pub fn dlopen_mode(load_flags: Option<i32>) -> i32 {
    load_flags.unwrap_or(0)
}

#[cfg(target_os = "macos")]
pub fn dyld_shared_cache_contains_path(path: &str) -> Result<bool, alloc::ffi::NulError> {
    let c_path = alloc::ffi::CString::new(path)?;

    unsafe extern "C" {
        fn _dyld_shared_cache_contains_path(path: *const c_char) -> bool;
    }

    Ok(unsafe { _dyld_shared_cache_contains_path(c_path.as_ptr()) })
}

/// # Safety
///
/// `ptr` must be valid to read until the first NUL byte.
pub unsafe fn strlen(ptr: *const c_char) -> usize {
    #[cfg(any(unix, windows, target_os = "wasi"))]
    {
        unsafe { libc::strlen(ptr) }
    }
    #[cfg(not(any(unix, windows, target_os = "wasi")))]
    {
        let mut len = 0;
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        len
    }
}

/// # Safety
///
/// `ptr` must be valid to read until the first NUL wide character.
pub unsafe fn wcslen(ptr: *const WChar) -> usize {
    let mut len = 0;
    while unsafe { *ptr.add(len) } != 0 as WChar {
        len += 1;
    }
    len
}

/// # Safety
///
/// `ptr` must be a valid NUL-terminated C string.
pub unsafe fn read_c_string_bytes(ptr: *const c_char) -> Vec<u8> {
    unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec()
}

#[inline]
pub fn read_pointer_from_buffer(buffer: &[u8]) -> usize {
    const PTR_SIZE: usize = core::mem::size_of::<usize>();
    buffer
        .first_chunk::<PTR_SIZE>()
        .copied()
        .map_or(0, usize::from_ne_bytes)
}

pub const WCHAR_SIZE: usize = core::mem::size_of::<WChar>();

#[inline]
pub fn wchar_from_bytes(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < WCHAR_SIZE {
        return None;
    }
    Some(if WCHAR_SIZE == 2 {
        u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
    } else {
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

#[inline]
pub fn wchar_to_bytes(ch: u32, buffer: &mut [u8]) {
    if WCHAR_SIZE == 2 {
        if buffer.len() >= 2 {
            buffer[..2].copy_from_slice(&(ch as u16).to_ne_bytes());
        }
    } else if buffer.len() >= 4 {
        buffer[..4].copy_from_slice(&ch.to_ne_bytes());
    }
}

pub fn wstring_from_bytes(buffer: &[u8]) -> String {
    let mut chars = Vec::new();
    for chunk in buffer.chunks(WCHAR_SIZE) {
        if chunk.len() < WCHAR_SIZE {
            break;
        }
        let Some(code) = wchar_from_bytes(chunk) else {
            break;
        };
        if code == 0 {
            break;
        }
        if let Some(ch) = char::from_u32(code) {
            chars.push(ch);
        }
    }
    chars.into_iter().collect()
}

pub fn wchar_array_field_value(buffer: &[u8]) -> String {
    let wchars: Vec<WChar> = buffer
        .chunks(WCHAR_SIZE)
        .filter_map(|chunk| wchar_from_bytes(chunk).filter(|&wchar| wchar != 0))
        .map(|wchar| wchar as WChar)
        .collect();
    wide_chars_to_wtf8(&wchars).to_string()
}

pub fn write_wchar_array_value(buffer: &mut [u8], s: &Wtf8) -> Result<(), WCharArrayWriteError> {
    let wchar_count = buffer.len() / WCHAR_SIZE;
    let char_count = s.code_points().count();

    if char_count > wchar_count {
        return Err(WCharArrayWriteError::TooLong);
    }

    for (i, ch) in s.code_points().enumerate() {
        let offset = i * WCHAR_SIZE;
        wchar_to_bytes(ch.to_u32(), &mut buffer[offset..]);
    }

    let terminator_offset = char_count * WCHAR_SIZE;
    if terminator_offset + WCHAR_SIZE <= buffer.len() {
        wchar_to_bytes(0, &mut buffer[terminator_offset..]);
    }
    Ok(())
}

pub fn encode_wtf8_to_wchar_padded(s: &Wtf8, size: usize) -> Vec<u8> {
    let mut wchar_bytes = Vec::with_capacity(size);
    for cp in s.code_points().take(size / WCHAR_SIZE) {
        let mut bytes = [0u8; 4];
        wchar_to_bytes(cp.to_u32(), &mut bytes);
        wchar_bytes.extend_from_slice(&bytes[..WCHAR_SIZE]);
    }
    while wchar_bytes.len() < size {
        wchar_bytes.push(0);
    }
    wchar_bytes
}

pub fn wchar_null_terminated_bytes(s: &Wtf8) -> Vec<u8> {
    let wchars: Vec<WChar> = s
        .code_points()
        .map(|cp| cp.to_u32() as WChar)
        .chain(core::iter::once(0))
        .collect();
    vec_into_bytes(wchars)
}

pub fn vec_into_bytes<T>(vec: Vec<T>) -> Vec<u8> {
    let len = vec.len() * core::mem::size_of::<T>();
    let cap = vec.capacity() * core::mem::size_of::<T>();
    let ptr = vec.as_ptr() as *mut u8;
    core::mem::forget(vec);
    unsafe { Vec::from_raw_parts(ptr, len, cap) }
}

pub enum IntegerValue {
    Signed(i64),
    Unsigned(u64),
}

pub enum AddressValue {
    ByteString(u8),
    Integer(IntegerValue),
    Float(f64),
    Pointer(usize),
    Bytes(Vec<u8>),
}

pub enum AddressWriteValue<'a> {
    Pointer(usize),
    U8(u8),
    I16(i16),
    I32(i32),
    I64(i64),
    Float(f64),
    Bytes(&'a [u8]),
}

pub enum ArrayElementWriteValue<'a> {
    Byte(u8),
    Wchar(u32),
    Pointer { value: usize, size: usize },
    Float { value: f64, size: usize },
    Bytes { bytes: &'a [u8], size: usize },
}

pub enum WCharArrayWriteError {
    TooLong,
}

pub enum SimpleStorageValue {
    Byte(u8),
    Wchar(u32),
    Signed(i128),
    Float(f64),
    Bool(bool),
    Pointer(usize),
    ObjectId(usize),
    Zero,
}

pub enum DecodedValue {
    Bytes(Vec<u8>),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Bool(bool),
    Pointer(usize),
    String(String),
    None,
}

pub enum CallbackResultValue {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Pointer(usize),
    Bool(bool),
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub enum FfiArgRef<'a> {
    U8(&'a u8),
    I8(&'a i8),
    U16(&'a u16),
    I16(&'a i16),
    U32(&'a u32),
    I32(&'a i32),
    U64(&'a u64),
    I64(&'a i64),
    F32(&'a f32),
    F64(&'a f64),
    Pointer(&'a usize),
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
#[derive(Debug, Clone, Copy)]
pub enum FfiValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Pointer(usize),
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub enum CallResult {
    Void,
    Pointer(usize),
    Value(low::ffi_arg),
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub enum CdeclArgValue {
    Pointer(isize),
    Int(isize),
}

pub const POINTER_SIZE: usize = core::mem::size_of::<usize>();
pub const POINTER_FORMAT: &str = "X{}";

pub fn pointer_size() -> usize {
    POINTER_SIZE
}

pub fn pointer_format() -> &'static str {
    POINTER_FORMAT
}

pub fn has_pointer_width(buffer: &[u8]) -> bool {
    buffer.len() >= POINTER_SIZE
}

pub fn pointer_bytes(value: usize) -> Vec<u8> {
    pointer_to_sized_bytes(value, POINTER_SIZE)
}

pub fn null_pointer_bytes() -> Vec<u8> {
    vec![0; POINTER_SIZE]
}

pub fn zeroed_bytes(size: usize) -> Vec<u8> {
    vec![0; size]
}

pub fn copy_to_sized_bytes(src: &[u8], size: usize) -> Vec<u8> {
    let mut result = zeroed_bytes(size);
    let len = src.len().min(size);
    result[..len].copy_from_slice(&src[..len]);
    result
}

pub fn char_array_assignment_bytes(src: &[u8]) -> &[u8] {
    if let Some(null_pos) = src.iter().position(|&b| b == 0) {
        &src[..=null_pos]
    } else {
        src
    }
}

pub fn char_array_field_value(buffer: &[u8]) -> &[u8] {
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    &buffer[..end]
}

pub fn write_char_array_value(buffer: &mut [u8], src: &[u8]) {
    buffer[..src.len()].copy_from_slice(src);
    if src.len() < buffer.len() {
        buffer[src.len()] = 0;
    }
}

pub fn write_char_array_raw(buffer: &mut [u8], src: &[u8]) {
    buffer[..src.len()].copy_from_slice(src);
}

pub fn write_prefix_limited(buffer: &mut [u8], src: &[u8], size: usize) {
    let copy_size = size.min(buffer.len()).min(src.len());
    if copy_size > 0 {
        buffer[..copy_size].copy_from_slice(&src[..copy_size]);
    }
}

pub fn pointer_to_sized_bytes_endian(value: usize, size: usize, swapped: bool) -> Vec<u8> {
    let mut bytes = pointer_to_sized_bytes(value, size);
    if swapped {
        bytes.reverse();
    }
    bytes
}

pub fn write_pointer_to_buffer_at(buffer: &mut [u8], offset: usize, size: usize, value: usize) {
    if offset + size <= buffer.len() {
        let ptr_bytes = pointer_to_sized_bytes(value, size);
        buffer[offset..offset + size].copy_from_slice(&ptr_bytes);
    }
}

pub fn write_array_element(buffer: &mut [u8], offset: usize, value: ArrayElementWriteValue<'_>) {
    match value {
        ArrayElementWriteValue::Byte(value) => {
            if offset < buffer.len() {
                buffer[offset] = value;
            }
        }
        ArrayElementWriteValue::Wchar(value) => {
            if offset + WCHAR_SIZE <= buffer.len() {
                wchar_to_bytes(value, &mut buffer[offset..]);
            }
        }
        ArrayElementWriteValue::Pointer { value, size } => {
            write_pointer_to_buffer_at(buffer, offset, size, value);
        }
        ArrayElementWriteValue::Float { value, size } => {
            if offset + size <= buffer.len()
                && let Some(float_bytes) = float_to_sized_bytes(value, size)
            {
                buffer[offset..offset + size].copy_from_slice(&float_bytes);
            }
        }
        ArrayElementWriteValue::Bytes { bytes, size } => {
            let copy_len = bytes.len().min(size);
            if offset + copy_len <= buffer.len() {
                buffer[offset..offset + copy_len].copy_from_slice(&bytes[..copy_len]);
            }
        }
    }
}

pub fn read_array_element(
    buffer: &[u8],
    offset: usize,
    element_size: usize,
    type_code: Option<&str>,
) -> DecodedValue {
    let Some(rest) = buffer.get(offset..) else {
        return DecodedValue::Signed(0);
    };
    match type_code {
        Some("c") => DecodedValue::Bytes(vec![buffer.get(offset).copied().unwrap_or(0)]),
        Some("u") => {
            let value = wchar_from_bytes(rest)
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_default();
            DecodedValue::String(value)
        }
        Some("z") => {
            if offset + element_size > buffer.len() {
                return DecodedValue::None;
            }
            let ptr_bytes = &buffer[offset..offset + element_size];
            let ptr_val = read_pointer_from_buffer(ptr_bytes);
            unsafe {
                match read_c_string_from_address(ptr_val) {
                    Some(bytes) => DecodedValue::Bytes(bytes),
                    None => DecodedValue::None,
                }
            }
        }
        Some("Z") => {
            if offset + element_size > buffer.len() {
                return DecodedValue::None;
            }
            let ptr_bytes = &buffer[offset..offset + element_size];
            let ptr_val = read_pointer_from_buffer(ptr_bytes);
            unsafe {
                match read_wide_string_from_address(ptr_val) {
                    Some(s) => DecodedValue::String(s.to_string()),
                    None => DecodedValue::None,
                }
            }
        }
        Some("f") => DecodedValue::Float(
            rest.first_chunk::<4>()
                .copied()
                .map_or(0.0, f32::from_ne_bytes) as f64,
        ),
        Some("d" | "g") => DecodedValue::Float(
            rest.first_chunk::<8>()
                .copied()
                .map_or(0.0, f64::from_ne_bytes),
        ),
        _ => {
            if let Some(bytes) = rest.get(..element_size) {
                let is_unsigned = matches!(type_code, Some("B" | "H" | "I" | "L" | "Q"));
                match int_from_bytes(bytes, element_size, is_unsigned) {
                    IntegerValue::Signed(value) => DecodedValue::Signed(value),
                    IntegerValue::Unsigned(value) => DecodedValue::Unsigned(value),
                }
            } else {
                DecodedValue::Signed(0)
            }
        }
    }
}

pub fn int_from_bytes(bytes: &[u8], size: usize, unsigned: bool) -> IntegerValue {
    match (size, unsigned) {
        (1, false) => IntegerValue::Signed(bytes[0] as i8 as i64),
        (1, true) => IntegerValue::Unsigned(bytes[0].into()),
        (2, false) => IntegerValue::Signed(i16::from_ne_bytes([bytes[0], bytes[1]]).into()),
        (2, true) => IntegerValue::Unsigned(u16::from_ne_bytes([bytes[0], bytes[1]]).into()),
        (4, false) => IntegerValue::Signed(
            i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into(),
        ),
        (4, true) => IntegerValue::Unsigned(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into(),
        ),
        (8, false) => IntegerValue::Signed(i64::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        (8, true) => IntegerValue::Unsigned(u64::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
        _ => IntegerValue::Signed(0),
    }
}

pub fn int_to_sized_bytes(value: i64, size: usize) -> Vec<u8> {
    match size {
        1 => (value as i8).to_ne_bytes().to_vec(),
        2 => (value as i16).to_ne_bytes().to_vec(),
        4 => (value as i32).to_ne_bytes().to_vec(),
        8 => value.to_ne_bytes().to_vec(),
        _ => vec![0u8; size],
    }
}

pub fn uint_to_sized_bytes(value: u64, size: usize) -> Vec<u8> {
    match size {
        1 => (value as u8).to_ne_bytes().to_vec(),
        2 => (value as u16).to_ne_bytes().to_vec(),
        4 => (value as u32).to_ne_bytes().to_vec(),
        8 => value.to_ne_bytes().to_vec(),
        _ => vec![0u8; size],
    }
}

pub fn int_to_sized_bytes_endian(value: i64, size: usize, swapped: bool) -> Vec<u8> {
    if swapped {
        #[cfg(target_endian = "little")]
        {
            match size {
                1 => (value as i8).to_ne_bytes().to_vec(),
                2 => (value as i16).to_be_bytes().to_vec(),
                4 => (value as i32).to_be_bytes().to_vec(),
                8 => value.to_be_bytes().to_vec(),
                _ => vec![0u8; size],
            }
        }
        #[cfg(target_endian = "big")]
        {
            match size {
                1 => (value as i8).to_ne_bytes().to_vec(),
                2 => (value as i16).to_le_bytes().to_vec(),
                4 => (value as i32).to_le_bytes().to_vec(),
                8 => value.to_le_bytes().to_vec(),
                _ => vec![0u8; size],
            }
        }
    } else {
        int_to_sized_bytes(value, size)
    }
}

pub fn uint_to_sized_bytes_endian(value: u64, size: usize, swapped: bool) -> Vec<u8> {
    if swapped {
        #[cfg(target_endian = "little")]
        {
            match size {
                1 => (value as u8).to_ne_bytes().to_vec(),
                2 => (value as u16).to_be_bytes().to_vec(),
                4 => (value as u32).to_be_bytes().to_vec(),
                8 => value.to_be_bytes().to_vec(),
                _ => vec![0u8; size],
            }
        }
        #[cfg(target_endian = "big")]
        {
            match size {
                1 => (value as u8).to_ne_bytes().to_vec(),
                2 => (value as u16).to_le_bytes().to_vec(),
                4 => (value as u32).to_le_bytes().to_vec(),
                8 => value.to_le_bytes().to_vec(),
                _ => vec![0u8; size],
            }
        }
    } else {
        uint_to_sized_bytes(value, size)
    }
}

pub fn float_to_sized_bytes(value: f64, size: usize) -> Option<Vec<u8>> {
    match size {
        4 => Some((value as f32).to_ne_bytes().to_vec()),
        8 => Some(value.to_ne_bytes().to_vec()),
        _ => None,
    }
}

pub fn float_to_sized_bytes_endian(value: f64, size: usize, swapped: bool) -> Option<Vec<u8>> {
    if swapped {
        #[cfg(target_endian = "little")]
        {
            match size {
                4 => Some((value as f32).to_be_bytes().to_vec()),
                8 => Some(value.to_be_bytes().to_vec()),
                _ => None,
            }
        }
        #[cfg(target_endian = "big")]
        {
            match size {
                4 => Some((value as f32).to_le_bytes().to_vec()),
                8 => Some(value.to_le_bytes().to_vec()),
                _ => None,
            }
        }
    } else {
        float_to_sized_bytes(value, size)
    }
}

pub fn pointer_to_sized_bytes(value: usize, size: usize) -> Vec<u8> {
    let mut result = vec![0u8; size];
    let bytes = value.to_ne_bytes();
    let len = core::cmp::min(bytes.len(), size);
    result[..len].copy_from_slice(&bytes[..len]);
    result
}

pub fn wchar_code_to_bytes_endian(ch: u32, swapped: bool) -> Vec<u8> {
    let mut buffer = vec![0u8; WCHAR_SIZE];
    wchar_to_bytes(ch, &mut buffer);
    if swapped {
        buffer.reverse();
    }
    buffer
}

pub fn simple_storage_value_to_bytes_endian(
    type_code: &str,
    value: SimpleStorageValue,
    swapped: bool,
) -> Vec<u8> {
    match type_code {
        "c" => match value {
            SimpleStorageValue::Byte(value) => vec![value],
            _ => vec![0],
        },
        "u" => match value {
            SimpleStorageValue::Wchar(value) => wchar_code_to_bytes_endian(value, swapped),
            _ => vec![0; WCHAR_SIZE],
        },
        "b" => match value {
            SimpleStorageValue::Signed(value) => vec![(value as i8) as u8],
            _ => vec![0],
        },
        "B" => match value {
            SimpleStorageValue::Signed(value) => vec![value as u8],
            _ => vec![0],
        },
        "h" => match value {
            SimpleStorageValue::Signed(value) => {
                int_to_sized_bytes_endian((value as i16).into(), 2, swapped)
            }
            _ => vec![0; 2],
        },
        "H" => match value {
            SimpleStorageValue::Signed(value) => {
                uint_to_sized_bytes_endian((value as u16).into(), 2, swapped)
            }
            _ => vec![0; 2],
        },
        "i" => match value {
            SimpleStorageValue::Signed(value) => {
                int_to_sized_bytes_endian((value as i32).into(), 4, swapped)
            }
            _ => vec![0; 4],
        },
        "I" => match value {
            SimpleStorageValue::Signed(value) => {
                uint_to_sized_bytes_endian((value as u32).into(), 4, swapped)
            }
            _ => vec![0; 4],
        },
        "l" => match value {
            SimpleStorageValue::Signed(value) => c_long_bytes_endian(value, swapped),
            _ => vec![0; simple_type_size("l").expect("invalid ctypes simple type")],
        },
        "L" => match value {
            SimpleStorageValue::Signed(value) => c_ulong_bytes_endian(value, swapped),
            _ => vec![0; simple_type_size("L").expect("invalid ctypes simple type")],
        },
        "q" => match value {
            SimpleStorageValue::Signed(value) => {
                int_to_sized_bytes_endian(value as i64, 8, swapped)
            }
            _ => vec![0; 8],
        },
        "Q" => match value {
            SimpleStorageValue::Signed(value) => {
                uint_to_sized_bytes_endian(value as u64, 8, swapped)
            }
            _ => vec![0; 8],
        },
        "f" => match value {
            SimpleStorageValue::Float(value) => {
                float_to_sized_bytes_endian(value, 4, swapped).expect("f32 size is fixed")
            }
            _ => vec![0; 4],
        },
        "d" => match value {
            SimpleStorageValue::Float(value) => {
                float_to_sized_bytes_endian(value, 8, swapped).expect("f64 size is fixed")
            }
            _ => vec![0; 8],
        },
        "g" => {
            let value = match value {
                SimpleStorageValue::Float(value) => value,
                _ => 0.0,
            };
            let mut result =
                float_to_sized_bytes_endian(value, 8, swapped).expect("f64 size is fixed");
            result.resize(
                simple_type_size("g").expect("invalid ctypes simple type"),
                0,
            );
            result
        }
        "?" => match value {
            SimpleStorageValue::Bool(value) => vec![if value { 1 } else { 0 }],
            _ => vec![0],
        },
        "v" => match value {
            SimpleStorageValue::Bool(value) => {
                let value: i16 = if value { -1 } else { 0 };
                int_to_sized_bytes_endian(value.into(), 2, swapped)
            }
            _ => vec![0; 2],
        },
        "P" | "z" | "Z" => match value {
            SimpleStorageValue::Pointer(value) => {
                uint_to_sized_bytes_endian(value as u64, pointer_size(), swapped)
            }
            _ => null_pointer_bytes(),
        },
        "O" => match value {
            SimpleStorageValue::ObjectId(value) => {
                uint_to_sized_bytes_endian(value as u64, pointer_size(), swapped)
            }
            _ => null_pointer_bytes(),
        },
        _ => vec![0],
    }
}

pub fn utf16z_bytes(s: &Wtf8) -> Vec<u8> {
    vec_into_bytes::<u16>(s.encode_wide().chain(core::iter::once(0)).collect())
}

pub fn null_terminated_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut buffer = bytes.to_vec();
    buffer.push(0);
    buffer
}

pub fn decode_type_code(type_code: &str, bytes: &[u8]) -> DecodedValue {
    match type_code {
        "c" => DecodedValue::Bytes(bytes.to_vec()),
        "b" => DecodedValue::Signed(if !bytes.is_empty() {
            bytes[0] as i8 as i64
        } else {
            0
        }),
        "B" => DecodedValue::Unsigned(if !bytes.is_empty() {
            bytes[0].into()
        } else {
            0
        }),
        "h" => {
            const SIZE: usize = core::mem::size_of::<c_short>();
            DecodedValue::Signed(if bytes.len() >= SIZE {
                c_short::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")).into()
            } else {
                0
            })
        }
        "H" => {
            const SIZE: usize = core::mem::size_of::<c_ushort>();
            DecodedValue::Unsigned(if bytes.len() >= SIZE {
                c_ushort::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")).into()
            } else {
                0
            })
        }
        "i" => {
            const SIZE: usize = core::mem::size_of::<c_int>();
            DecodedValue::Signed(if bytes.len() >= SIZE {
                c_int::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")).into()
            } else {
                0
            })
        }
        "I" => {
            const SIZE: usize = core::mem::size_of::<c_uint>();
            DecodedValue::Unsigned(if bytes.len() >= SIZE {
                c_uint::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")).into()
            } else {
                0
            })
        }
        "l" => {
            const SIZE: usize = core::mem::size_of::<c_long>();
            DecodedValue::Signed(if bytes.len() >= SIZE {
                #[allow(
                    clippy::unnecessary_cast,
                    clippy::useless_conversion,
                    reason = "c_long width is platform-dependent"
                )]
                let val: i64 =
                    c_long::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")) as i64;
                val
            } else {
                0
            })
        }
        "L" => {
            const SIZE: usize = core::mem::size_of::<c_ulong>();
            DecodedValue::Unsigned(if bytes.len() >= SIZE {
                #[allow(
                    clippy::unnecessary_cast,
                    clippy::useless_conversion,
                    reason = "c_ulong width is platform-dependent"
                )]
                let val: u64 =
                    c_ulong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")) as u64;
                val
            } else {
                0
            })
        }
        "q" => {
            const SIZE: usize = core::mem::size_of::<c_longlong>();
            DecodedValue::Signed(if bytes.len() >= SIZE {
                c_longlong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
            } else {
                0
            })
        }
        "Q" => {
            const SIZE: usize = core::mem::size_of::<c_ulonglong>();
            DecodedValue::Unsigned(if bytes.len() >= SIZE {
                c_ulonglong::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
            } else {
                0
            })
        }
        "f" => {
            const SIZE: usize = core::mem::size_of::<c_float>();
            DecodedValue::Float(if bytes.len() >= SIZE {
                c_float::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked")) as f64
            } else {
                0.0
            })
        }
        "d" | "g" => {
            const SIZE: usize = core::mem::size_of::<c_double>();
            DecodedValue::Float(if bytes.len() >= SIZE {
                c_double::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
            } else {
                0.0
            })
        }
        "?" => DecodedValue::Bool(!bytes.is_empty() && bytes[0] != 0),
        "v" => {
            const SIZE: usize = core::mem::size_of::<c_short>();
            let val = if bytes.len() >= SIZE {
                c_short::from_ne_bytes(bytes[..SIZE].try_into().expect("size checked"))
            } else {
                0
            };
            DecodedValue::Bool(val != 0)
        }
        "z" => unsafe {
            match read_c_string_from_address(read_pointer_from_buffer(bytes)) {
                Some(bytes) => DecodedValue::Bytes(bytes),
                None => DecodedValue::None,
            }
        },
        "Z" => unsafe {
            match read_wide_string_from_address(read_pointer_from_buffer(bytes)) {
                Some(s) => DecodedValue::String(s.to_string()),
                None => DecodedValue::None,
            }
        },
        "P" => DecodedValue::Pointer(read_pointer_from_buffer(bytes)),
        "u" => {
            let val = if bytes.len() >= core::mem::size_of::<WChar>() {
                let wc = if core::mem::size_of::<WChar>() == 2 {
                    u16::from_ne_bytes([bytes[0], bytes[1]]) as u32
                } else {
                    u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                };
                char::from_u32(wc).unwrap_or('\0')
            } else {
                '\0'
            };
            DecodedValue::String(val.to_string())
        }
        _ => DecodedValue::None,
    }
}

/// # Safety
///
/// `ptr` must point to a valid callback argument storage for the given ctypes
/// `type_code`.
pub unsafe fn callback_arg_value(type_code: Option<&str>, ptr: *const c_void) -> DecodedValue {
    match type_code {
        Some("b") => DecodedValue::Signed(unsafe { *(ptr as *const i8) as i64 }),
        Some("B") => DecodedValue::Unsigned(unsafe { *(ptr as *const u8) as u64 }),
        Some("c") => DecodedValue::Bytes(vec![unsafe { *(ptr as *const u8) }]),
        Some("h") => DecodedValue::Signed(unsafe { *(ptr as *const i16) as i64 }),
        Some("H") => DecodedValue::Unsigned(unsafe { *(ptr as *const u16) as u64 }),
        Some("i") => DecodedValue::Signed(unsafe { *(ptr as *const i32) as i64 }),
        Some("I") => DecodedValue::Unsigned(unsafe { *(ptr as *const u32) as u64 }),
        Some("l") => DecodedValue::Signed({
            #[allow(
                clippy::unnecessary_cast,
                clippy::useless_conversion,
                reason = "c_long width is platform-dependent"
            )]
            let val: i64 = unsafe { *(ptr as *const c_long) as i64 };
            val
        }),
        Some("L") => DecodedValue::Unsigned({
            #[allow(
                clippy::unnecessary_cast,
                clippy::useless_conversion,
                reason = "c_ulong width is platform-dependent"
            )]
            let val: u64 = unsafe { *(ptr as *const c_ulong) as u64 };
            val
        }),
        Some("q") => DecodedValue::Signed(unsafe { *(ptr as *const c_longlong) }),
        Some("Q") => DecodedValue::Unsigned(unsafe { *(ptr as *const c_ulonglong) }),
        Some("f") => DecodedValue::Float(unsafe { *(ptr as *const f32) as f64 }),
        Some("d") => DecodedValue::Float(unsafe { *(ptr as *const f64) }),
        Some("z") => {
            let cstr_ptr = unsafe { *(ptr as *const *const c_char) };
            if cstr_ptr.is_null() {
                DecodedValue::None
            } else {
                DecodedValue::Bytes(unsafe { read_c_string_bytes(cstr_ptr) })
            }
        }
        Some("Z") => {
            let wstr_ptr = unsafe { *(ptr as *const *const WChar) };
            if wstr_ptr.is_null() {
                DecodedValue::None
            } else {
                DecodedValue::String(unsafe { read_wide_string(wstr_ptr) }.to_string())
            }
        }
        Some("P") => DecodedValue::Pointer(unsafe { *(ptr as *const usize) }),
        Some("?") => DecodedValue::Bool(unsafe { *(ptr as *const u8) != 0 }),
        _ => DecodedValue::None,
    }
}

/// # Safety
///
/// `args` must point to a libffi callback argument array with a valid entry at
/// `index`, and that entry must be valid for the given ctypes `type_code`.
pub unsafe fn callback_arg_value_at(
    type_code: Option<&str>,
    args: *const *const c_void,
    index: usize,
) -> DecodedValue {
    let ptr = unsafe { *args.add(index) };
    unsafe { callback_arg_value(type_code, ptr) }
}

/// # Safety
///
/// `result` must point to valid callback result storage for the given ctypes
/// `type_code`.
pub unsafe fn write_callback_result(
    type_code: Option<&str>,
    result: *mut c_void,
    value: CallbackResultValue,
) {
    match (type_code, value) {
        (Some("b"), CallbackResultValue::Signed(v)) => unsafe { *(result as *mut i8) = v as i8 },
        (Some("B" | "c"), CallbackResultValue::Unsigned(v)) => unsafe {
            *(result as *mut u8) = v as u8
        },
        (Some("h"), CallbackResultValue::Signed(v)) => unsafe { *(result as *mut i16) = v as i16 },
        (Some("H"), CallbackResultValue::Unsigned(v)) => unsafe {
            *(result as *mut u16) = v as u16
        },
        (Some("i"), CallbackResultValue::Signed(v)) => unsafe {
            *(result as *mut CallbackIntResult) = v as i32 as CallbackIntResult
        },
        (Some("I"), CallbackResultValue::Unsigned(v)) => unsafe {
            *(result as *mut u32) = v as u32
        },
        (Some("l"), CallbackResultValue::Signed(v)) => unsafe {
            *(result as *mut c_long) = v as c_long
        },
        (Some("L"), CallbackResultValue::Unsigned(v)) => unsafe {
            *(result as *mut c_ulong) = v as c_ulong
        },
        (Some("q"), CallbackResultValue::Signed(v)) => unsafe { *(result as *mut i64) = v },
        (Some("Q"), CallbackResultValue::Unsigned(v)) => unsafe { *(result as *mut u64) = v },
        (Some("f"), CallbackResultValue::Float(v)) => unsafe { *(result as *mut f32) = v as f32 },
        (Some("d"), CallbackResultValue::Float(v)) => unsafe { *(result as *mut f64) = v },
        (Some("P" | "z" | "Z"), CallbackResultValue::Pointer(v)) => unsafe {
            *(result as *mut usize) = v
        },
        (Some("?"), CallbackResultValue::Bool(v)) => unsafe { *(result as *mut u8) = u8::from(v) },
        _ => {}
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_value_from_type_code(type_code: &str, buffer: &[u8]) -> FfiValue {
    match type_code {
        "c" | "b" => FfiValue::I8(buffer.first().map(|&b| b as i8).unwrap_or(0)),
        "B" => FfiValue::U8(buffer.first().copied().unwrap_or(0)),
        "h" => FfiValue::I16(buffer.first_chunk().copied().map_or(0, i16::from_ne_bytes)),
        "H" => FfiValue::U16(buffer.first_chunk().copied().map_or(0, u16::from_ne_bytes)),
        "i" => FfiValue::I32(buffer.first_chunk().copied().map_or(0, i32::from_ne_bytes)),
        "I" => FfiValue::U32(buffer.first_chunk().copied().map_or(0, u32::from_ne_bytes)),
        "l" | "q" => FfiValue::I64(if let Some(&bytes) = buffer.first_chunk::<8>() {
            i64::from_ne_bytes(bytes)
        } else if let Some(&bytes) = buffer.first_chunk::<4>() {
            i32::from_ne_bytes(bytes).into()
        } else {
            0
        }),
        "L" | "Q" => FfiValue::U64(if let Some(&bytes) = buffer.first_chunk::<8>() {
            u64::from_ne_bytes(bytes)
        } else if let Some(&bytes) = buffer.first_chunk::<4>() {
            u32::from_ne_bytes(bytes).into()
        } else {
            0
        }),
        "f" => FfiValue::F32(
            buffer
                .first_chunk::<4>()
                .copied()
                .map_or(0.0, f32::from_ne_bytes),
        ),
        "d" | "g" => FfiValue::F64(
            buffer
                .first_chunk::<8>()
                .copied()
                .map_or(0.0, f64::from_ne_bytes),
        ),
        "z" | "Z" | "P" | "O" => FfiValue::Pointer(read_pointer_from_buffer(buffer)),
        "?" => FfiValue::U8(if buffer.first().map(|&b| b != 0).unwrap_or(false) {
            1
        } else {
            0
        }),
        "u" => FfiValue::U32(buffer.first_chunk().copied().map_or(0, u32::from_ne_bytes)),
        _ => FfiValue::Pointer(0),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_value_from_type(buffer: &[u8], ty: Type) -> Option<FfiValue> {
    if core::ptr::eq(ty.as_raw_ptr(), Type::u8().as_raw_ptr()) {
        Some(FfiValue::U8(*buffer.first()?))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::i8().as_raw_ptr()) {
        Some(FfiValue::I8(*buffer.first()? as i8))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::u16().as_raw_ptr()) {
        Some(FfiValue::U16(u16::from_ne_bytes(
            *buffer.first_chunk::<2>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::i16().as_raw_ptr()) {
        Some(FfiValue::I16(i16::from_ne_bytes(
            *buffer.first_chunk::<2>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::u32().as_raw_ptr()) {
        Some(FfiValue::U32(u32::from_ne_bytes(
            *buffer.first_chunk::<4>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::i32().as_raw_ptr()) {
        Some(FfiValue::I32(i32::from_ne_bytes(
            *buffer.first_chunk::<4>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::u64().as_raw_ptr()) {
        Some(FfiValue::U64(u64::from_ne_bytes(
            *buffer.first_chunk::<8>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::i64().as_raw_ptr()) {
        Some(FfiValue::I64(i64::from_ne_bytes(
            *buffer.first_chunk::<8>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::f32().as_raw_ptr()) {
        Some(FfiValue::F32(f32::from_ne_bytes(
            *buffer.first_chunk::<4>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::f64().as_raw_ptr()) {
        Some(FfiValue::F64(f64::from_ne_bytes(
            *buffer.first_chunk::<8>()?,
        )))
    } else if core::ptr::eq(ty.as_raw_ptr(), Type::pointer().as_raw_ptr()) {
        Some(FfiValue::Pointer(read_pointer_from_buffer(buffer)))
    } else {
        None
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_type_from_code(ty: &str) -> Option<Type> {
    match ty {
        "c" => Some(Type::u8()),
        "u" => Some(if core::mem::size_of::<WChar>() == 2 {
            Type::u16()
        } else {
            Type::u32()
        }),
        "b" => Some(Type::i8()),
        "B" | "?" => Some(Type::u8()),
        "h" | "v" => Some(Type::i16()),
        "H" => Some(Type::u16()),
        "i" => Some(Type::i32()),
        "I" => Some(Type::u32()),
        "l" => Some(if core::mem::size_of::<c_long>() == 8 {
            Type::i64()
        } else {
            Type::i32()
        }),
        "L" => Some(if core::mem::size_of::<c_ulong>() == 8 {
            Type::u64()
        } else {
            Type::u32()
        }),
        "q" => Some(Type::i64()),
        "Q" => Some(Type::u64()),
        "f" => Some(Type::f32()),
        "d" | "g" => Some(Type::f64()),
        "z" | "Z" | "P" | "X" | "O" => Some(Type::pointer()),
        "void" => Some(Type::void()),
        _ => None,
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_type_from_tag(tag: u8) -> Type {
    match tag {
        b'c' | b'b' => Type::i8(),
        b'B' | b'?' => Type::u8(),
        b'h' | b'v' => Type::i16(),
        b'H' => Type::u16(),
        b'i' => Type::i32(),
        b'I' => Type::u32(),
        b'l' => {
            if core::mem::size_of::<c_long>() == 8 {
                Type::i64()
            } else {
                Type::i32()
            }
        }
        b'L' => {
            if core::mem::size_of::<c_ulong>() == 8 {
                Type::u64()
            } else {
                Type::u32()
            }
        }
        b'q' => Type::i64(),
        b'Q' => Type::u64(),
        b'f' => Type::f32(),
        b'd' | b'g' => Type::f64(),
        b'u' => {
            if core::mem::size_of::<WChar>() == 2 {
                Type::u16()
            } else {
                Type::u32()
            }
        }
        _ => Type::pointer(),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_type_from_format(fmt: &str) -> Type {
    match fmt.trim_start_matches(['<', '>', '!', '@', '=']) {
        "b" => Type::i8(),
        "B" => Type::u8(),
        "h" => Type::i16(),
        "H" => Type::u16(),
        "i" | "l" => Type::i32(),
        "I" | "L" => Type::u32(),
        "q" => Type::i64(),
        "Q" => Type::u64(),
        "f" => Type::f32(),
        "d" => Type::f64(),
        "P" | "z" | "Z" | "O" => Type::pointer(),
        _ => Type::u8(),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_repeat_type(elem_type: Type, len: usize) -> Type {
    Type::structure(core::iter::repeat_n(elem_type, len))
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_byte_struct(size: usize) -> Type {
    ffi_repeat_type(Type::u8(), size)
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_pointer_type() -> Type {
    Type::pointer()
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_i32_type() -> Type {
    Type::i32()
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_f64_type() -> Type {
    Type::f64()
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_void_type() -> Type {
    Type::void()
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_type_for_return_size(size: usize) -> Type {
    if size <= 4 {
        Type::i32()
    } else if size <= 8 {
        Type::i64()
    } else {
        Type::pointer()
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CTypeParamKind {
    Structure,
    Union,
    Array,
    Pointer,
    Simple,
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_type_for_layout(
    kind: CTypeParamKind,
    ffi_field_types: &[Type],
    size: usize,
    length: usize,
    format: Option<&str>,
) -> Type {
    const MAX_FFI_STRUCT_SIZE: usize = 1024 * 1024;

    match kind {
        CTypeParamKind::Structure | CTypeParamKind::Union => {
            if !ffi_field_types.is_empty() {
                Type::structure(ffi_field_types.iter().cloned())
            } else if size <= MAX_FFI_STRUCT_SIZE {
                ffi_byte_struct(size)
            } else {
                ffi_pointer_type()
            }
        }
        CTypeParamKind::Array => {
            if size > MAX_FFI_STRUCT_SIZE || length > MAX_FFI_STRUCT_SIZE {
                ffi_pointer_type()
            } else if let Some(fmt) = format {
                ffi_repeat_type(ffi_type_from_format(fmt), length)
            } else {
                ffi_byte_struct(size)
            }
        }
        CTypeParamKind::Pointer => ffi_pointer_type(),
        CTypeParamKind::Simple => {
            if let Some(fmt) = format {
                ffi_type_from_format(fmt)
            } else {
                Type::u8()
            }
        }
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn callproc(
    code_ptr: CodePtr,
    ffi_arg_types: Vec<Type>,
    ffi_return_type: Type,
    ffi_args: &[Arg<'_>],
    restype_is_none: bool,
    is_pointer_return: bool,
) -> CallResult {
    let cif = Cif::new(ffi_arg_types, ffi_return_type);
    if restype_is_none {
        unsafe { cif.call::<()>(code_ptr, ffi_args) };
        CallResult::Void
    } else if is_pointer_return {
        CallResult::Pointer(unsafe { cif.call::<usize>(code_ptr, ffi_args) })
    } else {
        CallResult::Value(unsafe { cif.call::<low::ffi_arg>(code_ptr, ffi_args) })
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn call_cdecl_i32(code_ptr: usize, arg_types: Vec<Type>, arg_values: &[isize]) -> c_int {
    let ffi_args: Vec<_> = arg_values.iter().map(Arg::new).collect();
    let cif = Cif::new(arg_types, Type::c_int());
    let code_ptr = CodePtr::from_ptr(code_ptr as *const _);
    unsafe { cif.call(code_ptr, &ffi_args) }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn call_cdecl_i32_values(code_ptr: usize, args: &[CdeclArgValue]) -> c_int {
    let mut arg_values = Vec::with_capacity(args.len());
    let mut arg_types = Vec::with_capacity(args.len());
    for arg in args {
        match *arg {
            CdeclArgValue::Pointer(value) => {
                arg_values.push(value);
                arg_types.push(Type::pointer());
            }
            CdeclArgValue::Int(value) => {
                arg_values.push(value);
                arg_types.push(Type::isize());
            }
        }
    }
    call_cdecl_i32(code_ptr, arg_types, &arg_values)
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_arg(value: FfiArgRef<'_>) -> Arg<'_> {
    match value {
        FfiArgRef::U8(v) => Arg::new(v),
        FfiArgRef::I8(v) => Arg::new(v),
        FfiArgRef::U16(v) => Arg::new(v),
        FfiArgRef::I16(v) => Arg::new(v),
        FfiArgRef::U32(v) => Arg::new(v),
        FfiArgRef::I32(v) => Arg::new(v),
        FfiArgRef::U64(v) => Arg::new(v),
        FfiArgRef::I64(v) => Arg::new(v),
        FfiArgRef::F32(v) => Arg::new(v),
        FfiArgRef::F64(v) => Arg::new(v),
        FfiArgRef::Pointer(v) => Arg::new(v),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn ffi_arg_from_value(value: &FfiValue) -> Arg<'_> {
    match value {
        FfiValue::U8(v) => ffi_arg(FfiArgRef::U8(v)),
        FfiValue::I8(v) => ffi_arg(FfiArgRef::I8(v)),
        FfiValue::U16(v) => ffi_arg(FfiArgRef::U16(v)),
        FfiValue::I16(v) => ffi_arg(FfiArgRef::I16(v)),
        FfiValue::U32(v) => ffi_arg(FfiArgRef::U32(v)),
        FfiValue::I32(v) => ffi_arg(FfiArgRef::I32(v)),
        FfiValue::U64(v) => ffi_arg(FfiArgRef::U64(v)),
        FfiValue::I64(v) => ffi_arg(FfiArgRef::I64(v)),
        FfiValue::F32(v) => ffi_arg(FfiArgRef::F32(v)),
        FfiValue::F64(v) => ffi_arg(FfiArgRef::F64(v)),
        FfiValue::Pointer(v) => ffi_arg(FfiArgRef::Pointer(v)),
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn code_ptr_from_addr(addr: usize) -> Option<CodePtr> {
    if addr == 0 {
        None
    } else {
        Some(CodePtr(addr as *mut _))
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn null_code_ptr() -> CodePtr {
    CodePtr(core::ptr::null_mut())
}

#[cfg(windows)]
pub enum ComMethodError {
    NullComPointer,
    NullVtablePointer,
    NullFunctionPointer,
}

#[cfg(windows)]
pub const HRESULT_E_POINTER: i32 = crate::windows::HRESULT_E_POINTER;

#[cfg(windows)]
pub const HRESULT_S_OK: i32 = crate::windows::HRESULT_S_OK;

#[cfg(windows)]
pub fn format_error_message(code: Option<u32>) -> Option<String> {
    crate::windows::format_error_message(code)
}

#[cfg(windows)]
pub fn resolve_com_vtable_entry(com_ptr: usize, idx: usize) -> Result<CodePtr, ComMethodError> {
    if com_ptr == 0 {
        return Err(ComMethodError::NullComPointer);
    }
    let vtable_ptr = unsafe { *(com_ptr as *const usize) };
    if vtable_ptr == 0 {
        return Err(ComMethodError::NullVtablePointer);
    }
    let fptr = unsafe {
        let vtable = vtable_ptr as *const usize;
        *vtable.add(idx)
    };
    if fptr == 0 {
        return Err(ComMethodError::NullFunctionPointer);
    }
    Ok(CodePtr(fptr as *mut _))
}

#[cfg(windows)]
pub fn copy_com_pointer(src_ptr: usize, dst_addr: usize) -> i32 {
    if dst_addr == 0 {
        return HRESULT_E_POINTER;
    }

    if src_ptr != 0 {
        unsafe {
            let iunknown = src_ptr as *mut *const usize;
            let vtable = *iunknown;
            if vtable.is_null() {
                return HRESULT_E_POINTER;
            }
            let addref_fn: extern "system" fn(*mut c_void) -> u32 =
                core::mem::transmute(*vtable.add(1));
            addref_fn(src_ptr as *mut c_void);
        }
    }

    unsafe {
        *(dst_addr as *mut usize) = src_ptr;
    }

    HRESULT_S_OK
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub struct CallbackThunk<U: 'static> {
    #[allow(dead_code)]
    closure: Closure<'static>,
    userdata_ptr: *mut U,
    code_ptr: CodePtr,
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
impl<U: 'static> CallbackThunk<U> {
    pub fn new(
        ffi_arg_types: Vec<Type>,
        ffi_res_type: Type,
        userdata: Box<U>,
        callback: unsafe extern "C" fn(&low::ffi_cif, &mut c_void, *const *const c_void, &U),
    ) -> Self {
        let cif = Cif::new(ffi_arg_types, ffi_res_type);
        let userdata_ptr = Box::into_raw(userdata);
        let userdata_ref: &'static U = unsafe { &*userdata_ptr };
        let closure = Closure::new(cif, callback, userdata_ref);
        let code_ptr = CodePtr(*closure.code_ptr() as *mut _);
        Self {
            closure,
            userdata_ptr,
            code_ptr,
        }
    }

    pub fn code_ptr(&self) -> CodePtr {
        self.code_ptr
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
impl<U: 'static> Drop for CallbackThunk<U> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.userdata_ptr));
        }
    }
}

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
pub fn call_result_bytes(raw_result: &CallResult) -> Option<(Vec<u8>, usize)> {
    match raw_result {
        CallResult::Void => None,
        CallResult::Pointer(ptr) => {
            let bytes = ptr.to_ne_bytes();
            Some((bytes.to_vec(), core::mem::size_of::<usize>()))
        }
        CallResult::Value(val) => {
            let bytes = val.to_ne_bytes();
            Some((bytes.to_vec(), core::mem::size_of_val(val)))
        }
    }
}

/// # Safety
///
/// `ptr` must point to `len` readable bytes.
pub unsafe fn bytes_at(ptr: *const u8, len: usize) -> Vec<u8> {
    unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec()
}

/// # Safety
///
/// The caller must ensure `ptr..ptr+size` remains valid for the lifetime of the returned slice.
pub unsafe fn borrow_memory(ptr: *const u8, size: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(ptr, size) }
}

/// # Safety
///
/// The caller must ensure `ptr..ptr+size` remains valid and uniquely borrowed for the lifetime of the returned slice.
pub unsafe fn borrow_memory_mut(ptr: *mut u8, size: usize) -> &'static mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(ptr, size) }
}

/// # Safety
///
/// `slice` must point to memory that is valid and writable for its full length.
#[allow(
    clippy::mut_from_ref,
    reason = "ctypes borrowed buffers may wrap writable memory behind a shared slice"
)]
pub unsafe fn borrowed_slice_as_mut(slice: &[u8]) -> &mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(slice.as_ptr() as *mut u8, slice.len()) }
}

pub fn wide_chars_to_wtf8(wchars: &[WChar]) -> Wtf8Buf {
    #[cfg(windows)]
    {
        let wide: Vec<u16> = wchars.to_vec();
        Wtf8Buf::from_wide(&wide)
    }
    #[cfg(not(windows))]
    {
        #[allow(
            clippy::useless_conversion,
            reason = "wchar_t is i32 on some platforms and u32 on others"
        )]
        let s: String = wchars
            .iter()
            .filter_map(|&c| u32::try_from(c).ok().and_then(char::from_u32))
            .collect();
        Wtf8Buf::from_string(s)
    }
}

/// # Safety
///
/// `ptr` must be a valid NUL-terminated wide C string.
pub unsafe fn read_wide_string(ptr: *const WChar) -> Wtf8Buf {
    let len = unsafe { wcslen(ptr) };
    let wchars = unsafe { core::slice::from_raw_parts(ptr, len) };
    wide_chars_to_wtf8(wchars)
}

/// # Safety
///
/// `addr` must either be zero or a valid NUL-terminated C string pointer.
pub unsafe fn read_c_string_from_address(addr: usize) -> Option<Vec<u8>> {
    if addr == 0 {
        None
    } else {
        Some(unsafe { read_c_string_bytes(addr as *const c_char) })
    }
}

/// # Safety
///
/// `addr` must either be zero or a valid NUL-terminated wide C string pointer.
pub unsafe fn read_wide_string_from_address(addr: usize) -> Option<Wtf8Buf> {
    if addr == 0 {
        None
    } else {
        Some(unsafe { read_wide_string(addr as *const WChar) })
    }
}

/// # Safety
///
/// `ptr` must point to `len` readable wide characters.
pub unsafe fn read_wide_string_with_len(ptr: *const WChar, len: usize) -> Wtf8Buf {
    let wchars = unsafe { core::slice::from_raw_parts(ptr, len) };
    wide_chars_to_wtf8(wchars)
}

pub fn string_at(ptr: usize, size: isize) -> Result<Vec<u8>, StringAtError> {
    if ptr == 0 {
        return Err(StringAtError::NullPointer);
    }
    if size < 0 {
        // SAFETY: caller passed a non-null C string pointer; same precondition as previous VM path.
        return Ok(unsafe { read_c_string_bytes(ptr as _) });
    }
    let len = {
        let size_usize = size as usize;
        if size_usize > isize::MAX as usize / 2 {
            return Err(StringAtError::TooLong);
        }
        size_usize
    };
    // SAFETY: caller requested exactly `len` readable bytes from non-null pointer.
    Ok(unsafe { bytes_at(ptr as *const u8, len) })
}

pub fn wstring_at(ptr: usize, size: isize) -> Result<Wtf8Buf, StringAtError> {
    if ptr == 0 {
        return Err(StringAtError::NullPointer);
    }
    let w_ptr = ptr as *const WChar;
    if size < 0 {
        // SAFETY: caller passed a non-null NUL-terminated wide string pointer.
        return Ok(unsafe { read_wide_string(w_ptr) });
    }
    let len = {
        let size_usize = size as usize;
        if size_usize > isize::MAX as usize / core::mem::size_of::<WChar>() {
            return Err(StringAtError::TooLong);
        }
        size_usize
    };
    // SAFETY: caller requested exactly `len` readable wide characters from non-null pointer.
    Ok(unsafe { read_wide_string_with_len(w_ptr, len) })
}

/// # Safety
///
/// `start` must be valid to read `len` elements following `step`.
pub unsafe fn read_bytes_strided(start: *const u8, len: usize, step: isize) -> Vec<u8> {
    if step == 1 {
        return unsafe { bytes_at(start, len) };
    }
    let mut result = Vec::with_capacity(len);
    let mut cur = start;
    for _ in 0..len {
        result.push(unsafe { *cur });
        cur = unsafe { cur.offset(step) };
    }
    result
}

pub fn pointer_item_address(ptr_value: usize, index: isize, element_size: usize) -> usize {
    let offset = index * element_size as isize;
    (ptr_value as isize + offset) as usize
}

pub fn offset_address(base: usize, offset: isize) -> usize {
    (base as isize + offset) as usize
}

/// # Safety
///
/// `ptr_value + start * element_size` must be valid to read `len` bytes following
/// `step * element_size`.
pub unsafe fn read_pointer_char_slice(
    ptr_value: usize,
    start: isize,
    len: usize,
    step: isize,
    element_size: usize,
) -> Vec<u8> {
    let start_addr = pointer_item_address(ptr_value, start, element_size) as *const u8;
    if step == 1 {
        unsafe { bytes_at(start_addr, len) }
    } else {
        unsafe { read_bytes_strided(start_addr, len, step * element_size as isize) }
    }
}

/// # Safety
///
/// `start` must be valid to read `len` wide characters following `step`.
pub unsafe fn read_wide_string_strided(start: *const WChar, len: usize, step: isize) -> Wtf8Buf {
    if step == 1 {
        return unsafe { read_wide_string_with_len(start, len) };
    }
    let mut wchars = Vec::with_capacity(len);
    let mut cur = start;
    for _ in 0..len {
        wchars.push(unsafe { *cur });
        cur = unsafe { cur.offset(step) };
    }
    wide_chars_to_wtf8(&wchars)
}

/// # Safety
///
/// `ptr_value + start * sizeof(wchar_t)` must be valid to read `len` wide
/// characters following `step`.
pub unsafe fn read_pointer_wchar_slice(
    ptr_value: usize,
    start: isize,
    len: usize,
    step: isize,
) -> Wtf8Buf {
    let wchar_size = core::mem::size_of::<WChar>();
    let start_addr = (ptr_value as isize + start * wchar_size as isize) as *const WChar;
    unsafe { read_wide_string_strided(start_addr, len, step) }
}

/// # Safety
///
/// `addr` must be readable for `size` bytes and match the alignment/validity
/// requirements implied by `type_code`.
pub unsafe fn read_value_at_address(
    addr: usize,
    size: usize,
    type_code: Option<&str>,
) -> AddressValue {
    let ptr = addr as *const u8;
    match type_code {
        Some("c") => AddressValue::ByteString(unsafe { *ptr }),
        Some("b") => AddressValue::Integer(IntegerValue::Signed(unsafe { *ptr as i8 as i64 })),
        Some("B") => AddressValue::Integer(IntegerValue::Unsigned(unsafe { (*ptr).into() })),
        Some("h") => AddressValue::Integer(IntegerValue::Signed(
            unsafe { core::ptr::read_unaligned(ptr as *const i16) }.into(),
        )),
        Some("H") => AddressValue::Integer(IntegerValue::Unsigned(
            unsafe { core::ptr::read_unaligned(ptr as *const u16) }.into(),
        )),
        Some("i") => AddressValue::Integer(IntegerValue::Signed(
            unsafe { core::ptr::read_unaligned(ptr as *const i32) }.into(),
        )),
        Some("I") => AddressValue::Integer(IntegerValue::Unsigned(
            unsafe { core::ptr::read_unaligned(ptr as *const u32) }.into(),
        )),
        Some("l") => AddressValue::Integer(IntegerValue::Signed(unsafe {
            core::ptr::read_unaligned(ptr as *const c_long)
        } as i64)),
        Some("L") => AddressValue::Integer(IntegerValue::Unsigned(unsafe {
            core::ptr::read_unaligned(ptr as *const c_ulong)
        } as u64)),
        Some("q") => AddressValue::Integer(IntegerValue::Signed(unsafe {
            core::ptr::read_unaligned(ptr as *const i64)
        })),
        Some("Q") => AddressValue::Integer(IntegerValue::Unsigned(unsafe {
            core::ptr::read_unaligned(ptr as *const u64)
        })),
        Some("f") => {
            AddressValue::Float(unsafe { core::ptr::read_unaligned(ptr as *const f32) as f64 })
        }
        Some("d" | "g") => {
            AddressValue::Float(unsafe { core::ptr::read_unaligned(ptr as *const f64) })
        }
        Some("P" | "z" | "Z") => {
            AddressValue::Pointer(unsafe { core::ptr::read_unaligned(ptr as *const usize) })
        }
        _ => AddressValue::Bytes(unsafe { bytes_at(ptr, size) }),
    }
}

/// # Safety
///
/// `addr` must be valid to write one `u8`.
pub unsafe fn write_u8_at_address(addr: usize, value: u8) {
    unsafe { *(addr as *mut u8) = value };
}

/// # Safety
///
/// `addr` must be valid to write one `i16`.
pub unsafe fn write_i16_at_address(addr: usize, value: i16) {
    unsafe { core::ptr::write_unaligned(addr as *mut i16, value) };
}

/// # Safety
///
/// `addr` must be valid to write one `i32`.
pub unsafe fn write_i32_at_address(addr: usize, value: i32) {
    unsafe { core::ptr::write_unaligned(addr as *mut i32, value) };
}

/// # Safety
///
/// `addr` must be valid to write one `i64`.
pub unsafe fn write_i64_at_address(addr: usize, value: i64) {
    unsafe { core::ptr::write_unaligned(addr as *mut i64, value) };
}

/// # Safety
///
/// `addr` must be valid to write one `usize`.
pub unsafe fn write_pointer_at_address(addr: usize, value: usize) {
    unsafe { core::ptr::write_unaligned(addr as *mut usize, value) };
}

/// # Safety
///
/// `addr` must be valid to write one `f32`.
pub unsafe fn write_f32_at_address(addr: usize, value: f32) {
    unsafe { core::ptr::write_unaligned(addr as *mut f32, value) };
}

/// # Safety
///
/// `addr` must be valid to write one `f64`.
pub unsafe fn write_f64_at_address(addr: usize, value: f64) {
    unsafe { core::ptr::write_unaligned(addr as *mut f64, value) };
}

/// # Safety
///
/// `addr` must be valid for writing the storage required by `value`.
pub unsafe fn write_value_to_address(addr: usize, size: usize, value: AddressWriteValue<'_>) {
    match value {
        AddressWriteValue::Pointer(value) => unsafe { write_pointer_at_address(addr, value) },
        AddressWriteValue::U8(value) => unsafe { write_u8_at_address(addr, value) },
        AddressWriteValue::I16(value) => unsafe { write_i16_at_address(addr, value) },
        AddressWriteValue::I32(value) => unsafe { write_i32_at_address(addr, value) },
        AddressWriteValue::I64(value) => unsafe { write_i64_at_address(addr, value) },
        AddressWriteValue::Float(value) => match size {
            4 => unsafe { write_f32_at_address(addr, value as f32) },
            8 => unsafe { write_f64_at_address(addr, value) },
            _ => {}
        },
        AddressWriteValue::Bytes(bytes) => unsafe { copy_bytes_to_address(addr, bytes, size) },
    }
}

/// # Safety
///
/// `addr` must be valid to write `min(bytes.len(), size)` bytes.
pub unsafe fn copy_bytes_to_address(addr: usize, bytes: &[u8], size: usize) {
    let copy_len = bytes.len().min(size);
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, copy_len) };
}

pub fn write_simple_storage_buffer(buffer: &mut Cow<'_, [u8]>, bytes: &[u8]) {
    match buffer {
        Cow::Borrowed(slice) => {
            // SAFETY: ctypes borrowed buffers are created only from writable Python buffers.
            unsafe {
                copy_bytes_to_address(slice.as_ptr() as usize, bytes, slice.len());
            }
        }
        Cow::Owned(vec) => {
            vec.copy_from_slice(bytes);
        }
    }
}

pub fn write_cow_bytes_at_offset(buffer: &mut Cow<'_, [u8]>, offset: usize, bytes: &[u8]) {
    if offset + bytes.len() > buffer.len() {
        return;
    }

    match buffer {
        Cow::Borrowed(slice) => {
            // SAFETY: callers only construct borrowed ctypes buffers for writable memory.
            unsafe {
                copy_bytes_to_address(slice.as_ptr() as usize + offset, bytes, bytes.len());
            }
        }
        Cow::Owned(vec) => {
            vec[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }
}

pub fn resize_owned_bytes(old_data: &[u8], new_size: usize) -> Vec<u8> {
    let mut new_data = vec![0u8; new_size];
    let copy_len = old_data.len().min(new_size);
    new_data[..copy_len].copy_from_slice(&old_data[..copy_len]);
    new_data
}

#[cfg(any(unix, windows, target_os = "wasi"))]
pub fn memmove_addr() -> usize {
    libc::memmove as *const () as usize
}

#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub fn memmove_addr() -> usize {
    0
}

#[cfg(any(unix, windows, target_os = "wasi"))]
pub fn memset_addr() -> usize {
    libc::memset as *const () as usize
}

#[cfg(not(any(unix, windows, target_os = "wasi")))]
pub fn memset_addr() -> usize {
    0
}

#[cfg(any(unix, windows))]
pub enum LookupSymbolError {
    LibraryNotFound,
    LibraryClosed,
    Load(String),
}

#[cfg(any(unix, windows))]
struct SharedLibrary {
    lib: Mutex<Option<Library>>,
}

#[cfg(any(unix, windows))]
impl SharedLibrary {
    #[cfg(windows)]
    fn new(name: impl AsRef<OsStr>) -> Result<Self, libloading::Error> {
        Ok(Self {
            lib: Mutex::new(unsafe { Some(Library::new(name.as_ref())?) }),
        })
    }

    #[cfg(unix)]
    fn new_with_mode(name: impl AsRef<OsStr>, mode: i32) -> Result<Self, libloading::Error> {
        Ok(Self {
            lib: Mutex::new(Some(unsafe {
                UnixLibrary::open(Some(name.as_ref()), mode)?.into()
            })),
        })
    }

    #[cfg(unix)]
    fn from_raw_handle(handle: *mut c_void) -> Self {
        Self {
            lib: Mutex::new(Some(unsafe { UnixLibrary::from_raw(handle).into() })),
        }
    }

    fn get_pointer(&self) -> usize {
        let lib_lock = self.lib.lock();
        if let Some(l) = &*lib_lock {
            unsafe { core::mem::transmute_copy::<Library, usize>(l) }
        } else {
            0
        }
    }

    fn lookup_data_symbol_addr(&self, symbol_name: &[u8]) -> Result<usize, LookupSymbolError> {
        let lib_lock = self.lib.lock();
        let Some(lib) = &*lib_lock else {
            return Err(LookupSymbolError::LibraryClosed);
        };
        let pointer = unsafe {
            lib.get::<*const u8>(symbol_name)
                .map_err(|err| LookupSymbolError::Load(err.to_string()))?
        };
        Ok(*pointer as usize)
    }

    fn lookup_function_symbol_addr(&self, symbol_name: &[u8]) -> Result<usize, LookupSymbolError> {
        let lib_lock = self.lib.lock();
        let Some(lib) = &*lib_lock else {
            return Err(LookupSymbolError::LibraryClosed);
        };
        let pointer = unsafe {
            lib.get::<unsafe extern "C" fn()>(symbol_name)
                .map_err(|err| LookupSymbolError::Load(err.to_string()))?
        };
        Ok(*pointer as *const () as usize)
    }
}

#[cfg(any(unix, windows))]
struct ExternalLibs {
    libraries: HashMap<usize, SharedLibrary>,
}

#[cfg(any(unix, windows))]
impl ExternalLibs {
    fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    fn get_lib(&self, key: usize) -> Option<&SharedLibrary> {
        self.libraries.get(&key)
    }

    #[cfg(windows)]
    fn open_library(
        &mut self,
        library_path: impl AsRef<OsStr>,
    ) -> Result<usize, libloading::Error> {
        let new_lib = SharedLibrary::new(library_path)?;
        let key = new_lib.get_pointer();
        if self.libraries.contains_key(&key) {
            drop(new_lib);
            return Ok(key);
        }
        self.libraries.insert(key, new_lib);
        Ok(key)
    }

    #[cfg(unix)]
    fn open_library_with_mode(
        &mut self,
        library_path: impl AsRef<OsStr>,
        mode: i32,
    ) -> Result<usize, libloading::Error> {
        let new_lib = SharedLibrary::new_with_mode(library_path, mode)?;
        let key = new_lib.get_pointer();
        if self.libraries.contains_key(&key) {
            drop(new_lib);
            return Ok(key);
        }
        self.libraries.insert(key, new_lib);
        Ok(key)
    }

    #[cfg(unix)]
    fn insert_raw_library_handle(&mut self, handle: *mut c_void) -> usize {
        let key = handle as usize;
        self.libraries
            .insert(key, SharedLibrary::from_raw_handle(handle));
        key
    }

    fn drop_library(&mut self, key: usize) {
        self.libraries.remove(&key);
    }
}

#[cfg(any(unix, windows))]
fn libcache() -> &'static RwLock<ExternalLibs> {
    static LIBCACHE: OnceLock<RwLock<ExternalLibs>> = OnceLock::new();
    LIBCACHE.get_or_init(|| RwLock::new(ExternalLibs::new()))
}

#[cfg(windows)]
pub fn open_library(name: impl AsRef<OsStr>) -> Result<usize, libloading::Error> {
    libcache().write().open_library(name)
}

#[cfg(unix)]
pub fn open_library_with_mode(
    name: impl AsRef<OsStr>,
    mode: i32,
) -> Result<usize, libloading::Error> {
    libcache().write().open_library_with_mode(name, mode)
}

#[cfg(not(unix))]
pub fn open_library_with_mode(
    _name: impl AsRef<std::ffi::OsStr>,
    _mode: i32,
) -> Result<usize, String> {
    Err("dlopen() error".to_string())
}

#[cfg(unix)]
pub fn insert_raw_library_handle(handle: *mut c_void) -> usize {
    libcache().write().insert_raw_library_handle(handle)
}

#[cfg(not(unix))]
pub fn insert_raw_library_handle(_handle: *mut c_void) -> usize {
    0
}

#[cfg(any(unix, windows))]
pub fn drop_library(handle: usize) {
    libcache().write().drop_library(handle);
}

#[cfg(not(any(unix, windows)))]
pub fn drop_library(_handle: usize) {}

#[cfg(any(unix, windows))]
pub fn lookup_data_symbol_addr(
    handle: usize,
    symbol_name: &[u8],
) -> Result<usize, LookupSymbolError> {
    let cache = libcache().read();
    cache
        .get_lib(handle)
        .ok_or(LookupSymbolError::LibraryNotFound)?
        .lookup_data_symbol_addr(symbol_name)
}

#[cfg(any(unix, windows))]
pub fn lookup_function_symbol_addr(
    handle: usize,
    symbol_name: &[u8],
) -> Result<usize, LookupSymbolError> {
    let cache = libcache().read();
    cache
        .get_lib(handle)
        .ok_or(LookupSymbolError::LibraryNotFound)?
        .lookup_function_symbol_addr(symbol_name)
}

#[cfg(all(unix, not(target_os = "wasi")))]
pub fn dlopen_self(mode: c_int) -> Result<*mut c_void, String> {
    let handle = unsafe { libc::dlopen(core::ptr::null(), mode) };
    if handle.is_null() {
        let err = unsafe { libc::dlerror() };
        Err(if err.is_null() {
            "dlopen() error".to_string()
        } else {
            unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned()
        })
    } else {
        Ok(handle)
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "wasi")))))]
pub fn dlopen_self(_mode: c_int) -> Result<*mut c_void, String> {
    Err("dlopen() error".to_string())
}

#[cfg(all(unix, not(target_os = "wasi")))]
pub fn dlsym_checked(handle: usize, symbol_name: &CStr) -> Result<*mut c_void, String> {
    unsafe {
        libc::dlerror();
    }

    let ptr = unsafe { libc::dlsym(handle as *mut c_void, symbol_name.as_ptr()) };
    let err = unsafe { libc::dlerror() };
    if !err.is_null() {
        return Err(unsafe { CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned());
    }
    if ptr.is_null() {
        return Err(format!(
            "symbol '{}' not found",
            symbol_name.to_string_lossy()
        ));
    }
    Ok(ptr)
}

#[cfg(not(any(windows, all(unix, not(target_os = "wasi")))))]
pub fn dlsym_checked(_handle: usize, symbol_name: &CStr) -> Result<*mut c_void, String> {
    Err(format!(
        "symbol '{}' not found",
        symbol_name.to_string_lossy()
    ))
}

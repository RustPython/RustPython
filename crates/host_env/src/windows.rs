use rustpython_wtf8::Wtf8;
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
};
use windows_sys::Win32::{
    Foundation::{
        E_POINTER, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FLAGS, ERROR_NO_UNICODE_TRANSLATION,
        MAX_PATH, S_OK,
    },
    Networking::WinSock::WSAStartup,
    Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    },
    System::{
        Diagnostics::Debug::{
            FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
            FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
        },
        LibraryLoader::{GetModuleFileNameW, GetModuleHandleW},
        SystemInformation::{GetVersionExW, OSVERSIONINFOEXW, OSVERSIONINFOW},
        Threading::{GetCurrentThreadStackLimits, SetThreadStackGuarantee},
    },
};

/// _MAX_ENV from Windows CRT stdlib.h - maximum environment variable size
pub const _MAX_ENV: usize = 32767;
pub const HRESULT_E_POINTER: i32 = E_POINTER;
pub const HRESULT_S_OK: i32 = S_OK;
pub const CP_ACP: u32 = windows_sys::Win32::Globalization::CP_ACP;
pub const CP_OEMCP: u32 = windows_sys::Win32::Globalization::CP_OEMCP;
pub const CP_UTF7: u32 = windows_sys::Win32::Globalization::CP_UTF7;
pub const CP_UTF8: u32 = windows_sys::Win32::Globalization::CP_UTF8;
pub const MB_ERR_INVALID_CHARS: u32 = windows_sys::Win32::Globalization::MB_ERR_INVALID_CHARS;
pub const WC_ERR_INVALID_CHARS: u32 = windows_sys::Win32::Globalization::WC_ERR_INVALID_CHARS;
pub const WC_NO_BEST_FIT_CHARS: u32 = windows_sys::Win32::Globalization::WC_NO_BEST_FIT_CHARS;
pub const ERROR_INVALID_FLAGS_I32: i32 = ERROR_INVALID_FLAGS as i32;
pub const ERROR_NO_UNICODE_TRANSLATION_I32: i32 = ERROR_NO_UNICODE_TRANSLATION as i32;
pub const ERROR_INSUFFICIENT_BUFFER_I32: i32 = ERROR_INSUFFICIENT_BUFFER as i32;

pub fn init_winsock() {
    static WSA_INIT: parking_lot::Once = parking_lot::Once::new();
    WSA_INIT.call_once(|| unsafe {
        let mut wsa_data = core::mem::MaybeUninit::uninit();
        let _ = WSAStartup(0x0101, wsa_data.as_mut_ptr());
    })
}

#[derive(Clone, Debug)]
pub struct WindowsVersionInfo {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub platform: u32,
    pub service_pack: String,
    pub service_pack_major: u16,
    pub service_pack_minor: u16,
    pub suite_mask: u16,
    pub product_type: u8,
}

fn get_kernel32_version() -> io::Result<(u32, u32, u32)> {
    unsafe {
        let module_name: Vec<u16> = OsStr::new("kernel32.dll").to_wide_with_nul();
        let h_kernel32 = GetModuleHandleW(module_name.as_ptr());
        if h_kernel32.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut kernel32_path = [0u16; MAX_PATH as usize];
        let len = GetModuleFileNameW(
            h_kernel32,
            kernel32_path.as_mut_ptr(),
            kernel32_path.len() as u32,
        );
        if len == 0 {
            return Err(io::Error::last_os_error());
        }

        let ver_block_size = GetFileVersionInfoSizeW(kernel32_path.as_ptr(), core::ptr::null_mut());
        if ver_block_size == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut ver_block = vec![0u8; ver_block_size as usize];
        if GetFileVersionInfoW(
            kernel32_path.as_ptr(),
            0,
            ver_block_size,
            ver_block.as_mut_ptr() as *mut _,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }

        let sub_block: Vec<u16> = OsStr::new("").to_wide_with_nul();

        let mut ffi_ptr: *mut VS_FIXEDFILEINFO = core::ptr::null_mut();
        let mut ffi_len: u32 = 0;
        if VerQueryValueW(
            ver_block.as_ptr() as *const _,
            sub_block.as_ptr(),
            &mut ffi_ptr as *mut *mut VS_FIXEDFILEINFO as *mut *mut _,
            &mut ffi_len as *mut u32,
        ) == 0
            || ffi_ptr.is_null()
        {
            return Err(io::Error::last_os_error());
        }

        let ffi = *ffi_ptr;
        let real_major = (ffi.dwProductVersionMS >> 16) & 0xFFFF;
        let real_minor = ffi.dwProductVersionMS & 0xFFFF;
        let real_build = (ffi.dwProductVersionLS >> 16) & 0xFFFF;

        Ok((real_major, real_minor, real_build))
    }
}

pub fn get_windows_version() -> io::Result<WindowsVersionInfo> {
    let mut version: OSVERSIONINFOEXW = unsafe { core::mem::zeroed() };
    version.dwOSVersionInfoSize = core::mem::size_of::<OSVERSIONINFOEXW>() as u32;
    let result = unsafe {
        let os_vi = &mut version as *mut OSVERSIONINFOEXW as *mut OSVERSIONINFOW;
        GetVersionExW(os_vi)
    };

    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    let service_pack = {
        let (last, _) = version
            .szCSDVersion
            .iter()
            .take_while(|&x| x != &0)
            .enumerate()
            .last()
            .unwrap_or((0, &0));
        let sp = OsString::from_wide(&version.szCSDVersion[..last]);
        sp.into_string()
            .map_err(|_| io::Error::other("service pack is not ASCII"))?
    };
    let (major, minor, build) = get_kernel32_version()?;
    Ok(WindowsVersionInfo {
        major,
        minor,
        build,
        platform: version.dwPlatformId,
        service_pack,
        service_pack_major: version.wServicePackMajor,
        service_pack_minor: version.wServicePackMinor,
        suite_mask: version.wSuiteMask,
        product_type: version.wProductType,
    })
}

pub fn current_thread_stack_bounds() -> (usize, usize) {
    let mut low: usize = 0;
    let mut high: usize = 0;
    unsafe {
        GetCurrentThreadStackLimits(&mut low as *mut usize, &mut high as *mut usize);
        let mut guarantee: u32 = 0;
        SetThreadStackGuarantee(&mut guarantee);
        low += guarantee as usize;
    }
    (low, high)
}

pub fn set_last_error(error: u32) {
    unsafe { windows_sys::Win32::Foundation::SetLastError(error) }
}

pub fn get_last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

pub fn format_error_message(code: Option<u32>) -> Option<String> {
    let error_code = code.unwrap_or_else(get_last_error);
    let mut buffer: *mut u16 = core::ptr::null_mut();
    let len = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            core::ptr::null(),
            error_code,
            0,
            &mut buffer as *mut *mut u16 as *mut u16,
            0,
            core::ptr::null(),
        )
    };

    if len == 0 || buffer.is_null() {
        return None;
    }

    let message = unsafe {
        let slice = core::slice::from_raw_parts(buffer, len as usize);
        let msg = String::from_utf16_lossy(slice).trim_end().to_string();
        windows_sys::Win32::Foundation::LocalFree(buffer as *mut _);
        msg
    };
    Some(message)
}

pub fn wide_char_to_multi_byte_len(
    code_page: u32,
    flags: u32,
    wide: &[u16],
    track_default_char: bool,
) -> io::Result<(usize, bool)> {
    let mut used_default_char = 0i32;
    let pused = if track_default_char {
        &mut used_default_char as *mut i32
    } else {
        core::ptr::null_mut()
    };
    let size = unsafe {
        windows_sys::Win32::Globalization::WideCharToMultiByte(
            code_page,
            flags,
            wide.as_ptr(),
            wide.len() as i32,
            core::ptr::null_mut(),
            0,
            core::ptr::null(),
            pused,
        )
    };
    if size <= 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((size as usize, used_default_char != 0))
    }
}

pub fn wide_char_to_multi_byte(
    code_page: u32,
    flags: u32,
    wide: &[u16],
    out: &mut [u8],
    track_default_char: bool,
) -> io::Result<(usize, bool)> {
    let mut used_default_char = 0i32;
    let pused = if track_default_char {
        &mut used_default_char as *mut i32
    } else {
        core::ptr::null_mut()
    };
    let size = unsafe {
        windows_sys::Win32::Globalization::WideCharToMultiByte(
            code_page,
            flags,
            wide.as_ptr(),
            wide.len() as i32,
            out.as_mut_ptr().cast(),
            out.len() as i32,
            core::ptr::null(),
            pused,
        )
    };
    if size <= 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((size as usize, used_default_char != 0))
    }
}

pub fn multi_byte_to_wide_len(code_page: u32, flags: u32, bytes: &[u8]) -> io::Result<usize> {
    let size = unsafe {
        windows_sys::Win32::Globalization::MultiByteToWideChar(
            code_page,
            flags,
            bytes.as_ptr().cast(),
            bytes.len() as i32,
            core::ptr::null_mut(),
            0,
        )
    };
    if size <= 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(size as usize)
    }
}

pub fn multi_byte_to_wide(
    code_page: u32,
    flags: u32,
    bytes: &[u8],
    out: &mut [u16],
) -> io::Result<usize> {
    let size = unsafe {
        windows_sys::Win32::Globalization::MultiByteToWideChar(
            code_page,
            flags,
            bytes.as_ptr().cast(),
            bytes.len() as i32,
            out.as_mut_ptr(),
            out.len() as i32,
        )
    };
    if size <= 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(size as usize)
    }
}

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

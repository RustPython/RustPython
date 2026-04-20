#![allow(
    clippy::missing_safety_doc,
    reason = "This module intentionally exposes raw Win32 registry wrappers."
)]
#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "These wrappers mirror Win32 APIs that operate on caller-provided pointers."
)]
#![allow(
    clippy::too_many_arguments,
    reason = "These helpers preserve the underlying Win32 registry call shapes."
)]

extern crate alloc;

use alloc::string::FromUtf16Error;
use std::ffi::OsStr;

use crate::windows::ToWideString;
use windows_sys::Win32::{
    Foundation,
    Security::SECURITY_ATTRIBUTES,
    System::{Environment, Registry},
};

pub type HKEY = Registry::HKEY;
pub const ERROR_MORE_DATA: u32 = Foundation::ERROR_MORE_DATA;
pub const HKEY_CLASSES_ROOT: HKEY = Registry::HKEY_CLASSES_ROOT;
pub const HKEY_CURRENT_USER: HKEY = Registry::HKEY_CURRENT_USER;
pub const HKEY_LOCAL_MACHINE: HKEY = Registry::HKEY_LOCAL_MACHINE;
pub const HKEY_USERS: HKEY = Registry::HKEY_USERS;
pub const HKEY_PERFORMANCE_DATA: HKEY = Registry::HKEY_PERFORMANCE_DATA;
pub const HKEY_CURRENT_CONFIG: HKEY = Registry::HKEY_CURRENT_CONFIG;
pub const HKEY_DYN_DATA: HKEY = Registry::HKEY_DYN_DATA;

pub const KEY_ALL_ACCESS: u32 = Registry::KEY_ALL_ACCESS;
pub const KEY_CREATE_LINK: u32 = Registry::KEY_CREATE_LINK;
pub const KEY_CREATE_SUB_KEY: u32 = Registry::KEY_CREATE_SUB_KEY;
pub const KEY_ENUMERATE_SUB_KEYS: u32 = Registry::KEY_ENUMERATE_SUB_KEYS;
pub const KEY_EXECUTE: u32 = Registry::KEY_EXECUTE;
pub const KEY_NOTIFY: u32 = Registry::KEY_NOTIFY;
pub const KEY_QUERY_VALUE: u32 = Registry::KEY_QUERY_VALUE;
pub const KEY_READ: u32 = Registry::KEY_READ;
pub const KEY_SET_VALUE: u32 = Registry::KEY_SET_VALUE;
pub const KEY_WOW64_32KEY: u32 = Registry::KEY_WOW64_32KEY;
pub const KEY_WOW64_64KEY: u32 = Registry::KEY_WOW64_64KEY;
pub const KEY_WRITE: u32 = Registry::KEY_WRITE;

pub const REG_BINARY: u32 = Registry::REG_BINARY;
pub const REG_CREATED_NEW_KEY: u32 = Registry::REG_CREATED_NEW_KEY;
pub const REG_DWORD: u32 = Registry::REG_DWORD;
pub const REG_DWORD_BIG_ENDIAN: u32 = Registry::REG_DWORD_BIG_ENDIAN;
pub const REG_DWORD_LITTLE_ENDIAN: u32 = Registry::REG_DWORD_LITTLE_ENDIAN;
pub const REG_EXPAND_SZ: u32 = Registry::REG_EXPAND_SZ;
pub const REG_FULL_RESOURCE_DESCRIPTOR: u32 = Registry::REG_FULL_RESOURCE_DESCRIPTOR;
pub const REG_LINK: u32 = Registry::REG_LINK;
pub const REG_MULTI_SZ: u32 = Registry::REG_MULTI_SZ;
pub const REG_NONE: u32 = Registry::REG_NONE;
pub const REG_NOTIFY_CHANGE_ATTRIBUTES: u32 = Registry::REG_NOTIFY_CHANGE_ATTRIBUTES;
pub const REG_NOTIFY_CHANGE_LAST_SET: u32 = Registry::REG_NOTIFY_CHANGE_LAST_SET;
pub const REG_NOTIFY_CHANGE_NAME: u32 = Registry::REG_NOTIFY_CHANGE_NAME;
pub const REG_NOTIFY_CHANGE_SECURITY: u32 = Registry::REG_NOTIFY_CHANGE_SECURITY;
pub const REG_OPENED_EXISTING_KEY: u32 = Registry::REG_OPENED_EXISTING_KEY;
pub const REG_OPTION_BACKUP_RESTORE: u32 = Registry::REG_OPTION_BACKUP_RESTORE;
pub const REG_OPTION_CREATE_LINK: u32 = Registry::REG_OPTION_CREATE_LINK;
pub const REG_OPTION_NON_VOLATILE: u32 = Registry::REG_OPTION_NON_VOLATILE;
pub const REG_OPTION_OPEN_LINK: u32 = Registry::REG_OPTION_OPEN_LINK;
pub const REG_OPTION_RESERVED: u32 = Registry::REG_OPTION_RESERVED;
pub const REG_OPTION_VOLATILE: u32 = Registry::REG_OPTION_VOLATILE;
pub const REG_QWORD: u32 = Registry::REG_QWORD;
pub const REG_QWORD_LITTLE_ENDIAN: u32 = Registry::REG_QWORD_LITTLE_ENDIAN;
pub const REG_RESOURCE_LIST: u32 = Registry::REG_RESOURCE_LIST;
pub const REG_RESOURCE_REQUIREMENTS_LIST: u32 = Registry::REG_RESOURCE_REQUIREMENTS_LIST;
pub const REG_SZ: u32 = Registry::REG_SZ;
pub const REG_WHOLE_HIVE_VOLATILE: u32 = Registry::REG_WHOLE_HIVE_VOLATILE as u32;
pub const REG_REFRESH_HIVE: u32 = 0x00000002;
pub const REG_NO_LAZY_FLUSH: u32 = 0x00000004;
pub const REG_LEGAL_OPTION: u32 = Registry::REG_OPTION_RESERVED
    | Registry::REG_OPTION_NON_VOLATILE
    | Registry::REG_OPTION_VOLATILE
    | Registry::REG_OPTION_CREATE_LINK
    | Registry::REG_OPTION_BACKUP_RESTORE
    | Registry::REG_OPTION_OPEN_LINK;
pub const REG_LEGAL_CHANGE_FILTER: u32 = Registry::REG_NOTIFY_CHANGE_NAME
    | Registry::REG_NOTIFY_CHANGE_ATTRIBUTES
    | Registry::REG_NOTIFY_CHANGE_LAST_SET
    | Registry::REG_NOTIFY_CHANGE_SECURITY;

pub fn bytes_as_wide_slice(bytes: &[u8]) -> &[u16] {
    let (prefix, u16_slice, suffix) = unsafe { bytes.align_to::<u16>() };
    debug_assert!(
        prefix.is_empty() && suffix.is_empty(),
        "Registry data should be u16-aligned"
    );
    u16_slice
}

pub fn close_key(hkey: Registry::HKEY) -> u32 {
    unsafe { Registry::RegCloseKey(hkey) }
}

pub unsafe fn connect_registry(
    computer_name: *const u16,
    key: Registry::HKEY,
    out_key: *mut Registry::HKEY,
) -> u32 {
    unsafe { Registry::RegConnectRegistryW(computer_name, key, out_key) }
}

pub unsafe fn create_key(
    key: Registry::HKEY,
    sub_key: *const u16,
    out_key: *mut Registry::HKEY,
) -> u32 {
    unsafe { Registry::RegCreateKeyW(key, sub_key, out_key) }
}

pub unsafe fn create_key_ex(
    key: Registry::HKEY,
    sub_key: *const u16,
    reserved: u32,
    class: *mut u16,
    options: u32,
    sam: u32,
    security: *const SECURITY_ATTRIBUTES,
    result: *mut Registry::HKEY,
    disposition: *mut u32,
) -> u32 {
    unsafe {
        Registry::RegCreateKeyExW(
            key,
            sub_key,
            reserved,
            class,
            options,
            sam,
            security,
            result,
            disposition,
        )
    }
}

pub unsafe fn delete_key(key: Registry::HKEY, sub_key: *const u16) -> u32 {
    unsafe { Registry::RegDeleteKeyW(key, sub_key) }
}

pub unsafe fn delete_key_ex(
    key: Registry::HKEY,
    sub_key: *const u16,
    sam: u32,
    reserved: u32,
) -> u32 {
    unsafe { Registry::RegDeleteKeyExW(key, sub_key, sam, reserved) }
}

pub unsafe fn delete_value(key: Registry::HKEY, value_name: *const u16) -> u32 {
    unsafe { Registry::RegDeleteValueW(key, value_name) }
}

pub unsafe fn enum_key_ex(
    key: Registry::HKEY,
    index: u32,
    name: *mut u16,
    name_len: *mut u32,
) -> u32 {
    unsafe {
        Registry::RegEnumKeyExW(
            key,
            index,
            name,
            name_len,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    }
}

pub unsafe fn query_info_key(
    key: Registry::HKEY,
    sub_keys: *mut u32,
    values: *mut u32,
    max_value_name_len: *mut u32,
    max_value_len: *mut u32,
) -> u32 {
    unsafe {
        Registry::RegQueryInfoKeyW(
            key,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            sub_keys,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            values,
            max_value_name_len,
            max_value_len,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    }
}

pub struct QueryInfo {
    pub sub_keys: u32,
    pub values: u32,
    pub last_write_time: u64,
}

pub fn query_info_key_full(key: Registry::HKEY) -> Result<QueryInfo, u32> {
    let mut sub_keys = 0;
    let mut values = 0;
    let mut last_write_time: Foundation::FILETIME = unsafe { core::mem::zeroed() };
    let err = unsafe {
        Registry::RegQueryInfoKeyW(
            key,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0 as _,
            &mut sub_keys,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut values,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut last_write_time,
        )
    };
    if err != 0 {
        return Err(err);
    }
    Ok(QueryInfo {
        sub_keys,
        values,
        last_write_time: ((last_write_time.dwHighDateTime as u64) << 32)
            | last_write_time.dwLowDateTime as u64,
    })
}

pub unsafe fn enum_value(
    key: Registry::HKEY,
    index: u32,
    value_name: *mut u16,
    value_name_len: *mut u32,
    value_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    unsafe {
        Registry::RegEnumValueW(
            key,
            index,
            value_name,
            value_name_len,
            core::ptr::null_mut(),
            value_type,
            data,
            data_len,
        )
    }
}

pub fn flush_key(key: Registry::HKEY) -> u32 {
    unsafe { Registry::RegFlushKey(key) }
}

pub unsafe fn load_key(key: Registry::HKEY, sub_key: *const u16, file_name: *const u16) -> u32 {
    unsafe { Registry::RegLoadKeyW(key, sub_key, file_name) }
}

pub unsafe fn open_key_ex(
    key: Registry::HKEY,
    sub_key: *const u16,
    options: u32,
    sam: u32,
    out_key: *mut Registry::HKEY,
) -> u32 {
    unsafe { Registry::RegOpenKeyExW(key, sub_key, options, sam, out_key) }
}

pub unsafe fn query_value_ex(
    key: Registry::HKEY,
    value_name: *const u16,
    value_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    unsafe {
        Registry::RegQueryValueExW(
            key,
            value_name,
            core::ptr::null_mut(),
            value_type,
            data,
            data_len,
        )
    }
}

pub unsafe fn save_key(key: Registry::HKEY, file_name: *const u16) -> u32 {
    unsafe { Registry::RegSaveKeyW(key, file_name, core::ptr::null_mut()) }
}

pub unsafe fn set_value_ex(
    key: Registry::HKEY,
    value_name: *const u16,
    typ: u32,
    ptr: *const u8,
    len: u32,
) -> u32 {
    unsafe { Registry::RegSetValueExW(key, value_name, 0, typ, ptr, len) }
}

pub fn disable_reflection_key(key: Registry::HKEY) -> u32 {
    unsafe { Registry::RegDisableReflectionKey(key) }
}

pub fn enable_reflection_key(key: Registry::HKEY) -> u32 {
    unsafe { Registry::RegEnableReflectionKey(key) }
}

pub unsafe fn query_reflection_key(key: Registry::HKEY, result: *mut i32) -> u32 {
    unsafe { Registry::RegQueryReflectionKey(key, result) }
}

pub enum ExpandEnvironmentStringsError {
    Os,
    Utf16(FromUtf16Error),
}

pub enum QueryStringError {
    Code(u32),
    Utf16(FromUtf16Error),
}

pub fn query_default_value(
    hkey: Registry::HKEY,
    sub_key: Option<&OsStr>,
) -> Result<String, QueryStringError> {
    let child_key = if let Some(sub_key) = sub_key.filter(|s| !s.is_empty()) {
        let wide_sub_key = sub_key.to_wide_with_nul();
        let mut out_key = core::ptr::null_mut();
        let res = unsafe {
            open_key_ex(
                hkey,
                wide_sub_key.as_ptr(),
                0,
                Registry::KEY_QUERY_VALUE,
                &mut out_key,
            )
        };
        if res != 0 {
            return Err(QueryStringError::Code(res));
        }
        Some(out_key)
    } else {
        None
    };

    let target_key = child_key.unwrap_or(hkey);
    let mut buf_size: u32 = 256;
    let mut buffer: Vec<u8> = vec![0; buf_size as usize];
    let mut reg_type: u32 = 0;

    let result = loop {
        let mut size = buf_size;
        let res = unsafe {
            query_value_ex(
                target_key,
                core::ptr::null(),
                &mut reg_type,
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        if res == Foundation::ERROR_MORE_DATA {
            buf_size *= 2;
            buffer.resize(buf_size as usize, 0);
            continue;
        }
        if res == Foundation::ERROR_FILE_NOT_FOUND {
            break Ok(String::new());
        }
        if res != 0 {
            break Err(QueryStringError::Code(res));
        }
        if reg_type != Registry::REG_SZ {
            break Err(QueryStringError::Code(Foundation::ERROR_INVALID_DATA));
        }

        let u16_slice = bytes_as_wide_slice(&buffer[..size as usize]);
        let len = u16_slice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(u16_slice.len());
        break String::from_utf16(&u16_slice[..len]).map_err(QueryStringError::Utf16);
    };

    if let Some(ck) = child_key {
        close_key(ck);
    }

    result
}

pub fn query_value_bytes(hkey: Registry::HKEY, value_name: &OsStr) -> Result<(Vec<u8>, u32), u32> {
    let wide_name = value_name.to_wide_with_nul();
    let mut buf_size: u32 = 0;
    let res = unsafe {
        query_value_ex(
            hkey,
            wide_name.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut buf_size,
        )
    };
    if res == Foundation::ERROR_MORE_DATA || buf_size == 0 {
        buf_size = 256;
    } else if res != 0 {
        return Err(res);
    }

    let mut ret_buf = vec![0u8; buf_size as usize];
    let mut typ = 0;

    loop {
        let mut ret_size = buf_size;
        let res = unsafe {
            query_value_ex(
                hkey,
                wide_name.as_ptr(),
                &mut typ,
                ret_buf.as_mut_ptr(),
                &mut ret_size,
            )
        };
        if res != Foundation::ERROR_MORE_DATA {
            if res != 0 {
                return Err(res);
            }
            ret_buf.truncate(ret_size as usize);
            return Ok((ret_buf, typ));
        }
        buf_size *= 2;
        ret_buf.resize(buf_size as usize, 0);
    }
}

pub fn set_default_value(hkey: Registry::HKEY, sub_key: &OsStr, typ: u32, value: &OsStr) -> u32 {
    let child_key = if !sub_key.is_empty() {
        let wide_sub_key = sub_key.to_wide_with_nul();
        let mut out_key = core::ptr::null_mut();
        let res = unsafe {
            create_key_ex(
                hkey,
                wide_sub_key.as_ptr(),
                0,
                core::ptr::null_mut(),
                0,
                Registry::KEY_SET_VALUE,
                core::ptr::null(),
                &mut out_key,
                core::ptr::null_mut(),
            )
        };
        if res != 0 {
            return res;
        }
        Some(out_key)
    } else {
        None
    };

    let target_key = child_key.unwrap_or(hkey);
    let wide_value = value.to_wide_with_nul();
    let res = unsafe {
        set_value_ex(
            target_key,
            core::ptr::null(),
            typ,
            wide_value.as_ptr() as *const u8,
            (wide_value.len() * 2) as u32,
        )
    };

    if let Some(ck) = child_key {
        close_key(ck);
    }
    res
}

pub fn expand_environment_strings(input: &OsStr) -> Result<String, ExpandEnvironmentStringsError> {
    let wide_input = input.to_wide_with_nul();
    let required_size = unsafe {
        Environment::ExpandEnvironmentStringsW(wide_input.as_ptr(), core::ptr::null_mut(), 0)
    };
    if required_size == 0 {
        return Err(ExpandEnvironmentStringsError::Os);
    }

    let mut out = vec![0u16; required_size as usize];
    let written = unsafe {
        Environment::ExpandEnvironmentStringsW(wide_input.as_ptr(), out.as_mut_ptr(), required_size)
    };
    if written == 0 {
        return Err(ExpandEnvironmentStringsError::Os);
    }

    let len = out.iter().position(|&c| c == 0).unwrap_or(out.len());
    String::from_utf16(&out[..len]).map_err(ExpandEnvironmentStringsError::Utf16)
}

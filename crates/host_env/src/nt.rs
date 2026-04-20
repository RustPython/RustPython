// cspell:ignore hchmod
use std::{ffi::OsStr, io, os::windows::io::AsRawHandle};

use crate::{crt_fd, windows::ToWideString};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO, FileBasicInfo, GetFileAttributesW,
        GetFileInformationByHandleEx, INVALID_FILE_ATTRIBUTES, SetFileAttributesW,
        SetFileInformationByHandle,
    },
};

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn win32_hchmod(handle: HANDLE, mode: u32, write_bit: u32) -> io::Result<()> {
    let mut info: FILE_BASIC_INFO = unsafe { core::mem::zeroed() };
    let ret = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut info as *mut FILE_BASIC_INFO).cast(),
            core::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    if mode & write_bit != 0 {
        info.FileAttributes &= !FILE_ATTRIBUTE_READONLY;
    } else {
        info.FileAttributes |= FILE_ATTRIBUTE_READONLY;
    }

    let ret = unsafe {
        SetFileInformationByHandle(
            handle,
            FileBasicInfo,
            (&info as *const FILE_BASIC_INFO).cast(),
            core::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn fchmod(fd: i32, mode: u32, write_bit: u32) -> io::Result<()> {
    let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
    let handle = crt_fd::as_handle(borrowed)?;
    win32_hchmod(handle.as_raw_handle() as HANDLE, mode, write_bit)
}

pub fn win32_lchmod(path: &OsStr, mode: u32, write_bit: u32) -> io::Result<()> {
    let wide = path.to_wide_with_nul();
    let attr = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attr == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    let new_attr = if mode & write_bit != 0 {
        attr & !FILE_ATTRIBUTE_READONLY
    } else {
        attr | FILE_ATTRIBUTE_READONLY
    };
    let ret = unsafe { SetFileAttributesW(wide.as_ptr(), new_attr) };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "This module mirrors raw Win32 path, handle, and CRT entry points."
)]

// cspell:ignore hchmod
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::Path,
};

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    crt_fd,
    fileutils::{
        StatStruct,
        windows::{FILE_INFO_BY_NAME_CLASS, get_file_information_by_name, stat_basic_info_to_stat},
    },
    windows::ToWideString,
};
use libc::intptr_t;
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, MAX_PATH},
    Globalization::{CP_UTF8, MultiByteToWideChar, WideCharToMultiByte},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_READONLY, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_TYPE_UNKNOWN, FileBasicInfo,
        FindClose, FindFirstFileW, GetFileAttributesW, GetFileInformationByHandleEx, GetFileType,
        GetFullPathNameW, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING, SetFileAttributesW,
        SetFileInformationByHandle, WIN32_FIND_DATAW,
    },
    System::{Console, Threading},
};

#[cfg(target_env = "msvc")]
unsafe extern "C" {
    fn _cwait(termstat: *mut i32, procHandle: intptr_t, action: i32) -> intptr_t;
    fn _wexecv(cmdname: *const u16, argv: *const *const u16) -> intptr_t;
    fn _wexecve(cmdname: *const u16, argv: *const *const u16, envp: *const *const u16) -> intptr_t;
    fn _wspawnv(mode: i32, cmdname: *const u16, argv: *const *const u16) -> intptr_t;
    fn _wspawnve(
        mode: i32,
        cmdname: *const u16,
        argv: *const *const u16,
        envp: *const *const u16,
    ) -> intptr_t;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TestType {
    RegularFile,
    Directory,
    Symlink,
    Junction,
    LinkReparsePoint,
    RegularReparsePoint,
}

const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C;
const S_IFMT: u16 = libc::S_IFMT as u16;
const S_IFDIR_MODE: u16 = libc::S_IFDIR as u16;
const S_IFCHR_MODE: u16 = libc::S_IFCHR as u16;
const S_IFIFO_MODE: u16 = crate::fileutils::windows::S_IFIFO as u16;

#[repr(C)]
#[derive(Default)]
struct FileAttributeTagInfo {
    file_attributes: u32,
    reparse_tag: u32,
}

fn win32_large_integer_to_time(li: i64) -> (libc::time_t, i32) {
    let nsec = ((li % 10_000_000) * 100) as i32;
    let sec = (li / 10_000_000 - crate::fileutils::windows::SECS_BETWEEN_EPOCHS) as libc::time_t;
    (sec, nsec)
}

fn win32_filetime_to_time(ft_low: u32, ft_high: u32) -> (libc::time_t, i32) {
    let ticks = ((ft_high as i64) << 32) | (ft_low as i64);
    let nsec = ((ticks % 10_000_000) * 100) as i32;
    let sec = (ticks / 10_000_000 - crate::fileutils::windows::SECS_BETWEEN_EPOCHS) as libc::time_t;
    (sec, nsec)
}

fn win32_attribute_data_to_stat(
    info: &windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
    reparse_tag: u32,
    basic_info: Option<&windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO>,
    id_info: Option<&windows_sys::Win32::Storage::FileSystem::FILE_ID_INFO>,
) -> StatStruct {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let mut st_mode: u16 = 0;
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        st_mode |= S_IFDIR_MODE | 0o111;
    } else {
        st_mode |= libc::S_IFREG as u16;
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0 {
        st_mode |= 0o444;
    } else {
        st_mode |= 0o666;
    }

    let st_size = ((info.nFileSizeHigh as u64) << 32) | (info.nFileSizeLow as u64);
    let st_dev = id_info
        .map(|id| id.VolumeSerialNumber as u32)
        .unwrap_or(info.dwVolumeSerialNumber);
    let st_nlink = info.nNumberOfLinks as i32;

    let (st_birthtime, st_birthtime_nsec, st_mtime, st_mtime_nsec, st_atime, st_atime_nsec) =
        if let Some(bi) = basic_info {
            let (birth, birth_nsec) = win32_large_integer_to_time(bi.CreationTime);
            let (mtime, mtime_nsec) = win32_large_integer_to_time(bi.LastWriteTime);
            let (atime, atime_nsec) = win32_large_integer_to_time(bi.LastAccessTime);
            (birth, birth_nsec, mtime, mtime_nsec, atime, atime_nsec)
        } else {
            let (birth, birth_nsec) = win32_filetime_to_time(
                info.ftCreationTime.dwLowDateTime,
                info.ftCreationTime.dwHighDateTime,
            );
            let (mtime, mtime_nsec) = win32_filetime_to_time(
                info.ftLastWriteTime.dwLowDateTime,
                info.ftLastWriteTime.dwHighDateTime,
            );
            let (atime, atime_nsec) = win32_filetime_to_time(
                info.ftLastAccessTime.dwLowDateTime,
                info.ftLastAccessTime.dwHighDateTime,
            );
            (birth, birth_nsec, mtime, mtime_nsec, atime, atime_nsec)
        };

    let (st_ino, st_ino_high) = if let Some(id) = id_info {
        let bytes = id.FileId.Identifier;
        (
            u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        )
    } else {
        (
            ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64),
            0,
        )
    };

    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && reparse_tag == IO_REPARSE_TAG_SYMLINK
    {
        st_mode = (st_mode & !S_IFMT) | crate::fileutils::windows::S_IFLNK as u16;
    }

    StatStruct {
        st_dev,
        st_ino,
        st_ino_high,
        st_mode,
        st_nlink,
        st_uid: 0,
        st_gid: 0,
        st_rdev: 0,
        st_size,
        st_atime,
        st_atime_nsec,
        st_mtime,
        st_mtime_nsec,
        st_ctime: 0,
        st_ctime_nsec: 0,
        st_birthtime,
        st_birthtime_nsec,
        st_file_attributes: info.dwFileAttributes,
        st_reparse_tag: reparse_tag,
    }
}

pub fn visible_env_vars() -> impl Iterator<Item = (String, String)> {
    crate::os::vars().filter(|(key, _)| !key.starts_with('='))
}

#[derive(Debug)]
pub enum ReadlinkError {
    Io(io::Error),
    NotSymbolicLink,
    InvalidReparseData,
}

#[derive(Debug)]
pub enum ReadConsoleError {
    Io(io::Error),
    BufferTooSmall { available: usize, required: usize },
}

pub fn access(path: &Path, mode: u8) -> bool {
    let wide = path.as_os_str().to_wide_with_nul();
    let attr = unsafe { GetFileAttributesW(wide.as_ptr()) };
    attr != INVALID_FILE_ATTRIBUTES
        && (mode & 2 == 0
            || attr & FILE_ATTRIBUTE_READONLY == 0
            || attr & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0)
}

pub fn remove(path: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        DeleteFileW, RemoveDirectoryW, WIN32_FIND_DATAW,
    };
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };

    let wide_path = path.as_os_str().to_wide_with_nul();
    let attrs = unsafe { GetFileAttributesW(wide_path.as_ptr()) };

    let mut is_directory = false;
    let mut is_link = false;

    if attrs != INVALID_FILE_ATTRIBUTES {
        is_directory =
            (attrs & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY) != 0;

        if is_directory
            && (attrs & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT) != 0
        {
            let mut find_data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };
            let handle = unsafe { FindFirstFileW(wide_path.as_ptr(), &mut find_data) };
            if handle != INVALID_HANDLE_VALUE {
                is_link = find_data.dwReserved0 == IO_REPARSE_TAG_SYMLINK
                    || find_data.dwReserved0 == IO_REPARSE_TAG_MOUNT_POINT;
                unsafe { FindClose(handle) };
            }
        }
    }

    let ok = if is_directory && is_link {
        unsafe { RemoveDirectoryW(wide_path.as_ptr()) }
    } else {
        unsafe { DeleteFileW(wide_path.as_ptr()) }
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn supports_virtual_terminal() -> bool {
    let mut mode = 0;
    let handle = unsafe { Console::GetStdHandle(Console::STD_ERROR_HANDLE) };
    (unsafe { Console::GetConsoleMode(handle, &mut mode) }) != 0
        && mode & Console::ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
}

pub fn symlink(
    src: &Path,
    dst: &Path,
    src_wide: &widestring::WideCStr,
    dst_wide: &widestring::WideCStr,
    target_is_directory: bool,
) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::WIN32_FILE_ATTRIBUTE_DATA;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateSymbolicLinkW, FILE_ATTRIBUTE_DIRECTORY, GetFileAttributesExW,
        SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, SYMBOLIC_LINK_FLAG_DIRECTORY,
    };

    static HAS_UNPRIVILEGED_FLAG: AtomicBool = AtomicBool::new(true);

    fn check_dir(src: &Path, dst: &Path) -> bool {
        use windows_sys::Win32::Storage::FileSystem::GetFileExInfoStandard;

        let Some(dst_parent) = dst.parent() else {
            return false;
        };
        let resolved = if src.is_absolute() {
            src.to_path_buf()
        } else {
            dst_parent.join(src)
        };
        let wide = match widestring::WideCString::from_os_str(&resolved) {
            Ok(wide) => wide,
            Err(_) => return false,
        };
        let mut info: WIN32_FILE_ATTRIBUTE_DATA = unsafe { core::mem::zeroed() };
        let ok = unsafe {
            GetFileAttributesExW(
                wide.as_ptr(),
                GetFileExInfoStandard,
                (&mut info as *mut WIN32_FILE_ATTRIBUTE_DATA).cast(),
            )
        };
        ok != 0 && (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0
    }

    let mut flags = 0u32;
    if HAS_UNPRIVILEGED_FLAG.load(Ordering::Relaxed) {
        flags |= SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
    }
    if target_is_directory || check_dir(src, dst) {
        flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
    }

    let mut result = unsafe { CreateSymbolicLinkW(dst_wide.as_ptr(), src_wide.as_ptr(), flags) };
    if !result
        && HAS_UNPRIVILEGED_FLAG.load(Ordering::Relaxed)
        && unsafe { windows_sys::Win32::Foundation::GetLastError() }
            == windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER
    {
        let flags = flags & !SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
        result = unsafe { CreateSymbolicLinkW(dst_wide.as_ptr(), src_wide.as_ptr(), flags) };
        if result
            || unsafe { windows_sys::Win32::Foundation::GetLastError() }
                != windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER
        {
            HAS_UNPRIVILEGED_FLAG.store(false, Ordering::Relaxed);
        }
    }

    if result {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

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

pub fn chmod_follow(path: &widestring::WideCStr, mode: u32, write_bit: u32) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
    };

    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let result = win32_hchmod(handle, mode, write_bit);
    unsafe { CloseHandle(handle) };
    result
}

pub fn find_first_file_name(path: &Path) -> io::Result<OsString> {
    let wide_path = path.as_os_str().to_wide_with_nul();
    let mut find_data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };

    let handle = unsafe { FindFirstFileW(wide_path.as_ptr(), &mut find_data) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    unsafe { FindClose(handle) };

    let len = find_data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(find_data.cFileName.len());
    Ok(OsString::from_wide(&find_data.cFileName[..len]))
}

pub fn path_isdevdrive(path: &Path) -> io::Result<bool> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetVolumePathNameW,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_QUERY_PERSISTENT_VOLUME_STATE;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

    const PERSISTENT_VOLUME_STATE_DEV_VOLUME: u32 = 0x0000_2000;

    #[repr(C)]
    struct FileFsPersistentVolumeInformation {
        volume_flags: u32,
        flag_mask: u32,
        version: u32,
        reserved: u32,
    }

    let wide_path = path.as_os_str().to_wide_with_nul();
    let mut volume = [0u16; MAX_PATH as usize];
    let ok =
        unsafe { GetVolumePathNameW(wide_path.as_ptr(), volume.as_mut_ptr(), volume.len() as _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { GetDriveTypeW(volume.as_ptr()) } != DRIVE_FIXED {
        return Ok(false);
    }

    let handle = unsafe {
        CreateFileW(
            volume.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut volume_state = FileFsPersistentVolumeInformation {
        volume_flags: 0,
        flag_mask: PERSISTENT_VOLUME_STATE_DEV_VOLUME,
        version: 1,
        reserved: 0,
    };
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
            (&volume_state as *const FileFsPersistentVolumeInformation).cast(),
            core::mem::size_of::<FileFsPersistentVolumeInformation>() as u32,
            (&mut volume_state as *mut FileFsPersistentVolumeInformation).cast(),
            core::mem::size_of::<FileFsPersistentVolumeInformation>() as u32,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(handle) };

    if ok == 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error()
            == Some(windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER as i32)
        {
            return Ok(false);
        }
        return Err(err);
    }

    Ok((volume_state.volume_flags & PERSISTENT_VOLUME_STATE_DEV_VOLUME) != 0)
}

pub fn is_reparse_tag_name_surrogate(tag: u32) -> bool {
    (tag & 0x20000000) != 0
}

pub fn file_info_error_is_trustworthy(error: u32) -> bool {
    use windows_sys::Win32::Foundation;
    matches!(
        error,
        Foundation::ERROR_FILE_NOT_FOUND
            | Foundation::ERROR_PATH_NOT_FOUND
            | Foundation::ERROR_NOT_READY
            | Foundation::ERROR_BAD_NET_NAME
            | Foundation::ERROR_BAD_NETPATH
            | Foundation::ERROR_BAD_PATHNAME
            | Foundation::ERROR_INVALID_NAME
            | Foundation::ERROR_FILENAME_EXCED_RANGE
    )
}

pub fn test_info(
    attributes: u32,
    reparse_tag: u32,
    disk_device: bool,
    tested_type: TestType,
) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };

    match tested_type {
        TestType::RegularFile => {
            disk_device && attributes != 0 && (attributes & FILE_ATTRIBUTE_DIRECTORY) == 0
        }
        TestType::Directory => (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0,
        TestType::Symlink => {
            (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                && reparse_tag == IO_REPARSE_TAG_SYMLINK
        }
        TestType::Junction => {
            (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                && reparse_tag == IO_REPARSE_TAG_MOUNT_POINT
        }
        TestType::LinkReparsePoint => {
            (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                && is_reparse_tag_name_surrogate(reparse_tag)
        }
        TestType::RegularReparsePoint => {
            (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                && reparse_tag != 0
                && !is_reparse_tag_name_surrogate(reparse_tag)
        }
    }
}

pub fn test_file_type_by_handle(handle: HANDLE, tested_type: TestType, disk_only: bool) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TAG_INFO, FILE_TYPE_DISK, FileAttributeTagInfo as FileAttributeTagInfoClass,
    };

    let disk_device = unsafe { GetFileType(handle) } == FILE_TYPE_DISK;
    if disk_only && !disk_device {
        return false;
    }

    if tested_type != TestType::RegularFile && tested_type != TestType::Directory {
        let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfoClass,
                (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                core::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ret == 0 {
            return false;
        }
        test_info(
            info.FileAttributes,
            info.ReparseTag,
            disk_device,
            tested_type,
        )
    } else {
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
            return false;
        }
        test_info(info.FileAttributes, 0, disk_device, tested_type)
    }
}

fn win32_xstat_attributes_from_dir(
    path: &OsStr,
) -> io::Result<(
    windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
    u32,
)> {
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let wide: Vec<u16> = path.to_wide_with_nul();
    let mut find_data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };

    let handle = unsafe { FindFirstFileW(wide.as_ptr(), &mut find_data) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    unsafe { FindClose(handle) };

    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { core::mem::zeroed() };
    info.dwFileAttributes = find_data.dwFileAttributes;
    info.ftCreationTime = find_data.ftCreationTime;
    info.ftLastAccessTime = find_data.ftLastAccessTime;
    info.ftLastWriteTime = find_data.ftLastWriteTime;
    info.nFileSizeHigh = find_data.nFileSizeHigh;
    info.nFileSizeLow = find_data.nFileSizeLow;

    let reparse_tag = if find_data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        find_data.dwReserved0
    } else {
        0
    };

    Ok((info, reparse_tag))
}

fn win32_xstat_slow_impl(path: &OsStr, traverse: bool) -> io::Result<StatStruct> {
    use windows_sys::Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_CANT_ACCESS_FILE, ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_SHARING_VIOLATION, GENERIC_READ,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FILE_TYPE_CHAR, FILE_TYPE_PIPE,
            FileAttributeTagInfo as FileAttributeTagInfoClass, FileBasicInfo, FileIdInfo,
            GetFileAttributesW, GetFileInformationByHandle,
        },
    };

    let wide: Vec<u16> = path.to_wide_with_nul();
    let access = FILE_READ_ATTRIBUTES;
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !traverse {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }

    let mut h_file = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            flags,
            core::ptr::null_mut(),
        )
    };

    let mut file_info: BY_HANDLE_FILE_INFORMATION = unsafe { core::mem::zeroed() };
    let mut tag_info = FileAttributeTagInfo::default();
    let mut is_unhandled_tag = false;

    if h_file == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        match error.raw_os_error().unwrap_or(0) as u32 {
            ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION => {
                let (info, reparse_tag) = win32_xstat_attributes_from_dir(path)?;
                file_info = info;
                tag_info.reparse_tag = reparse_tag;

                if file_info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                    && (traverse || !is_reparse_tag_name_surrogate(tag_info.reparse_tag))
                {
                    return Err(error);
                }
            }
            ERROR_INVALID_PARAMETER => {
                h_file = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        access | GENERIC_READ,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        core::ptr::null(),
                        OPEN_EXISTING,
                        flags,
                        core::ptr::null_mut(),
                    )
                };
                if h_file == INVALID_HANDLE_VALUE {
                    return Err(error);
                }
            }
            ERROR_CANT_ACCESS_FILE if traverse => {
                is_unhandled_tag = true;
                h_file = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        access,
                        0,
                        core::ptr::null(),
                        OPEN_EXISTING,
                        flags | FILE_FLAG_OPEN_REPARSE_POINT,
                        core::ptr::null_mut(),
                    )
                };
                if h_file == INVALID_HANDLE_VALUE {
                    return Err(error);
                }
            }
            _ => return Err(error),
        }
    }

    let result = (|| -> io::Result<StatStruct> {
        if h_file != INVALID_HANDLE_VALUE {
            let file_type = unsafe { GetFileType(h_file) };
            if file_type != windows_sys::Win32::Storage::FileSystem::FILE_TYPE_DISK {
                if file_type == FILE_TYPE_UNKNOWN {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error().unwrap_or(0) != 0 {
                        return Err(err);
                    }
                }
                let file_attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
                let mut st_mode = 0;
                if file_attributes != INVALID_FILE_ATTRIBUTES
                    && file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                {
                    st_mode = S_IFDIR_MODE;
                } else if file_type == FILE_TYPE_CHAR {
                    st_mode = S_IFCHR_MODE;
                } else if file_type == FILE_TYPE_PIPE {
                    st_mode = S_IFIFO_MODE;
                }
                return Ok(StatStruct {
                    st_mode,
                    ..Default::default()
                });
            }

            if !traverse || is_unhandled_tag {
                let mut local_tag_info: FileAttributeTagInfo = unsafe { core::mem::zeroed() };
                let ret = unsafe {
                    GetFileInformationByHandleEx(
                        h_file,
                        FileAttributeTagInfoClass,
                        (&mut local_tag_info as *mut FileAttributeTagInfo).cast(),
                        core::mem::size_of::<FileAttributeTagInfo>() as u32,
                    )
                };
                if ret == 0 {
                    match io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32 {
                        ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => {
                            local_tag_info.file_attributes = FILE_ATTRIBUTE_NORMAL;
                            local_tag_info.reparse_tag = 0;
                        }
                        _ => return Err(io::Error::last_os_error()),
                    }
                } else if local_tag_info.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    if is_reparse_tag_name_surrogate(local_tag_info.reparse_tag) {
                        if is_unhandled_tag {
                            return Err(io::Error::from_raw_os_error(
                                ERROR_CANT_ACCESS_FILE as i32,
                            ));
                        }
                    } else if !is_unhandled_tag {
                        unsafe { CloseHandle(h_file) };
                        h_file = INVALID_HANDLE_VALUE;
                        return win32_xstat_slow_impl(path, true);
                    }
                }
                tag_info = local_tag_info;
            }

            if unsafe { GetFileInformationByHandle(h_file, &mut file_info) } == 0 {
                match io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32 {
                    ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => {
                        return Ok(StatStruct {
                            st_mode: 0x6000,
                            ..Default::default()
                        });
                    }
                    _ => return Err(io::Error::last_os_error()),
                }
            }

            let mut basic_info: FILE_BASIC_INFO = unsafe { core::mem::zeroed() };
            let has_basic_info = unsafe {
                GetFileInformationByHandleEx(
                    h_file,
                    FileBasicInfo,
                    (&mut basic_info as *mut FILE_BASIC_INFO).cast(),
                    core::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            } != 0;

            let mut id_info: FILE_ID_INFO = unsafe { core::mem::zeroed() };
            let has_id_info = unsafe {
                GetFileInformationByHandleEx(
                    h_file,
                    FileIdInfo,
                    (&mut id_info as *mut FILE_ID_INFO).cast(),
                    core::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            } != 0;

            let mut result = win32_attribute_data_to_stat(
                &file_info,
                tag_info.reparse_tag,
                if has_basic_info {
                    Some(&basic_info)
                } else {
                    None
                },
                if has_id_info { Some(&id_info) } else { None },
            );
            result.update_st_mode_from_path(path, file_info.dwFileAttributes);
            Ok(result)
        } else {
            let mut result =
                win32_attribute_data_to_stat(&file_info, tag_info.reparse_tag, None, None);
            result.update_st_mode_from_path(path, file_info.dwFileAttributes);
            Ok(result)
        }
    })();

    if h_file != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(h_file) };
    }
    result
}

pub fn win32_xstat(path: &OsStr, traverse: bool) -> io::Result<StatStruct> {
    use windows_sys::Win32::{Foundation, Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT};

    match get_file_information_by_name(path, FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo) {
        Ok(stat_info) => {
            if (stat_info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0)
                || (!traverse && is_reparse_tag_name_surrogate(stat_info.ReparseTag))
            {
                let mut result = stat_basic_info_to_stat(&stat_info);
                if result.st_ino != 0 || result.st_ino_high != 0 {
                    result.update_st_mode_from_path(path, stat_info.FileAttributes);
                    result.st_ctime = result.st_birthtime;
                    result.st_ctime_nsec = result.st_birthtime_nsec;
                    return Ok(result);
                }
            }
        }
        Err(err) => {
            if let Some(errno) = err.raw_os_error()
                && matches!(
                    errno as u32,
                    Foundation::ERROR_FILE_NOT_FOUND
                        | Foundation::ERROR_PATH_NOT_FOUND
                        | Foundation::ERROR_NOT_READY
                        | Foundation::ERROR_BAD_NET_NAME
                )
            {
                return Err(err);
            }
        }
    }

    let mut result = win32_xstat_slow_impl(path, traverse)?;
    result.st_ctime = result.st_birthtime;
    result.st_ctime_nsec = result.st_birthtime_nsec;
    Ok(result)
}

pub fn test_file_type_by_name(path: &Path, tested_type: TestType) -> bool {
    match get_file_information_by_name(
        path.as_os_str(),
        FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo,
    ) {
        Ok(info) => {
            let disk_device = matches!(
                info.DeviceType,
                windows_sys::Win32::Storage::FileSystem::FILE_DEVICE_DISK
                    | windows_sys::Win32::System::Ioctl::FILE_DEVICE_VIRTUAL_DISK
                    | windows_sys::Win32::Storage::FileSystem::FILE_DEVICE_CD_ROM
            );
            let result = test_info(
                info.FileAttributes,
                info.ReparseTag,
                disk_device,
                tested_type,
            );
            if !result
                || !matches!(tested_type, TestType::RegularFile | TestType::Directory)
                || (info.FileAttributes
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT)
                    == 0
            {
                return result;
            }
        }
        Err(err) => {
            if let Some(code) = err.raw_os_error()
                && file_info_error_is_trustworthy(code as u32)
            {
                return false;
            }
        }
    }

    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !matches!(tested_type, TestType::RegularFile | TestType::Directory) {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let wide_path = path.as_os_str().to_wide_with_nul();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            flags,
            core::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        let result = test_file_type_by_handle(handle, tested_type, false);
        unsafe { CloseHandle(handle) };
        return result;
    }

    match unsafe { GetLastError() } {
        windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
        | windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION
        | windows_sys::Win32::Foundation::ERROR_CANT_ACCESS_FILE
        | windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER => {
            let stat = win32_xstat(
                path.as_os_str(),
                matches!(tested_type, TestType::RegularFile | TestType::Directory),
            );
            if let Ok(st) = stat {
                let disk_device = (st.st_mode & libc::S_IFREG as u16) != 0;
                return test_info(
                    st.st_file_attributes,
                    st.st_reparse_tag,
                    disk_device,
                    tested_type,
                );
            }
        }
        _ => {}
    }

    false
}

pub fn test_file_exists_by_name(path: &Path, follow_links: bool) -> bool {
    match get_file_information_by_name(
        path.as_os_str(),
        FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo,
    ) {
        Ok(info) => {
            if (info.FileAttributes
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT)
                == 0
                || (!follow_links && is_reparse_tag_name_surrogate(info.ReparseTag))
            {
                return true;
            }
        }
        Err(err) => {
            if let Some(code) = err.raw_os_error()
                && file_info_error_is_trustworthy(code as u32)
            {
                return false;
            }
        }
    }

    let wide_path = path.as_os_str().to_wide_with_nul();
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow_links {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            flags,
            core::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        if follow_links {
            unsafe { CloseHandle(handle) };
            return true;
        }
        let is_regular_reparse_point =
            test_file_type_by_handle(handle, TestType::RegularReparsePoint, false);
        unsafe { CloseHandle(handle) };
        if !is_regular_reparse_point {
            return true;
        }
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                FILE_READ_ATTRIBUTES,
                0,
                core::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                core::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
            return true;
        }
    }

    match unsafe { GetLastError() } {
        windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
        | windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION
        | windows_sys::Win32::Foundation::ERROR_CANT_ACCESS_FILE
        | windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER => {
            return win32_xstat(path.as_os_str(), follow_links).is_ok();
        }
        _ => {}
    }

    false
}

pub fn path_exists_via_open(path: &Path, follow_links: bool) -> bool {
    let wide_path = path.as_os_str().to_wide_with_nul();
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow_links {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            flags,
            core::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        if follow_links {
            unsafe { CloseHandle(handle) };
            return true;
        }
        let is_regular_reparse_point =
            test_file_type_by_handle(handle, TestType::RegularReparsePoint, false);
        unsafe { CloseHandle(handle) };
        if !is_regular_reparse_point {
            return true;
        }
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                FILE_READ_ATTRIBUTES,
                0,
                core::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                core::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
            return true;
        }
    }
    false
}

pub fn fd_exists(fd: crate::crt_fd::Borrowed<'_>) -> bool {
    let handle = match crate::crt_fd::as_handle(fd) {
        Ok(handle) => handle,
        Err(_) => return false,
    };
    let file_type = unsafe { GetFileType(handle.as_raw_handle() as _) };
    if file_type != FILE_TYPE_UNKNOWN {
        true
    } else {
        unsafe { GetLastError() == 0 }
    }
}

pub fn pipe() -> io::Result<(i32, i32)> {
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    let mut attr = SECURITY_ATTRIBUTES {
        nLength: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: core::ptr::null_mut(),
        bInheritHandle: 0,
    };

    let (read_handle, write_handle) = unsafe {
        let mut read = core::mem::MaybeUninit::<isize>::uninit();
        let mut write = core::mem::MaybeUninit::<isize>::uninit();
        let ok = CreatePipe(
            read.as_mut_ptr() as *mut _,
            write.as_mut_ptr() as *mut _,
            &mut attr as *mut _,
            0,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        (read.assume_init(), write.assume_init())
    };

    const O_NOINHERIT: i32 = 0x80;
    let read_fd = match crate::msvcrt::open_osfhandle(read_handle, O_NOINHERIT) {
        Ok(fd) => fd,
        Err(err) => {
            unsafe {
                CloseHandle(read_handle as _);
                CloseHandle(write_handle as _);
            }
            return Err(err);
        }
    };
    let write_fd = match crate::msvcrt::open_osfhandle(write_handle, libc::O_WRONLY | O_NOINHERIT) {
        Ok(fd) => fd,
        Err(err) => {
            let _ = unsafe { crt_fd::Owned::from_raw(read_fd) };
            unsafe { CloseHandle(write_handle as _) };
            return Err(err);
        }
    };

    Ok((read_fd, write_fd))
}

pub fn mkdir(path: &widestring::WideCStr, mode: i32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let ok = if mode == 0o700 {
        let mut sec_attr = SECURITY_ATTRIBUTES {
            nLength: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: core::ptr::null_mut(),
            bInheritHandle: 0,
        };
        let sddl: Vec<u16> = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)\0"
            .encode_utf16()
            .collect();
        let convert_ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sec_attr.lpSecurityDescriptor,
                core::ptr::null_mut(),
            )
        };
        if convert_ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(
                path.as_ptr(),
                (&sec_attr as *const SECURITY_ATTRIBUTES).cast(),
            )
        };
        unsafe { LocalFree(sec_attr.lpSecurityDescriptor) };
        ok
    } else {
        unsafe {
            windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(
                path.as_ptr(),
                core::ptr::null_mut(),
            )
        }
    };

    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

unsafe extern "C" {
    fn _umask(mask: i32) -> i32;
}

pub fn umask(mask: i32) -> io::Result<i32> {
    let result = unsafe { _umask(mask) };
    if result < 0 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(result)
    }
}

fn set_fd_inheritable(fd: i32, inheritable: bool) -> io::Result<()> {
    let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
    let handle = crt_fd::as_handle(borrowed)?;
    set_handle_inheritable(handle.as_raw_handle() as _, inheritable)
}

pub fn dup(fd: i32) -> io::Result<i32> {
    let fd2 = unsafe { crate::suppress_iph!(libc::dup(fd)) };
    if fd2 < 0 {
        return Err(crate::os::errno_io_error());
    }
    if let Err(err) = set_fd_inheritable(fd2, false) {
        let _ = unsafe { crt_fd::Owned::from_raw(fd2) };
        return Err(err);
    }
    Ok(fd2)
}

pub fn dup2(fd: i32, fd2: i32, inheritable: bool) -> io::Result<i32> {
    let result = unsafe { crate::suppress_iph!(libc::dup2(fd, fd2)) };
    if result < 0 {
        return Err(crate::os::errno_io_error());
    }
    if !inheritable && let Err(err) = set_fd_inheritable(fd2, false) {
        let _ = unsafe { crt_fd::Owned::from_raw(fd2) };
        return Err(err);
    }
    Ok(fd2)
}

pub fn readlink(path: &Path) -> Result<OsString, ReadlinkError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };

    let wide_path = path.as_os_str().to_wide_with_nul();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            core::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(ReadlinkError::Io(io::Error::last_os_error()));
    }

    const BUFFER_SIZE: usize = 16384;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut bytes_returned: u32 = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            core::ptr::null(),
            0,
            buffer.as_mut_ptr() as *mut _,
            BUFFER_SIZE as u32,
            &mut bytes_returned,
            core::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(ReadlinkError::Io(io::Error::last_os_error()));
    }

    let reparse_tag = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    let (substitute_offset, substitute_length, path_buffer_start) =
        if reparse_tag == IO_REPARSE_TAG_SYMLINK {
            (
                u16::from_le_bytes([buffer[8], buffer[9]]) as usize,
                u16::from_le_bytes([buffer[10], buffer[11]]) as usize,
                20usize,
            )
        } else if reparse_tag == IO_REPARSE_TAG_MOUNT_POINT {
            (
                u16::from_le_bytes([buffer[8], buffer[9]]) as usize,
                u16::from_le_bytes([buffer[10], buffer[11]]) as usize,
                16usize,
            )
        } else {
            return Err(ReadlinkError::NotSymbolicLink);
        };

    let path_start = path_buffer_start + substitute_offset;
    let path_end = path_start + substitute_length;
    if path_end > buffer.len() {
        return Err(ReadlinkError::InvalidReparseData);
    }

    let path_slice = &buffer[path_start..path_end];
    let mut wide_chars: Vec<u16> = path_slice
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    if wide_chars.len() > 4
        && wide_chars[0] == b'\\' as u16
        && wide_chars[1] == b'?' as u16
        && wide_chars[2] == b'?' as u16
        && wide_chars[3] == b'\\' as u16
    {
        wide_chars[1] = b'\\' as u16;
    }

    Ok(OsString::from_wide(&wide_chars))
}

pub fn kill(pid: u32, sig: u32) -> io::Result<()> {
    if sig == Console::CTRL_C_EVENT || sig == Console::CTRL_BREAK_EVENT {
        let ok = unsafe { Console::GenerateConsoleCtrlEvent(sig, pid) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    } else {
        let handle = unsafe { Threading::OpenProcess(Threading::PROCESS_ALL_ACCESS, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let ok = unsafe { Threading::TerminateProcess(handle, sig) };
        let err = if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        unsafe { CloseHandle(handle) };
        err
    }
}

pub fn getfinalpathname(path: &Path) -> io::Result<OsString> {
    use windows_sys::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, VOLUME_NAME_DOS};

    let wide = path.as_os_str().to_wide_with_nul();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0u16; MAX_PATH as usize];
    let result = loop {
        let ret = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_DOS,
            )
        };
        if ret == 0 {
            break Err(io::Error::last_os_error());
        }
        if ret as usize >= buffer.len() {
            buffer.resize(ret as usize, 0);
            continue;
        }
        break Ok(OsString::from_wide(&buffer[..ret as usize]));
    };
    unsafe { CloseHandle(handle) };
    result
}

pub fn getfullpathname(path: &Path) -> io::Result<OsString> {
    let wide = path.as_os_str().to_wide_with_nul();
    let mut buffer = vec![0u16; MAX_PATH as usize];
    let mut ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFullPathNameW(
            wide.as_ptr(),
            buffer.len() as u32,
            buffer.as_mut_ptr(),
            core::ptr::null_mut(),
        )
    };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }
    if ret as usize > buffer.len() {
        buffer.resize(ret as usize, 0);
        ret = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFullPathNameW(
                wide.as_ptr(),
                buffer.len() as u32,
                buffer.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        };
        if ret == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(widestring::WideCString::from_vec_truncate(buffer).to_os_string())
}

pub fn getvolumepathname(path: &Path) -> io::Result<OsString> {
    let wide = path.as_os_str().to_wide_with_nul();
    let buflen = core::cmp::max(wide.len(), MAX_PATH as usize);
    let mut buffer = vec![0u16; buflen];
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW(
            wide.as_ptr(),
            buffer.as_mut_ptr(),
            buflen as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(widestring::WideCString::from_vec_truncate(buffer).to_os_string())
    }
}

pub fn getdiskusage(path: &Path) -> io::Result<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = path.as_os_str().to_wide_with_nul();
    let mut free_to_me = 0u64;
    let mut total = 0u64;
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_me, &mut total, &mut free) };
    if ok != 0 {
        return Ok((total, free));
    }

    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_DIRECTORY as i32)
        && let Some(parent) = path.parent()
    {
        let parent = widestring::WideCString::from_os_str(parent).unwrap();
        let ok =
            unsafe { GetDiskFreeSpaceExW(parent.as_ptr(), &mut free_to_me, &mut total, &mut free) };
        if ok != 0 {
            return Ok((total, free));
        }
    }
    Err(err)
}

pub fn get_handle_inheritable(handle: intptr_t) -> io::Result<bool> {
    let mut flags = 0;
    let ok =
        unsafe { windows_sys::Win32::Foundation::GetHandleInformation(handle as _, &mut flags) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(flags & windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT != 0)
    }
}

pub fn set_handle_inheritable(handle: intptr_t, inheritable: bool) -> io::Result<()> {
    let flags = if inheritable {
        windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT
    } else {
        0
    };
    let ok = unsafe {
        windows_sys::Win32::Foundation::SetHandleInformation(
            handle as _,
            windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT,
            flags,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn getlogin() -> io::Result<String> {
    let mut buffer = [0u16; 257];
    let mut size = buffer.len() as u32;
    let ok = unsafe {
        windows_sys::Win32::System::WindowsProgramming::GetUserNameW(buffer.as_mut_ptr(), &mut size)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OsString::from_wide(&buffer[..(size - 1) as usize])
        .to_str()
        .unwrap()
        .to_string())
}

pub fn listdrives() -> io::Result<Vec<OsString>> {
    let mut buffer = [0u16; 256];
    let len = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetLogicalDriveStringsW(
            buffer.len() as u32,
            buffer.as_mut_ptr(),
        )
    };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    if len as usize >= buffer.len() {
        return Err(io::Error::from_raw_os_error(
            windows_sys::Win32::Foundation::ERROR_MORE_DATA as i32,
        ));
    }
    Ok(buffer[..(len - 1) as usize]
        .split(|&c| c == 0)
        .map(OsString::from_wide)
        .collect())
}

pub fn listvolumes() -> io::Result<Vec<OsString>> {
    let mut result = Vec::new();
    let mut buffer = [0u16; MAX_PATH as usize + 1];

    let find = unsafe {
        windows_sys::Win32::Storage::FileSystem::FindFirstVolumeW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    };
    if find == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    loop {
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        result.push(OsString::from_wide(&buffer[..len]));

        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::FindNextVolumeW(
                find,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            unsafe { windows_sys::Win32::Storage::FileSystem::FindVolumeClose(find) };
            if err.raw_os_error()
                == Some(windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES as i32)
            {
                break;
            }
            return Err(err);
        }
    }

    Ok(result)
}

pub fn listmounts(volume: &Path) -> io::Result<Vec<OsString>> {
    let wide = volume.as_os_str().to_wide_with_nul();
    let mut buflen: u32 = MAX_PATH + 1;
    let mut buffer = vec![0u16; buflen as usize];

    loop {
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumePathNamesForVolumeNameW(
                wide.as_ptr(),
                buffer.as_mut_ptr(),
                buflen,
                &mut buflen,
            )
        };
        if ok != 0 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_MORE_DATA as i32) {
            buffer.resize(buflen as usize, 0);
            continue;
        }
        return Err(err);
    }

    let mut result = Vec::new();
    let mut start = 0;
    for (i, &c) in buffer.iter().enumerate() {
        if c == 0 {
            if i > start {
                result.push(OsString::from_wide(&buffer[start..i]));
            }
            start = i + 1;
            if start < buffer.len() && buffer[start] == 0 {
                break;
            }
        }
    }
    Ok(result)
}

pub fn getppid() -> u32 {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, PROCESS_BASIC_INFORMATION};

    type NtQueryInformationProcessFn = unsafe extern "system" fn(
        process_handle: isize,
        process_information_class: u32,
        process_information: *mut core::ffi::c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;

    let ntdll = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(windows_sys::w!("ntdll.dll"))
    };
    if ntdll.is_null() {
        return 0;
    }

    let func = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            ntdll,
            c"NtQueryInformationProcess".as_ptr() as *const u8,
        )
    };
    let Some(func) = func else {
        return 0;
    };
    let nt_query: NtQueryInformationProcessFn = unsafe { core::mem::transmute(func) };

    let mut info: PROCESS_BASIC_INFORMATION = unsafe { core::mem::zeroed() };
    let status = unsafe {
        nt_query(
            GetCurrentProcess() as isize,
            0,
            (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
            core::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            core::ptr::null_mut(),
        )
    };

    if status >= 0
        && info.InheritedFromUniqueProcessId != 0
        && info.InheritedFromUniqueProcessId < u32::MAX as usize
    {
        info.InheritedFromUniqueProcessId as u32
    } else {
        0
    }
}

pub fn path_skip_root(path: *const u16) -> Option<usize> {
    let mut end: *const u16 = core::ptr::null();
    let hr = unsafe { windows_sys::Win32::UI::Shell::PathCchSkipRoot(path, &mut end) };
    if hr >= 0 {
        assert!(!end.is_null());
        Some(
            unsafe { end.offset_from(path) }
                .try_into()
                .expect("len must be non-negative"),
        )
    } else {
        None
    }
}

pub fn get_terminal_size_handle(h: HANDLE) -> io::Result<(usize, usize)> {
    let mut csbi = core::mem::MaybeUninit::uninit();
    let ret = unsafe { Console::GetConsoleScreenBufferInfo(h, csbi.as_mut_ptr()) };
    if ret == 0 {
        let err = unsafe { GetLastError() };
        if err != windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED {
            return Err(io::Error::last_os_error());
        }
        let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let console_handle = unsafe {
            CreateFileW(
                conout.as_ptr(),
                windows_sys::Win32::Foundation::GENERIC_READ
                    | windows_sys::Win32::Foundation::GENERIC_WRITE,
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
                core::ptr::null(),
                windows_sys::Win32::Storage::FileSystem::OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
        if console_handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let ret = unsafe { Console::GetConsoleScreenBufferInfo(console_handle, csbi.as_mut_ptr()) };
        unsafe { CloseHandle(console_handle) };
        if ret == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let csbi = unsafe { csbi.assume_init() };
    let window = csbi.srWindow;
    let columns = (window.Right - window.Left + 1) as usize;
    let lines = (window.Bottom - window.Top + 1) as usize;
    Ok((columns, lines))
}

fn handle_from_fd(fd: i32) -> HANDLE {
    unsafe { crate::suppress_iph!(libc::get_osfhandle(fd)) as HANDLE }
}

pub fn console_type(handle: HANDLE) -> char {
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return '\0';
    }
    let mut mode: u32 = 0;
    if unsafe { Console::GetConsoleMode(handle, &mut mode) } == 0 {
        return '\0';
    }
    let mut peek_count: u32 = 0;
    if unsafe { Console::GetNumberOfConsoleInputEvents(handle, &mut peek_count) } != 0 {
        'r'
    } else {
        'w'
    }
}

pub fn console_type_from_fd(fd: i32) -> char {
    if fd < 0 {
        '\0'
    } else {
        console_type(handle_from_fd(fd))
    }
}

pub fn console_type_from_name(name: &str) -> char {
    if name.eq_ignore_ascii_case("CONIN$") {
        return 'r';
    }
    if name.eq_ignore_ascii_case("CONOUT$") {
        return 'w';
    }
    if name.eq_ignore_ascii_case("CON") {
        return 'x';
    }

    let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let mut buf = [0u16; MAX_PATH as usize];
    let length = unsafe {
        GetFullPathNameW(
            wide.as_ptr(),
            buf.len() as u32,
            buf.as_mut_ptr(),
            core::ptr::null_mut(),
        )
    };
    if length == 0 || length as usize > buf.len() {
        return '\0';
    }

    let full_path = &buf[..length as usize];
    let path_part = if full_path.len() >= 4
        && full_path[0] == b'\\' as u16
        && full_path[1] == b'\\' as u16
        && (full_path[2] == b'.' as u16 || full_path[2] == b'?' as u16)
        && full_path[3] == b'\\' as u16
    {
        &full_path[4..]
    } else {
        full_path
    };

    let path_str = String::from_utf16_lossy(path_part);
    if path_str.eq_ignore_ascii_case("CONIN$") {
        'r'
    } else if path_str.eq_ignore_ascii_case("CONOUT$") {
        'w'
    } else if path_str.eq_ignore_ascii_case("CON") {
        'x'
    } else {
        '\0'
    }
}

fn copy_from_small_buf(buf: &mut [u8; 4], dest: &mut [u8]) -> usize {
    let mut n = 0;
    while buf[0] != 0 && n < dest.len() {
        dest[n] = buf[0];
        n += 1;
        for i in 1..buf.len() {
            buf[i - 1] = buf[i];
        }
        buf[buf.len() - 1] = 0;
    }
    n
}

fn find_last_utf8_boundary(buf: &[u8], len: usize) -> usize {
    let len = len.min(buf.len());
    for count in 1..=4.min(len) {
        let c = buf[len - count];
        if c < 0x80 {
            return len;
        }
        if c >= 0xc0 {
            let expected = if c < 0xe0 {
                2
            } else if c < 0xf0 {
                3
            } else {
                4
            };
            if count < expected {
                return len - count;
            }
            return len;
        }
    }
    len
}

fn wchar_to_utf8_count(data: &[u8], mut len: usize, mut n: u32) -> usize {
    let mut start: usize = 0;
    loop {
        let mut mid = 0;
        for i in (len / 2)..=len {
            mid = find_last_utf8_boundary(data, i);
            if mid != 0 {
                break;
            }
        }
        if mid == len {
            return start + len;
        }
        if mid == 0 {
            mid = if len > 1 { len - 1 } else { 1 };
        }
        let wlen = unsafe {
            MultiByteToWideChar(
                CP_UTF8,
                0,
                data[start..].as_ptr(),
                mid as i32,
                core::ptr::null_mut(),
                0,
            )
        } as u32;
        if wlen <= n {
            start += mid;
            len -= mid;
            n -= wlen;
        } else {
            len = mid;
        }
    }
}

pub fn read_console_into(
    handle: HANDLE,
    dest: &mut [u8],
    smallbuf: &mut [u8; 4],
) -> Result<usize, ReadConsoleError> {
    if dest.is_empty() {
        return Ok(0);
    }

    let mut wlen = (dest.len() / 4) as u32;
    if wlen == 0 {
        wlen = 1;
    }

    let mut read_len = copy_from_small_buf(smallbuf, dest);
    if read_len > 0 {
        wlen = wlen.saturating_sub(1);
    }
    if read_len >= dest.len() || wlen == 0 {
        return Ok(read_len);
    }

    let mut wbuf = vec![0u16; wlen as usize];
    let mut nread: u32 = 0;
    if unsafe {
        Console::ReadConsoleW(
            handle,
            wbuf.as_mut_ptr().cast(),
            wlen,
            &mut nread,
            core::ptr::null(),
        )
    } == 0
    {
        return Err(ReadConsoleError::Io(io::Error::last_os_error()));
    }
    if nread == 0 || wbuf[0] == 0x1A {
        return Ok(read_len);
    }

    let remaining = dest.len() - read_len;
    let u8n = if remaining < 4 {
        let converted = unsafe {
            WideCharToMultiByte(
                CP_UTF8,
                0,
                wbuf.as_ptr(),
                nread as i32,
                smallbuf.as_mut_ptr().cast(),
                smallbuf.len() as i32,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };
        if converted > 0 {
            copy_from_small_buf(smallbuf, &mut dest[read_len..]) as i32
        } else {
            0
        }
    } else {
        unsafe {
            WideCharToMultiByte(
                CP_UTF8,
                0,
                wbuf.as_ptr(),
                nread as i32,
                dest[read_len..].as_mut_ptr().cast(),
                remaining as i32,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        }
    };

    if u8n > 0 {
        read_len += u8n as usize;
        return Ok(read_len);
    }

    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER as i32)
    {
        let needed = unsafe {
            WideCharToMultiByte(
                CP_UTF8,
                0,
                wbuf.as_ptr(),
                nread as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };
        if needed > 0 {
            return Err(ReadConsoleError::BufferTooSmall {
                available: remaining,
                required: needed as usize,
            });
        }
    }
    Err(ReadConsoleError::Io(err))
}

pub fn read_console_all(handle: HANDLE, smallbuf: &mut [u8; 4]) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut tmp = [0u8; 4];
    let n = copy_from_small_buf(smallbuf, &mut tmp);
    result.extend_from_slice(&tmp[..n]);

    let mut wbuf = vec![0u16; 8192];
    loop {
        let mut nread: u32 = 0;
        if unsafe {
            Console::ReadConsoleW(
                handle,
                wbuf.as_mut_ptr().cast(),
                wbuf.len() as u32,
                &mut nread,
                core::ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if nread == 0 || wbuf[0] == 0x1A {
            break;
        }

        let needed = unsafe {
            WideCharToMultiByte(
                CP_UTF8,
                0,
                wbuf.as_ptr(),
                nread as i32,
                core::ptr::null_mut(),
                0,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let offset = result.len();
        result.resize(offset + needed as usize, 0);
        if unsafe {
            WideCharToMultiByte(
                CP_UTF8,
                0,
                wbuf.as_ptr(),
                nread as i32,
                result[offset..].as_mut_ptr().cast(),
                needed,
                core::ptr::null(),
                core::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if nread < wbuf.len() as u32 {
            break;
        }
    }

    Ok(result)
}

pub fn write_console_utf8(handle: HANDLE, data: &[u8], max_bytes: usize) -> io::Result<usize> {
    if data.is_empty() {
        return Ok(0);
    }

    let mut len = data.len().min(max_bytes);
    let max_wlen: u32 = 32766 / 2;
    len = len.min(max_wlen as usize * 3);

    let wlen = loop {
        len = find_last_utf8_boundary(data, len);
        let wlen = unsafe {
            MultiByteToWideChar(
                CP_UTF8,
                0,
                data.as_ptr(),
                len as i32,
                core::ptr::null_mut(),
                0,
            )
        };
        if wlen as u32 <= max_wlen {
            break wlen;
        }
        len /= 2;
    };
    if wlen == 0 {
        return Ok(0);
    }

    let mut wbuf = vec![0u16; wlen as usize];
    let wlen = unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            0,
            data.as_ptr(),
            len as i32,
            wbuf.as_mut_ptr(),
            wlen,
        )
    };
    if wlen == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut written: u32 = 0;
    if unsafe {
        Console::WriteConsoleW(
            handle,
            wbuf.as_ptr().cast(),
            wlen as u32,
            &mut written,
            core::ptr::null(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    if written < wlen as u32 {
        len = wchar_to_utf8_count(data, len, written);
    }
    Ok(len)
}

pub fn open_console_path_fd(path: *const u16, writable: bool) -> io::Result<i32> {
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
    };

    let access = if writable {
        GENERIC_WRITE
    } else {
        GENERIC_READ
    };

    let mut handle = unsafe {
        CreateFileW(
            path,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        handle = unsafe {
            CreateFileW(
                path,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                core::ptr::null(),
                OPEN_EXISTING,
                0,
                core::ptr::null_mut(),
            )
        };
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let osf_flags = if writable {
        libc::O_WRONLY | libc::O_BINARY | 0x80
    } else {
        libc::O_RDONLY | libc::O_BINARY | 0x80
    };
    match crate::msvcrt::open_osfhandle(handle as isize, osf_flags) {
        Ok(fd) => Ok(fd),
        Err(err) => {
            unsafe { CloseHandle(handle) };
            Err(err)
        }
    }
}

#[cfg(target_env = "msvc")]
pub fn cwait(pid: intptr_t, opt: i32) -> io::Result<(intptr_t, i32)> {
    let mut status = 0;
    let pid = unsafe { crate::suppress_iph!(_cwait(&mut status, pid, opt)) };
    if pid == -1 {
        Err(crate::os::errno_io_error())
    } else {
        Ok((pid, status))
    }
}

#[cfg(target_env = "msvc")]
pub fn spawnv(mode: i32, path: *const u16, argv: *const *const u16) -> io::Result<intptr_t> {
    let result = unsafe { crate::suppress_iph!(_wspawnv(mode, path, argv)) };
    if result == -1 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(result)
    }
}

#[cfg(target_env = "msvc")]
pub fn spawnve(
    mode: i32,
    path: *const u16,
    argv: *const *const u16,
    envp: *const *const u16,
) -> io::Result<intptr_t> {
    let result = unsafe { crate::suppress_iph!(_wspawnve(mode, path, argv, envp)) };
    if result == -1 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(result)
    }
}

#[cfg(target_env = "msvc")]
pub fn execv(path: *const u16, argv: *const *const u16) -> io::Result<()> {
    let result = unsafe { crate::suppress_iph!(_wexecv(path, argv)) };
    if result == -1 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

#[cfg(target_env = "msvc")]
pub fn execve(
    path: *const u16,
    argv: *const *const u16,
    envp: *const *const u16,
) -> io::Result<()> {
    let result = unsafe { crate::suppress_iph!(_wexecve(path, argv, envp)) };
    if result == -1 {
        Err(crate::os::errno_io_error())
    } else {
        Ok(())
    }
}

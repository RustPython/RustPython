use crate::common::fileutils::{
    StatStruct,
    windows::{FILE_INFO_BY_NAME_CLASS, get_file_information_by_name},
};
use crate::{
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    convert::{ToPyObject, ToPyResult},
    stdlib::os::errno_err,
};
use std::ffi::OsStr;
use windows::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::{BOOL, HANDLE as RAW_HANDLE, INVALID_HANDLE_VALUE};

pub(crate) trait WindowsSysResultValue {
    type Ok: ToPyObject;
    fn is_err(&self) -> bool;
    fn into_ok(self) -> Self::Ok;
}

impl WindowsSysResultValue for RAW_HANDLE {
    type Ok = HANDLE;
    fn is_err(&self) -> bool {
        *self == INVALID_HANDLE_VALUE
    }
    fn into_ok(self) -> Self::Ok {
        HANDLE(self as _)
    }
}

impl WindowsSysResultValue for BOOL {
    type Ok = ();
    fn is_err(&self) -> bool {
        *self == 0
    }
    fn into_ok(self) -> Self::Ok {}
}

pub(crate) struct WindowsSysResult<T>(pub T);

impl<T: WindowsSysResultValue> WindowsSysResult<T> {
    pub fn is_err(&self) -> bool {
        self.0.is_err()
    }
    pub fn into_pyresult(self, vm: &VirtualMachine) -> PyResult<T::Ok> {
        if self.is_err() {
            Err(errno_err(vm))
        } else {
            Ok(self.0.into_ok())
        }
    }
}

impl<T: WindowsSysResultValue> ToPyResult for WindowsSysResult<T> {
    fn to_pyresult(self, vm: &VirtualMachine) -> PyResult {
        let ok = self.into_pyresult(vm)?;
        Ok(ok.to_pyobject(vm))
    }
}

type HandleInt = usize; // TODO: change to isize when fully ported to windows-rs

impl TryFromObject for HANDLE {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let handle = HandleInt::try_from_object(vm, obj)?;
        Ok(HANDLE(handle as isize))
    }
}

impl ToPyObject for HANDLE {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        (self.0 as HandleInt).to_pyobject(vm)
    }
}

pub fn init_winsock() {
    static WSA_INIT: parking_lot::Once = parking_lot::Once::new();
    WSA_INIT.call_once(|| unsafe {
        let mut wsa_data = std::mem::MaybeUninit::uninit();
        let _ = windows_sys::Win32::Networking::WinSock::WSAStartup(0x0101, wsa_data.as_mut_ptr());
    })
}

// win32_xstat in cpython
pub fn win32_xstat(path: &OsStr, traverse: bool) -> std::io::Result<StatStruct> {
    let mut result = win32_xstat_impl(path, traverse)?;
    // ctime is only deprecated from 3.12, so we copy birthtime across
    result.st_ctime = result.st_birthtime;
    result.st_ctime_nsec = result.st_birthtime_nsec;
    Ok(result)
}

fn is_reparse_tag_name_surrogate(tag: u32) -> bool {
    (tag & 0x20000000) > 0
}

// Constants
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C;
const S_IFMT: u16 = libc::S_IFMT as u16;
const S_IFDIR: u16 = libc::S_IFDIR as u16;
const S_IFREG: u16 = libc::S_IFREG as u16;
const S_IFCHR: u16 = libc::S_IFCHR as u16;
const S_IFLNK: u16 = crate::common::fileutils::windows::S_IFLNK as u16;
const S_IFIFO: u16 = crate::common::fileutils::windows::S_IFIFO as u16;

/// FILE_ATTRIBUTE_TAG_INFO structure for GetFileInformationByHandleEx
#[repr(C)]
#[derive(Default)]
struct FileAttributeTagInfo {
    file_attributes: u32,
    reparse_tag: u32,
}

/// Ported from attributes_to_mode (fileutils.c)
fn attributes_to_mode(attr: u32) -> u16 {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    };
    let mut m: u16 = 0;
    if attr & FILE_ATTRIBUTE_DIRECTORY != 0 {
        m |= S_IFDIR | 0o111; // IFEXEC for user,group,other
    } else {
        m |= S_IFREG;
    }
    if attr & FILE_ATTRIBUTE_READONLY != 0 {
        m |= 0o444;
    } else {
        m |= 0o666;
    }
    m
}

/// Ported from _Py_attribute_data_to_stat (fileutils.c)
/// Converts BY_HANDLE_FILE_INFORMATION to StatStruct
fn attribute_data_to_stat(
    info: &windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
    reparse_tag: u32,
    basic_info: Option<&windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO>,
    id_info: Option<&windows_sys::Win32::Storage::FileSystem::FILE_ID_INFO>,
) -> StatStruct {
    use crate::common::fileutils::windows::SECS_BETWEEN_EPOCHS;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let mut st_mode = attributes_to_mode(info.dwFileAttributes);
    let st_size = ((info.nFileSizeHigh as u64) << 32) | (info.nFileSizeLow as u64);
    let st_dev = id_info
        .map(|id| id.VolumeSerialNumber as u32)
        .unwrap_or(info.dwVolumeSerialNumber);
    let st_nlink = info.nNumberOfLinks as i32;

    // Convert FILETIME/LARGE_INTEGER to (time_t, nsec)
    let filetime_to_time = |ft_low: u32, ft_high: u32| -> (libc::time_t, i32) {
        let ticks = ((ft_high as i64) << 32) | (ft_low as i64);
        let nsec = ((ticks % 10_000_000) * 100) as i32;
        let sec = (ticks / 10_000_000 - SECS_BETWEEN_EPOCHS) as libc::time_t;
        (sec, nsec)
    };

    let large_integer_to_time = |li: i64| -> (libc::time_t, i32) {
        let nsec = ((li % 10_000_000) * 100) as i32;
        let sec = (li / 10_000_000 - SECS_BETWEEN_EPOCHS) as libc::time_t;
        (sec, nsec)
    };

    let (st_birthtime, st_birthtime_nsec);
    let (st_mtime, st_mtime_nsec);
    let (st_atime, st_atime_nsec);

    if let Some(bi) = basic_info {
        (st_birthtime, st_birthtime_nsec) = large_integer_to_time(bi.CreationTime);
        (st_mtime, st_mtime_nsec) = large_integer_to_time(bi.LastWriteTime);
        (st_atime, st_atime_nsec) = large_integer_to_time(bi.LastAccessTime);
    } else {
        (st_birthtime, st_birthtime_nsec) = filetime_to_time(
            info.ftCreationTime.dwLowDateTime,
            info.ftCreationTime.dwHighDateTime,
        );
        (st_mtime, st_mtime_nsec) = filetime_to_time(
            info.ftLastWriteTime.dwLowDateTime,
            info.ftLastWriteTime.dwHighDateTime,
        );
        (st_atime, st_atime_nsec) = filetime_to_time(
            info.ftLastAccessTime.dwLowDateTime,
            info.ftLastAccessTime.dwHighDateTime,
        );
    }

    // Get file ID from id_info or fallback to file index
    let (st_ino, st_ino_high) = if let Some(id) = id_info {
        // FILE_ID_INFO.FileId is FILE_ID_128 which is [u8; 16]
        let bytes = id.FileId.Identifier;
        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        (low, high)
    } else {
        let ino = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        (ino, 0u64)
    };

    // Set symlink mode if applicable
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && reparse_tag == IO_REPARSE_TAG_SYMLINK
    {
        st_mode = (st_mode & !S_IFMT) | S_IFLNK;
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
        st_ctime: 0, // Will be set by caller
        st_ctime_nsec: 0,
        st_birthtime,
        st_birthtime_nsec,
        st_file_attributes: info.dwFileAttributes,
        st_reparse_tag: reparse_tag,
    }
}

/// Get file info using FindFirstFileW (fallback when CreateFileW fails)
/// Ported from attributes_from_dir
fn attributes_from_dir(
    path: &OsStr,
) -> std::io::Result<(
    windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION,
    u32,
)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindFirstFileW,
        WIN32_FIND_DATAW,
    };

    let wide: Vec<u16> = path.encode_wide().chain(std::iter::once(0)).collect();
    let mut find_data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };

    let handle = unsafe { FindFirstFileW(wide.as_ptr(), &mut find_data) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    unsafe { FindClose(handle) };

    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
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

/// Ported from win32_xstat_slow_impl
fn win32_xstat_slow_impl(path: &OsStr, traverse: bool) -> std::io::Result<StatStruct> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANT_ACCESS_FILE, ERROR_INVALID_FUNCTION,
            ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_SHARING_VIOLATION, GENERIC_READ,
            INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
            FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_CHAR,
            FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_UNKNOWN, FileAttributeTagInfo, FileBasicInfo,
            FileIdInfo, GetFileAttributesW, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFileType, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
        },
    };

    let wide: Vec<u16> = path.encode_wide().chain(std::iter::once(0)).collect();

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
            std::ptr::null(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };

    let mut file_info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let mut tag_info = FileAttributeTagInfo::default();
    let mut is_unhandled_tag = false;

    if h_file == INVALID_HANDLE_VALUE {
        let error = std::io::Error::last_os_error();
        let error_code = error.raw_os_error().unwrap_or(0) as u32;

        match error_code {
            ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION => {
                // Try reading the parent directory using FindFirstFileW
                let (info, reparse_tag) = attributes_from_dir(path)?;
                file_info = info;
                tag_info.reparse_tag = reparse_tag;

                if file_info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                    && (traverse || !is_reparse_tag_name_surrogate(tag_info.reparse_tag))
                {
                    return Err(error);
                }
                // h_file remains INVALID_HANDLE_VALUE, we'll use file_info from FindFirstFileW
            }
            ERROR_INVALID_PARAMETER => {
                // Retry with GENERIC_READ (needed for \\.\con)
                h_file = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        access | GENERIC_READ,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        flags,
                        std::ptr::null_mut(),
                    )
                };
                if h_file == INVALID_HANDLE_VALUE {
                    return Err(error);
                }
            }
            ERROR_CANT_ACCESS_FILE if traverse => {
                // bpo37834: open unhandled reparse points if traverse fails
                is_unhandled_tag = true;
                h_file = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        access,
                        0,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        flags | FILE_FLAG_OPEN_REPARSE_POINT,
                        std::ptr::null_mut(),
                    )
                };
                if h_file == INVALID_HANDLE_VALUE {
                    return Err(error);
                }
            }
            _ => return Err(error),
        }
    }

    // Scope for handle cleanup
    let result = (|| -> std::io::Result<StatStruct> {
        if h_file != INVALID_HANDLE_VALUE {
            // Handle types other than files on disk
            let file_type = unsafe { GetFileType(h_file) };
            if file_type != FILE_TYPE_DISK {
                if file_type == FILE_TYPE_UNKNOWN {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error().unwrap_or(0) != 0 {
                        return Err(err);
                    }
                }
                let file_attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
                let mut st_mode: u16 = 0;
                if file_attributes != INVALID_FILE_ATTRIBUTES
                    && file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                {
                    st_mode = S_IFDIR;
                } else if file_type == FILE_TYPE_CHAR {
                    st_mode = S_IFCHR;
                } else if file_type == FILE_TYPE_PIPE {
                    st_mode = S_IFIFO;
                }
                return Ok(StatStruct {
                    st_mode,
                    ..Default::default()
                });
            }

            // Query the reparse tag
            if !traverse || is_unhandled_tag {
                let mut local_tag_info: FileAttributeTagInfo = unsafe { std::mem::zeroed() };
                let ret = unsafe {
                    GetFileInformationByHandleEx(
                        h_file,
                        FileAttributeTagInfo,
                        &mut local_tag_info as *mut _ as *mut _,
                        std::mem::size_of::<FileAttributeTagInfo>() as u32,
                    )
                };
                if ret == 0 {
                    let err_code =
                        std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
                    match err_code {
                        ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => {
                            local_tag_info.file_attributes = FILE_ATTRIBUTE_NORMAL;
                            local_tag_info.reparse_tag = 0;
                        }
                        _ => return Err(std::io::Error::last_os_error()),
                    }
                } else if local_tag_info.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    if is_reparse_tag_name_surrogate(local_tag_info.reparse_tag) {
                        if is_unhandled_tag {
                            return Err(std::io::Error::from_raw_os_error(
                                ERROR_CANT_ACCESS_FILE as i32,
                            ));
                        }
                        // This is a symlink, keep the tag info
                    } else if !is_unhandled_tag {
                        // Traverse a non-link reparse point
                        unsafe { CloseHandle(h_file) };
                        return win32_xstat_slow_impl(path, true);
                    }
                }
                tag_info = local_tag_info;
            }

            // Get file information
            let ret = unsafe { GetFileInformationByHandle(h_file, &mut file_info) };
            if ret == 0 {
                let err_code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32;
                match err_code {
                    ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED => {
                        // Volumes and physical disks are block devices
                        return Ok(StatStruct {
                            st_mode: 0x6000, // S_IFBLK
                            ..Default::default()
                        });
                    }
                    _ => return Err(std::io::Error::last_os_error()),
                }
            }

            // Get FILE_BASIC_INFO
            let mut basic_info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
            let has_basic_info = unsafe {
                GetFileInformationByHandleEx(
                    h_file,
                    FileBasicInfo,
                    &mut basic_info as *mut _ as *mut _,
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            } != 0;

            // Get FILE_ID_INFO (optional)
            let mut id_info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
            let has_id_info = unsafe {
                GetFileInformationByHandleEx(
                    h_file,
                    FileIdInfo,
                    &mut id_info as *mut _ as *mut _,
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            } != 0;

            let mut result = attribute_data_to_stat(
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
            // We got file_info from attributes_from_dir
            let mut result = attribute_data_to_stat(&file_info, tag_info.reparse_tag, None, None);
            result.update_st_mode_from_path(path, file_info.dwFileAttributes);
            Ok(result)
        }
    })();

    // Cleanup
    if h_file != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(h_file) };
    }

    result
}

fn win32_xstat_impl(path: &OsStr, traverse: bool) -> std::io::Result<StatStruct> {
    use windows_sys::Win32::{Foundation, Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT};

    let stat_info =
        get_file_information_by_name(path, FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo);
    match stat_info {
        Ok(stat_info) => {
            if (stat_info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0)
                || (!traverse && is_reparse_tag_name_surrogate(stat_info.ReparseTag))
            {
                let mut result =
                    crate::common::fileutils::windows::stat_basic_info_to_stat(&stat_info);
                result.update_st_mode_from_path(path, stat_info.FileAttributes);
                return Ok(result);
            }
        }
        Err(e) => {
            if let Some(errno) = e.raw_os_error()
                && matches!(
                    errno as u32,
                    Foundation::ERROR_FILE_NOT_FOUND
                        | Foundation::ERROR_PATH_NOT_FOUND
                        | Foundation::ERROR_NOT_READY
                        | Foundation::ERROR_BAD_NET_NAME
                )
            {
                return Err(e);
            }
        }
    }

    // Fallback to slow implementation
    win32_xstat_slow_impl(path, traverse)
}

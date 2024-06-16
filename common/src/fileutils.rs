// Python/fileutils.c in CPython
#![allow(non_snake_case)]

#[cfg(not(windows))]
pub use libc::stat as StatStruct;

#[cfg(windows)]
pub use windows::{fstat, StatStruct};

#[cfg(not(windows))]
pub fn fstat(fd: libc::c_int) -> std::io::Result<StatStruct> {
    let mut stat = std::mem::MaybeUninit::uninit();
    unsafe {
        let ret = libc::fstat(fd, stat.as_mut_ptr());
        if ret == -1 {
            Err(crate::os::last_os_error())
        } else {
            Ok(stat.assume_init())
        }
    }
}

#[cfg(windows)]
pub mod windows {
    use crate::suppress_iph;
    use crate::windows::ToWideString;
    use libc::{S_IFCHR, S_IFDIR, S_IFMT};
    use std::ffi::{CString, OsStr, OsString};
    use std::os::windows::ffi::OsStrExt;
    use std::sync::OnceLock;
    use windows_sys::core::PCWSTR;
    use windows_sys::Win32::Foundation::{
        FreeLibrary, SetLastError, BOOL, ERROR_INVALID_HANDLE, ERROR_NOT_SUPPORTED, FILETIME,
        HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        GetFileType, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO, FILE_TYPE_CHAR,
        FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_UNKNOWN,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;

    pub const S_IFIFO: libc::c_int = 0o010000;
    pub const S_IFLNK: libc::c_int = 0o120000;

    pub const SECS_BETWEEN_EPOCHS: i64 = 11644473600; // Seconds between 1.1.1601 and 1.1.1970

    #[derive(Default)]
    pub struct StatStruct {
        pub st_dev: libc::c_ulong,
        pub st_ino: u64,
        pub st_mode: libc::c_ushort,
        pub st_nlink: i32,
        pub st_uid: i32,
        pub st_gid: i32,
        pub st_rdev: libc::c_ulong,
        pub st_size: u64,
        pub st_atime: libc::time_t,
        pub st_atime_nsec: i32,
        pub st_mtime: libc::time_t,
        pub st_mtime_nsec: i32,
        pub st_ctime: libc::time_t,
        pub st_ctime_nsec: i32,
        pub st_birthtime: libc::time_t,
        pub st_birthtime_nsec: i32,
        pub st_file_attributes: libc::c_ulong,
        pub st_reparse_tag: u32,
        pub st_ino_high: u64,
    }

    impl StatStruct {
        // update_st_mode_from_path in cpython
        pub fn update_st_mode_from_path(&mut self, path: &OsStr, attr: u32) {
            if attr & FILE_ATTRIBUTE_DIRECTORY == 0 {
                let file_extension = path
                    .encode_wide()
                    .collect::<Vec<u16>>()
                    .split(|&c| c == '.' as u16)
                    .last()
                    .and_then(|s| String::from_utf16(s).ok());

                if let Some(file_extension) = file_extension {
                    if file_extension.eq_ignore_ascii_case("exe")
                        || file_extension.eq_ignore_ascii_case("bat")
                        || file_extension.eq_ignore_ascii_case("cmd")
                        || file_extension.eq_ignore_ascii_case("com")
                    {
                        self.st_mode |= 0o111;
                    }
                }
            }
        }
    }

    extern "C" {
        fn _get_osfhandle(fd: i32) -> libc::intptr_t;
    }

    fn get_osfhandle(fd: i32) -> std::io::Result<isize> {
        let ret = unsafe { suppress_iph!(_get_osfhandle(fd)) };
        if ret as HANDLE == INVALID_HANDLE_VALUE {
            Err(crate::os::last_os_error())
        } else {
            Ok(ret)
        }
    }

    // _Py_fstat_noraise in cpython
    pub fn fstat(fd: libc::c_int) -> std::io::Result<StatStruct> {
        let h = get_osfhandle(fd);
        if h.is_err() {
            unsafe { SetLastError(ERROR_INVALID_HANDLE) };
        }
        let h = h?;
        // reset stat?

        let file_type = unsafe { GetFileType(h) };
        if file_type == FILE_TYPE_UNKNOWN {
            return Err(std::io::Error::last_os_error());
        }
        if file_type != FILE_TYPE_DISK {
            let st_mode = if file_type == FILE_TYPE_CHAR {
                S_IFCHR
            } else if file_type == FILE_TYPE_PIPE {
                S_IFIFO
            } else {
                0
            } as u16;
            return Ok(StatStruct {
                st_mode,
                ..Default::default()
            });
        }

        let mut info = unsafe { std::mem::zeroed() };
        let mut basic_info: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
        let mut id_info: FILE_ID_INFO = unsafe { std::mem::zeroed() };

        if unsafe { GetFileInformationByHandle(h, &mut info) } == 0
            || unsafe {
                GetFileInformationByHandleEx(
                    h,
                    FileBasicInfo,
                    &mut basic_info as *mut _ as *mut _,
                    std::mem::size_of_val(&basic_info) as u32,
                )
            } == 0
        {
            return Err(std::io::Error::last_os_error());
        }

        let p_id_info = if unsafe {
            GetFileInformationByHandleEx(
                h,
                FileIdInfo,
                &mut id_info as *mut _ as *mut _,
                std::mem::size_of_val(&id_info) as u32,
            )
        } == 0
        {
            None
        } else {
            Some(&id_info)
        };

        Ok(attribute_data_to_stat(
            &info,
            0,
            Some(&basic_info),
            p_id_info,
        ))
    }

    fn large_integer_to_time_t_nsec(input: i64) -> (libc::time_t, libc::c_int) {
        let nsec_out = (input % 10_000_000) * 100; // FILETIME is in units of 100 nsec.
        let time_out = ((input / 10_000_000) - SECS_BETWEEN_EPOCHS) as libc::time_t;
        (time_out, nsec_out as _)
    }

    fn file_time_to_time_t_nsec(in_ptr: &FILETIME) -> (libc::time_t, libc::c_int) {
        let in_val: i64 = unsafe { std::mem::transmute_copy(in_ptr) };
        let nsec_out = (in_val % 10_000_000) * 100; // FILETIME is in units of 100 nsec.
        let time_out = (in_val / 10_000_000) - SECS_BETWEEN_EPOCHS;
        (time_out, nsec_out as _)
    }

    fn attribute_data_to_stat(
        info: &BY_HANDLE_FILE_INFORMATION,
        reparse_tag: u32,
        basic_info: Option<&FILE_BASIC_INFO>,
        id_info: Option<&FILE_ID_INFO>,
    ) -> StatStruct {
        let mut st_mode = attributes_to_mode(info.dwFileAttributes);
        let st_size = ((info.nFileSizeHigh as u64) << 32) + info.nFileSizeLow as u64;
        let st_dev: libc::c_ulong = if let Some(id_info) = id_info {
            id_info.VolumeSerialNumber as _
        } else {
            info.dwVolumeSerialNumber
        };
        let st_rdev = 0;

        let (st_birthtime, st_ctime, st_mtime, st_atime) = if let Some(basic_info) = basic_info {
            (
                large_integer_to_time_t_nsec(basic_info.CreationTime),
                large_integer_to_time_t_nsec(basic_info.ChangeTime),
                large_integer_to_time_t_nsec(basic_info.LastWriteTime),
                large_integer_to_time_t_nsec(basic_info.LastAccessTime),
            )
        } else {
            (
                file_time_to_time_t_nsec(&info.ftCreationTime),
                (0, 0),
                file_time_to_time_t_nsec(&info.ftLastWriteTime),
                file_time_to_time_t_nsec(&info.ftLastAccessTime),
            )
        };
        let st_nlink = info.nNumberOfLinks as i32;

        let st_ino = if let Some(id_info) = id_info {
            let file_id: [u64; 2] = unsafe { std::mem::transmute_copy(&id_info.FileId) };
            file_id
        } else {
            let ino = ((info.nFileIndexHigh as u64) << 32) + info.nFileIndexLow as u64;
            [ino, 0]
        };

        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && reparse_tag == IO_REPARSE_TAG_SYMLINK
        {
            st_mode = (st_mode & !(S_IFMT as u16)) | (S_IFLNK as u16);
        }
        let st_file_attributes = info.dwFileAttributes;

        StatStruct {
            st_dev,
            st_ino: st_ino[0],
            st_mode,
            st_nlink,
            st_uid: 0,
            st_gid: 0,
            st_rdev,
            st_size,
            st_atime: st_atime.0,
            st_atime_nsec: st_atime.1,
            st_mtime: st_mtime.0,
            st_mtime_nsec: st_mtime.1,
            st_ctime: st_ctime.0,
            st_ctime_nsec: st_ctime.1,
            st_birthtime: st_birthtime.0,
            st_birthtime_nsec: st_birthtime.1,
            st_file_attributes,
            st_reparse_tag: reparse_tag,
            st_ino_high: st_ino[1],
        }
    }

    fn attributes_to_mode(attr: u32) -> u16 {
        let mut m = 0;
        if attr & FILE_ATTRIBUTE_DIRECTORY != 0 {
            m |= libc::S_IFDIR | 0o111; // IFEXEC for user,group,other
        } else {
            m |= libc::S_IFREG;
        }
        if attr & FILE_ATTRIBUTE_READONLY != 0 {
            m |= 0o444;
        } else {
            m |= 0o666;
        }
        m as _
    }

    #[repr(C)]
    pub struct FILE_STAT_BASIC_INFORMATION {
        pub FileId: i64,
        pub CreationTime: i64,
        pub LastAccessTime: i64,
        pub LastWriteTime: i64,
        pub ChangeTime: i64,
        pub AllocationSize: i64,
        pub EndOfFile: i64,
        pub FileAttributes: u32,
        pub ReparseTag: u32,
        pub NumberOfLinks: u32,
        pub DeviceType: u32,
        pub DeviceCharacteristics: u32,
        pub Reserved: u32,
        pub VolumeSerialNumber: i64,
        pub FileId128: [u64; 2],
    }

    #[repr(C)]
    #[allow(dead_code)]
    pub enum FILE_INFO_BY_NAME_CLASS {
        FileStatByNameInfo,
        FileStatLxByNameInfo,
        FileCaseSensitiveByNameInfo,
        FileStatBasicByNameInfo,
        MaximumFileInfoByNameClass,
    }

    // _Py_GetFileInformationByName in cpython
    pub fn get_file_information_by_name(
        file_name: &OsStr,
        file_information_class: FILE_INFO_BY_NAME_CLASS,
    ) -> std::io::Result<FILE_STAT_BASIC_INFORMATION> {
        static GET_FILE_INFORMATION_BY_NAME: OnceLock<
            Option<
                unsafe extern "system" fn(
                    PCWSTR,
                    FILE_INFO_BY_NAME_CLASS,
                    *mut libc::c_void,
                    u32,
                ) -> BOOL,
            >,
        > = OnceLock::new();

        let GetFileInformationByName = GET_FILE_INFORMATION_BY_NAME
            .get_or_init(|| {
                let library_name = OsString::from("api-ms-win-core-file-l2-1-4").to_wide_with_nul();
                let module = unsafe { LoadLibraryW(library_name.as_ptr()) };
                if module == 0 {
                    return None;
                }
                let name = CString::new("GetFileInformationByName").unwrap();
                if let Some(proc) =
                    unsafe { GetProcAddress(module, name.as_bytes_with_nul().as_ptr()) }
                {
                    Some(unsafe {
                        std::mem::transmute::<
                            unsafe extern "system" fn() -> isize,
                            unsafe extern "system" fn(
                                *const u16,
                                FILE_INFO_BY_NAME_CLASS,
                                *mut libc::c_void,
                                u32,
                            ) -> i32,
                        >(proc)
                    })
                } else {
                    unsafe { FreeLibrary(module) };
                    None
                }
            })
            .ok_or_else(|| std::io::Error::from_raw_os_error(ERROR_NOT_SUPPORTED as _))?;

        let file_name = file_name.to_wide_with_nul();
        let file_info_buffer_size = std::mem::size_of::<FILE_STAT_BASIC_INFORMATION>() as u32;
        let mut file_info_buffer = std::mem::MaybeUninit::<FILE_STAT_BASIC_INFORMATION>::uninit();
        unsafe {
            if GetFileInformationByName(
                file_name.as_ptr(),
                file_information_class as _,
                file_info_buffer.as_mut_ptr() as _,
                file_info_buffer_size,
            ) == 0
            {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(file_info_buffer.assume_init())
            }
        }
    }
    pub fn stat_basic_info_to_stat(info: &FILE_STAT_BASIC_INFORMATION) -> StatStruct {
        use windows_sys::Win32::Storage::FileSystem;
        use windows_sys::Win32::System::Ioctl;

        const S_IFMT: u16 = self::S_IFMT as _;
        const S_IFDIR: u16 = self::S_IFDIR as _;
        const S_IFCHR: u16 = self::S_IFCHR as _;
        const S_IFIFO: u16 = self::S_IFIFO as _;
        const S_IFLNK: u16 = self::S_IFLNK as _;

        let mut st_mode = attributes_to_mode(info.FileAttributes);
        let st_size = info.EndOfFile as u64;
        let st_birthtime = large_integer_to_time_t_nsec(info.CreationTime);
        let st_ctime = large_integer_to_time_t_nsec(info.ChangeTime);
        let st_mtime = large_integer_to_time_t_nsec(info.LastWriteTime);
        let st_atime = large_integer_to_time_t_nsec(info.LastAccessTime);
        let st_nlink = info.NumberOfLinks as _;
        let st_dev = info.VolumeSerialNumber as u32;
        // File systems with less than 128-bits zero pad into this field
        let st_ino = info.FileId128;
        // bpo-37834: Only actual symlinks set the S_IFLNK flag. But lstat() will
        // open other name surrogate reparse points without traversing them. To
        // detect/handle these, check st_file_attributes and st_reparse_tag.
        let st_reparse_tag = info.ReparseTag;
        if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && info.ReparseTag == IO_REPARSE_TAG_SYMLINK
        {
            // set the bits that make this a symlink
            st_mode = (st_mode & !S_IFMT) | S_IFLNK;
        }
        let st_file_attributes = info.FileAttributes;
        match info.DeviceType {
            FileSystem::FILE_DEVICE_DISK
            | Ioctl::FILE_DEVICE_VIRTUAL_DISK
            | Ioctl::FILE_DEVICE_DFS
            | FileSystem::FILE_DEVICE_CD_ROM
            | Ioctl::FILE_DEVICE_CONTROLLER
            | Ioctl::FILE_DEVICE_DATALINK => {}
            Ioctl::FILE_DEVICE_DISK_FILE_SYSTEM
            | Ioctl::FILE_DEVICE_CD_ROM_FILE_SYSTEM
            | Ioctl::FILE_DEVICE_NETWORK_FILE_SYSTEM => {
                st_mode = (st_mode & !S_IFMT) | 0x6000; // _S_IFBLK
            }
            Ioctl::FILE_DEVICE_CONSOLE
            | Ioctl::FILE_DEVICE_NULL
            | Ioctl::FILE_DEVICE_KEYBOARD
            | Ioctl::FILE_DEVICE_MODEM
            | Ioctl::FILE_DEVICE_MOUSE
            | Ioctl::FILE_DEVICE_PARALLEL_PORT
            | Ioctl::FILE_DEVICE_PRINTER
            | Ioctl::FILE_DEVICE_SCREEN
            | Ioctl::FILE_DEVICE_SERIAL_PORT
            | Ioctl::FILE_DEVICE_SOUND => {
                st_mode = (st_mode & !S_IFMT) | S_IFCHR;
            }
            Ioctl::FILE_DEVICE_NAMED_PIPE => {
                st_mode = (st_mode & !S_IFMT) | S_IFIFO;
            }
            _ => {
                if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    st_mode = (st_mode & !S_IFMT) | S_IFDIR;
                }
            }
        }

        StatStruct {
            st_dev,
            st_ino: st_ino[0],
            st_mode,
            st_nlink,
            st_uid: 0,
            st_gid: 0,
            st_rdev: 0,
            st_size,
            st_atime: st_atime.0,
            st_atime_nsec: st_atime.1,
            st_mtime: st_mtime.0,
            st_mtime_nsec: st_mtime.1,
            st_ctime: st_ctime.0,
            st_ctime_nsec: st_ctime.1,
            st_birthtime: st_birthtime.0,
            st_birthtime_nsec: st_birthtime.1,
            st_file_attributes,
            st_reparse_tag,
            st_ino_high: st_ino[1],
        }
    }
}

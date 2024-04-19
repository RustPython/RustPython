// Python/fileutils.c in CPython
#[cfg(not(windows))]
pub use libc::stat as StatStruct;

#[cfg(windows)]
pub use windows::{fstat, StatStruct};

#[cfg(windows)]
mod windows {
    use crate::common::suppress_iph;
    use libc::{S_IFCHR, S_IFMT};
    use windows_sys::Win32::Foundation::SetLastError;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Foundation::{ERROR_INVALID_HANDLE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        GetFileType, FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_UNKNOWN,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO,
    };

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

    extern "C" {
        fn _get_osfhandle(fd: i32) -> libc::intptr_t;
    }

    fn get_osfhandle(fd: i32) -> std::io::Result<isize> {
        let ret = unsafe { suppress_iph!(_get_osfhandle(fd)) };
        if ret as HANDLE == INVALID_HANDLE_VALUE {
            Err(crate::common::os::last_os_error())
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

    fn i64_to_time_t_nsec(input: i64) -> (libc::time_t, libc::c_int) {
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
        use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;

        let mut st_mode: u16 = attributes_to_mode(info.dwFileAttributes) as _;
        let st_size = ((info.nFileSizeHigh as u64) << 32) + info.nFileSizeLow as u64;
        let st_dev: libc::c_ulong = if let Some(id_info) = id_info {
            id_info.VolumeSerialNumber as _
        } else {
            info.dwVolumeSerialNumber
        };
        let st_rdev = 0;

        let (st_birth_time, st_ctime, st_mtime, st_atime) = if let Some(basic_info) = basic_info {
            (
                i64_to_time_t_nsec(basic_info.CreationTime),
                i64_to_time_t_nsec(basic_info.ChangeTime),
                i64_to_time_t_nsec(basic_info.LastWriteTime),
                i64_to_time_t_nsec(basic_info.LastAccessTime),
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
            st_birthtime: st_birth_time.0,
            st_birthtime_nsec: st_birth_time.1,
            st_file_attributes,
            st_reparse_tag: reparse_tag,
            st_ino_high: st_ino[1],
        }
    }

    fn attributes_to_mode(attr: u32) -> libc::c_int {
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
        m
    }
}

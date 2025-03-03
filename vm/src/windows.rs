use crate::common::fileutils::{
    StatStruct,
    windows::{FILE_INFO_BY_NAME_CLASS, get_file_information_by_name},
};
use crate::{
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    convert::{ToPyObject, ToPyResult},
    stdlib::os::errno_err,
};
use std::{ffi::OsStr, time::SystemTime};
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
        HANDLE(self)
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
            if let Some(errno) = e.raw_os_error() {
                if matches!(
                    errno as u32,
                    Foundation::ERROR_FILE_NOT_FOUND
                        | Foundation::ERROR_PATH_NOT_FOUND
                        | Foundation::ERROR_NOT_READY
                        | Foundation::ERROR_BAD_NET_NAME
                ) {
                    return Err(e);
                }
            }
        }
    }

    // TODO: check if win32_xstat_slow_impl(&path, result, traverse) is required
    meta_to_stat(
        &crate::stdlib::os::fs_metadata(path, traverse)?,
        file_id(path)?,
    )
}

// Ported from zed: https://github.com/zed-industries/zed/blob/v0.131.6/crates/fs/src/fs.rs#L1532-L1562
// can we get file id not open the file twice?
// https://github.com/rust-lang/rust/issues/63010
fn file_id(path: &OsStr) -> std::io::Result<u64> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
        },
    };

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;

    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle
    // This function supports Windows XP+
    let ret = unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) };
    if ret == 0 {
        return Err(std::io::Error::last_os_error());
    };

    Ok(((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64))
}

fn meta_to_stat(meta: &std::fs::Metadata, file_id: u64) -> std::io::Result<StatStruct> {
    let st_mode = {
        // Based on CPython fileutils.c' attributes_to_mode
        let mut m = 0;
        if meta.is_dir() {
            m |= libc::S_IFDIR | 0o111; /* IFEXEC for user,group,other */
        } else {
            m |= libc::S_IFREG;
        }
        if meta.is_symlink() {
            m |= 0o100000;
        }
        if meta.permissions().readonly() {
            m |= 0o444;
        } else {
            m |= 0o666;
        }
        m as _
    };
    let (atime, mtime, ctime) = (meta.accessed()?, meta.modified()?, meta.created()?);
    let sec = |systime: SystemTime| match systime.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as libc::time_t,
        Err(e) => -(e.duration().as_secs() as libc::time_t),
    };
    let nsec = |systime: SystemTime| match systime.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.subsec_nanos() as i32,
        Err(e) => -(e.duration().subsec_nanos() as i32),
    };
    Ok(StatStruct {
        st_dev: 0,
        st_ino: file_id,
        st_mode,
        st_nlink: 0,
        st_uid: 0,
        st_gid: 0,
        st_size: meta.len(),
        st_atime: sec(atime),
        st_mtime: sec(mtime),
        st_birthtime: sec(ctime),
        st_atime_nsec: nsec(atime),
        st_mtime_nsec: nsec(mtime),
        st_birthtime_nsec: nsec(ctime),
        ..Default::default()
    })
}

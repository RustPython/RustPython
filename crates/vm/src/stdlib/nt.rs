// spell-checker:disable

pub(crate) use module::module_def;
pub use module::raw_set_handle_inheritable;

#[pymodule(name = "nt", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        Py, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDictRef, PyListRef, PyStrRef, PyTupleRef},
        common::{crt_fd, suppress_iph, windows::ToWideString},
        convert::ToPyException,
        exceptions::OSErrorBuilder,
        function::{Either, OptionalArg},
        ospath::{OsPath, OsPathOrFd},
        stdlib::os::{_os, DirFd, SupportFunc, TargetIsDirectory},
    };

    use core::mem::MaybeUninit;
    use libc::intptr_t;
    use std::os::windows::io::AsRawHandle;
    use std::{env, io, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        Foundation::{self, INVALID_HANDLE_VALUE},
        Storage::FileSystem,
        System::{Console, Threading},
    };

    #[pyattr]
    use libc::{O_BINARY, O_NOINHERIT, O_RANDOM, O_SEQUENTIAL, O_TEMPORARY, O_TEXT};

    // Windows spawn mode constants
    #[pyattr]
    const P_WAIT: i32 = 0;
    #[pyattr]
    const P_NOWAIT: i32 = 1;
    #[pyattr]
    const P_OVERLAY: i32 = 2;
    #[pyattr]
    const P_NOWAITO: i32 = 3;
    #[pyattr]
    const P_DETACH: i32 = 4;

    // _O_SHORT_LIVED is not in libc, define manually
    #[pyattr]
    const O_SHORT_LIVED: i32 = 0x1000;

    // Exit code constant
    #[pyattr]
    const EX_OK: i32 = 0;

    // Maximum number of temporary files
    #[pyattr]
    const TMP_MAX: i32 = i32::MAX;

    #[pyattr]
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_APPLICATION_DIR as _LOAD_LIBRARY_SEARCH_APPLICATION_DIR,
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS as _LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR as _LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32 as _LOAD_LIBRARY_SEARCH_SYSTEM32,
        LOAD_LIBRARY_SEARCH_USER_DIRS as _LOAD_LIBRARY_SEARCH_USER_DIRS,
    };

    #[pyfunction]
    pub(super) fn access(path: OsPath, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        let attr = unsafe { FileSystem::GetFileAttributesW(path.to_wide_cstring(vm)?.as_ptr()) };
        Ok(attr != FileSystem::INVALID_FILE_ATTRIBUTES
            && (mode & 2 == 0
                || attr & FileSystem::FILE_ATTRIBUTE_READONLY == 0
                || attr & FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0))
    }

    #[pyfunction]
    #[pyfunction(name = "unlink")]
    pub(super) fn remove(
        path: OsPath,
        dir_fd: DirFd<'static, 0>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // On Windows, use DeleteFileW directly.
        // Rust's std::fs::remove_file may have different behavior for read-only files.
        // See Py_DeleteFileW.
        use windows_sys::Win32::Storage::FileSystem::{
            DeleteFileW, FindClose, FindFirstFileW, RemoveDirectoryW, WIN32_FIND_DATAW,
        };
        use windows_sys::Win32::System::SystemServices::{
            IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
        };

        let [] = dir_fd.0;
        let wide_path = path.to_wide_cstring(vm)?;
        let attrs = unsafe { FileSystem::GetFileAttributesW(wide_path.as_ptr()) };

        let mut is_directory = false;
        let mut is_link = false;

        if attrs != FileSystem::INVALID_FILE_ATTRIBUTES {
            is_directory = (attrs & FileSystem::FILE_ATTRIBUTE_DIRECTORY) != 0;

            // Check if it's a symlink or junction point
            if is_directory && (attrs & FileSystem::FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                let mut find_data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };
                let handle = unsafe { FindFirstFileW(wide_path.as_ptr(), &mut find_data) };
                if handle != INVALID_HANDLE_VALUE {
                    is_link = find_data.dwReserved0 == IO_REPARSE_TAG_SYMLINK
                        || find_data.dwReserved0 == IO_REPARSE_TAG_MOUNT_POINT;
                    unsafe { FindClose(handle) };
                }
            }
        }

        let result = if is_directory && is_link {
            unsafe { RemoveDirectoryW(wide_path.as_ptr()) }
        } else {
            unsafe { DeleteFileW(wide_path.as_ptr()) }
        };

        if result == 0 {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }
        Ok(())
    }

    #[pyfunction]
    pub(super) fn _supports_virtual_terminal() -> PyResult<bool> {
        let mut mode = 0;
        let handle = unsafe { Console::GetStdHandle(Console::STD_ERROR_HANDLE) };
        if unsafe { Console::GetConsoleMode(handle, &mut mode) } == 0 {
            return Ok(false);
        }
        Ok(mode & Console::ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0)
    }

    #[derive(FromArgs)]
    pub(super) struct SymlinkArgs<'fd> {
        src: OsPath,
        dst: OsPath,
        #[pyarg(flatten)]
        target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<'fd, { _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(args: SymlinkArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        use crate::exceptions::ToOSErrorBuilder;
        use core::sync::atomic::{AtomicBool, Ordering};
        use windows_sys::Win32::Storage::FileSystem::WIN32_FILE_ATTRIBUTE_DATA;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateSymbolicLinkW, FILE_ATTRIBUTE_DIRECTORY, GetFileAttributesExW,
            SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, SYMBOLIC_LINK_FLAG_DIRECTORY,
        };

        static HAS_UNPRIVILEGED_FLAG: AtomicBool = AtomicBool::new(true);

        fn check_dir(src: &OsPath, dst: &OsPath) -> bool {
            use windows_sys::Win32::Storage::FileSystem::GetFileExInfoStandard;

            let dst_parent = dst.as_path().parent();
            let Some(dst_parent) = dst_parent else {
                return false;
            };
            let resolved = if src.as_path().is_absolute() {
                src.as_path().to_path_buf()
            } else {
                dst_parent.join(src.as_path())
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
                    &mut info as *mut _ as *mut _,
                )
            };
            ok != 0 && (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0
        }

        let mut flags = 0u32;
        if HAS_UNPRIVILEGED_FLAG.load(Ordering::Relaxed) {
            flags |= SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
        }
        if args.target_is_directory.target_is_directory || check_dir(&args.src, &args.dst) {
            flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
        }

        let src = args.src.to_wide_cstring(vm)?;
        let dst = args.dst.to_wide_cstring(vm)?;

        let mut result = unsafe { CreateSymbolicLinkW(dst.as_ptr(), src.as_ptr(), flags) };
        if !result
            && HAS_UNPRIVILEGED_FLAG.load(Ordering::Relaxed)
            && unsafe { Foundation::GetLastError() } == Foundation::ERROR_INVALID_PARAMETER
        {
            let flags = flags & !SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
            result = unsafe { CreateSymbolicLinkW(dst.as_ptr(), src.as_ptr(), flags) };
            if result
                || unsafe { Foundation::GetLastError() } != Foundation::ERROR_INVALID_PARAMETER
            {
                HAS_UNPRIVILEGED_FLAG.store(false, Ordering::Relaxed);
            }
        }

        if !result {
            let err = io::Error::last_os_error();
            let builder = err.to_os_error_builder(vm);
            let builder = builder
                .filename(args.src.filename(vm))
                .filename2(args.dst.filename(vm));
            return Err(builder.build(vm).upcast());
        }

        Ok(())
    }

    #[pyfunction]
    fn set_inheritable(
        fd: crt_fd::Borrowed<'_>,
        inheritable: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let handle = crt_fd::as_handle(fd).map_err(|e| e.to_pyexception(vm))?;
        set_handle_inheritable(handle.as_raw_handle() as _, inheritable, vm)
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();

        for (key, value) in env::vars() {
            // Skip hidden Windows environment variables (e.g., =C:, =D:, =ExitCode)
            // These are internal cmd.exe bookkeeping variables that store per-drive
            // current directories and cannot be reliably modified via _wputenv().
            if key.starts_with('=') {
                continue;
            }
            environ.set_item(&key, vm.new_pyobj(value), vm).unwrap();
        }
        environ
    }

    #[pyfunction]
    fn _create_environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars() {
            if key.starts_with('=') {
                continue;
            }
            environ.set_item(&key, vm.new_pyobj(value), vm).unwrap();
        }
        environ
    }

    #[derive(FromArgs)]
    struct ChmodArgs<'a> {
        #[pyarg(any)]
        path: OsPathOrFd<'a>,
        #[pyarg(any)]
        mode: u32,
        #[pyarg(flatten)]
        dir_fd: DirFd<'static, 0>,
        #[pyarg(named, name = "follow_symlinks", optional)]
        follow_symlinks: OptionalArg<bool>,
    }

    const S_IWRITE: u32 = 128;

    fn win32_hchmod(handle: Foundation::HANDLE, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandleEx,
            SetFileInformationByHandle,
        };

        // Get current file info
        let mut info: FILE_BASIC_INFO = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                &mut info as *mut _ as *mut _,
                core::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }

        // Modify readonly attribute based on S_IWRITE bit
        if mode & S_IWRITE != 0 {
            info.FileAttributes &= !FileSystem::FILE_ATTRIBUTE_READONLY;
        } else {
            info.FileAttributes |= FileSystem::FILE_ATTRIBUTE_READONLY;
        }

        // Set the new attributes
        let ret = unsafe {
            SetFileInformationByHandle(
                handle,
                FileBasicInfo,
                &info as *const _ as *const _,
                core::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }

        Ok(())
    }

    fn fchmod_impl(fd: i32, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        // Get Windows HANDLE from fd
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| e.to_pyexception(vm))?;
        let hfile = handle.as_raw_handle() as Foundation::HANDLE;
        win32_hchmod(hfile, mode, vm)
    }

    fn win32_lchmod(path: &OsPath, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW};

        let wide = path.to_wide_cstring(vm)?;
        let attr = unsafe { GetFileAttributesW(wide.as_ptr()) };
        if attr == FileSystem::INVALID_FILE_ATTRIBUTES {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path.clone(), vm));
        }
        let new_attr = if mode & S_IWRITE != 0 {
            attr & !FileSystem::FILE_ATTRIBUTE_READONLY
        } else {
            attr | FileSystem::FILE_ATTRIBUTE_READONLY
        };
        let ret = unsafe { SetFileAttributesW(wide.as_ptr(), new_attr) };
        if ret == 0 {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path.clone(), vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn fchmod(fd: i32, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        fchmod_impl(fd, mode, vm)
    }

    #[pyfunction]
    fn chmod(args: ChmodArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let ChmodArgs {
            path,
            mode,
            dir_fd,
            follow_symlinks,
        } = args;
        let [] = dir_fd.0;

        // If path is a file descriptor, use fchmod
        if let OsPathOrFd::Fd(fd) = path {
            if follow_symlinks.into_option().is_some() {
                return Err(vm.new_value_error(
                    "chmod: follow_symlinks is not supported with fd argument".to_owned(),
                ));
            }
            return fchmod_impl(fd.as_raw(), mode, vm);
        }

        let OsPathOrFd::Path(path) = path else {
            unreachable!()
        };

        let follow_symlinks = follow_symlinks.into_option().unwrap_or(false);

        if follow_symlinks {
            use windows_sys::Win32::Storage::FileSystem::{
                CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
                FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
            };

            let wide = path.to_wide_cstring(vm)?;
            let handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    core::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    core::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                let err = io::Error::last_os_error();
                return Err(OSErrorBuilder::with_filename(&err, path, vm));
            }
            let result = win32_hchmod(handle, mode, vm);
            unsafe { Foundation::CloseHandle(handle) };
            result
        } else {
            win32_lchmod(&path, mode, vm)
        }
    }

    /// Get the real file name (with correct case) without accessing the file.
    /// Uses FindFirstFileW to get the name as stored on the filesystem.
    #[pyfunction]
    fn _findfirstfile(path: OsPath, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        use crate::common::windows::ToWideString;
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FindClose, FindFirstFileW, WIN32_FIND_DATAW,
        };

        let wide_path = path.as_ref().to_wide_with_nul();
        let mut find_data: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };

        let handle = unsafe { FindFirstFileW(wide_path.as_ptr(), &mut find_data) };
        if handle == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }

        unsafe { FindClose(handle) };

        // Convert the filename from the find data to a Rust string
        // cFileName is a null-terminated wide string
        let len = find_data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(find_data.cFileName.len());
        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[..len]);
        let filename_str = filename
            .to_str()
            .ok_or_else(|| vm.new_unicode_decode_error("filename contains invalid UTF-8"))?;

        Ok(vm.ctx.new_str(filename_str).to_owned())
    }

    #[derive(FromArgs)]
    struct PathArg {
        #[pyarg(any)]
        path: crate::PyObjectRef,
    }

    impl PathArg {
        fn to_path_or_fd(&self, vm: &VirtualMachine) -> Option<OsPathOrFd<'static>> {
            OsPathOrFd::try_from_object(vm, self.path.clone()).ok()
        }
    }

    // File type test constants (PY_IF* constants - internal, not from Windows API)
    const PY_IFREG: u32 = 1; // Regular file
    const PY_IFDIR: u32 = 2; // Directory
    const PY_IFLNK: u32 = 4; // Symlink
    const PY_IFMNT: u32 = 8; // Mount point (junction)
    const PY_IFLRP: u32 = 16; // Link Reparse Point (name-surrogate, symlink, junction)
    const PY_IFRRP: u32 = 32; // Regular Reparse Point

    /// _testInfo - determine file type based on attributes and reparse tag
    fn _test_info(attributes: u32, reparse_tag: u32, disk_device: bool, tested_type: u32) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        use windows_sys::Win32::System::SystemServices::{
            IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
        };

        match tested_type {
            PY_IFREG => {
                // diskDevice && attributes && !(attributes & FILE_ATTRIBUTE_DIRECTORY)
                disk_device && attributes != 0 && (attributes & FILE_ATTRIBUTE_DIRECTORY) == 0
            }
            PY_IFDIR => (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0,
            PY_IFLNK => {
                (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                    && reparse_tag == IO_REPARSE_TAG_SYMLINK
            }
            PY_IFMNT => {
                (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                    && reparse_tag == IO_REPARSE_TAG_MOUNT_POINT
            }
            PY_IFLRP => {
                (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                    && is_reparse_tag_name_surrogate(reparse_tag)
            }
            PY_IFRRP => {
                (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
                    && reparse_tag != 0
                    && !is_reparse_tag_name_surrogate(reparse_tag)
            }
            _ => false,
        }
    }

    fn is_reparse_tag_name_surrogate(tag: u32) -> bool {
        (tag & 0x20000000) != 0
    }

    fn file_info_error_is_trustworthy(error: u32) -> bool {
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

    /// _testFileTypeByHandle - test file type using an open handle
    fn _test_file_type_by_handle(
        handle: windows_sys::Win32::Foundation::HANDLE,
        tested_type: u32,
        disk_only: bool,
    ) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO, FILE_TYPE_DISK,
            FileAttributeTagInfo as FileAttributeTagInfoClass, FileBasicInfo,
            GetFileInformationByHandleEx, GetFileType,
        };

        let disk_device = unsafe { GetFileType(handle) } == FILE_TYPE_DISK;
        if disk_only && !disk_device {
            return false;
        }

        if tested_type != PY_IFREG && tested_type != PY_IFDIR {
            // For symlinks/junctions, need FileAttributeTagInfo to get reparse tag
            let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { core::mem::zeroed() };
            let ret = unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileAttributeTagInfoClass,
                    &mut info as *mut _ as *mut _,
                    core::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
                )
            };
            if ret == 0 {
                return false;
            }
            _test_info(
                info.FileAttributes,
                info.ReparseTag,
                disk_device,
                tested_type,
            )
        } else {
            // For regular files/directories, FileBasicInfo is sufficient
            let mut info: FILE_BASIC_INFO = unsafe { core::mem::zeroed() };
            let ret = unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileBasicInfo,
                    &mut info as *mut _ as *mut _,
                    core::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            };
            if ret == 0 {
                return false;
            }
            _test_info(info.FileAttributes, 0, disk_device, tested_type)
        }
    }

    /// _testFileTypeByName - test file type by path name
    fn _test_file_type_by_name(path: &std::path::Path, tested_type: u32) -> bool {
        use crate::common::fileutils::windows::{
            FILE_INFO_BY_NAME_CLASS, get_file_information_by_name,
        };
        use crate::common::windows::ToWideString;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, OPEN_EXISTING,
        };
        use windows_sys::Win32::Storage::FileSystem::{FILE_DEVICE_CD_ROM, FILE_DEVICE_DISK};
        use windows_sys::Win32::System::Ioctl::FILE_DEVICE_VIRTUAL_DISK;

        match get_file_information_by_name(
            path.as_os_str(),
            FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo,
        ) {
            Ok(info) => {
                let disk_device = matches!(
                    info.DeviceType,
                    FILE_DEVICE_DISK | FILE_DEVICE_VIRTUAL_DISK | FILE_DEVICE_CD_ROM
                );
                let result = _test_info(
                    info.FileAttributes,
                    info.ReparseTag,
                    disk_device,
                    tested_type,
                );
                if !result
                    || (tested_type != PY_IFREG && tested_type != PY_IFDIR)
                    || (info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) == 0
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

        let wide_path = path.to_wide_with_nul();

        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if tested_type != PY_IFREG && tested_type != PY_IFDIR {
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
            let result = _test_file_type_by_handle(handle, tested_type, false);
            unsafe { CloseHandle(handle) };
            return result;
        }

        match unsafe { windows_sys::Win32::Foundation::GetLastError() } {
            windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
            | windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION
            | windows_sys::Win32::Foundation::ERROR_CANT_ACCESS_FILE
            | windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER => {
                let stat = if tested_type == PY_IFREG || tested_type == PY_IFDIR {
                    crate::windows::win32_xstat(path.as_os_str(), true)
                } else {
                    crate::windows::win32_xstat(path.as_os_str(), false)
                };
                if let Ok(st) = stat {
                    let disk_device = (st.st_mode & libc::S_IFREG as u16) != 0;
                    return _test_info(
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

    /// _testFileExistsByName - test if path exists
    fn _test_file_exists_by_name(path: &std::path::Path, follow_links: bool) -> bool {
        use crate::common::fileutils::windows::{
            FILE_INFO_BY_NAME_CLASS, get_file_information_by_name,
        };
        use crate::common::windows::ToWideString;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, OPEN_EXISTING,
        };

        match get_file_information_by_name(
            path.as_os_str(),
            FILE_INFO_BY_NAME_CLASS::FileStatBasicByNameInfo,
        ) {
            Ok(info) => {
                if (info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) == 0
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

        let wide_path = path.to_wide_with_nul();
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
            let is_regular_reparse_point = _test_file_type_by_handle(handle, PY_IFRRP, false);
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

        match unsafe { windows_sys::Win32::Foundation::GetLastError() } {
            windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
            | windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION
            | windows_sys::Win32::Foundation::ERROR_CANT_ACCESS_FILE
            | windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER => {
                let stat = crate::windows::win32_xstat(path.as_os_str(), follow_links);
                return stat.is_ok();
            }
            _ => {}
        }

        false
    }

    /// _testFileType wrapper - handles both fd and path
    fn _test_file_type(path_or_fd: &OsPathOrFd<'_>, tested_type: u32) -> bool {
        match path_or_fd {
            OsPathOrFd::Fd(fd) => {
                if let Ok(handle) = crate::common::crt_fd::as_handle(*fd) {
                    use std::os::windows::io::AsRawHandle;
                    _test_file_type_by_handle(handle.as_raw_handle() as _, tested_type, true)
                } else {
                    false
                }
            }
            OsPathOrFd::Path(path) => _test_file_type_by_name(path.as_ref(), tested_type),
        }
    }

    /// _testFileExists wrapper - handles both fd and path
    fn _test_file_exists(path_or_fd: &OsPathOrFd<'_>, follow_links: bool) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_UNKNOWN, GetFileType};

        match path_or_fd {
            OsPathOrFd::Fd(fd) => {
                if let Ok(handle) = crate::common::crt_fd::as_handle(*fd) {
                    use std::os::windows::io::AsRawHandle;
                    let file_type = unsafe { GetFileType(handle.as_raw_handle() as _) };
                    // GetFileType(hfile) != FILE_TYPE_UNKNOWN || !GetLastError()
                    if file_type != FILE_TYPE_UNKNOWN {
                        return true;
                    }
                    // Check if GetLastError is 0 (no error means valid handle)
                    unsafe { windows_sys::Win32::Foundation::GetLastError() == 0 }
                } else {
                    false
                }
            }
            OsPathOrFd::Path(path) => _test_file_exists_by_name(path.as_ref(), follow_links),
        }
    }

    /// Check if a path is a directory.
    /// return _testFileType(path, PY_IFDIR)
    #[pyfunction]
    fn _path_isdir(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_type(&p, PY_IFDIR))
    }

    /// Check if a path is a regular file.
    /// return _testFileType(path, PY_IFREG)
    #[pyfunction]
    fn _path_isfile(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_type(&p, PY_IFREG))
    }

    /// Check if a path is a symbolic link.
    /// return _testFileType(path, PY_IFLNK)
    #[pyfunction]
    fn _path_islink(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_type(&p, PY_IFLNK))
    }

    /// Check if a path is a junction (mount point).
    /// return _testFileType(path, PY_IFMNT)
    #[pyfunction]
    fn _path_isjunction(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_type(&p, PY_IFMNT))
    }

    /// Check if a path exists (follows symlinks).
    /// return _testFileExists(path, TRUE)
    #[pyfunction]
    fn _path_exists(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_exists(&p, true))
    }

    /// Check if a path exists (does not follow symlinks).
    /// return _testFileExists(path, FALSE)
    #[pyfunction]
    fn _path_lexists(args: PathArg, vm: &VirtualMachine) -> bool {
        args.to_path_or_fd(vm)
            .is_some_and(|p| _test_file_exists(&p, false))
    }

    /// Check if a path is on a Windows Dev Drive.
    #[pyfunction]
    fn _path_isdevdrive(path: OsPath, vm: &VirtualMachine) -> PyResult<bool> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            FILE_SHARE_WRITE, GetDriveTypeW, GetVolumePathNameW, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::IO::DeviceIoControl;
        use windows_sys::Win32::System::Ioctl::FSCTL_QUERY_PERSISTENT_VOLUME_STATE;
        use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

        // PERSISTENT_VOLUME_STATE_DEV_VOLUME flag - not yet in windows-sys
        const PERSISTENT_VOLUME_STATE_DEV_VOLUME: u32 = 0x00002000;

        // FILE_FS_PERSISTENT_VOLUME_INFORMATION structure
        #[repr(C)]
        struct FileFsPersistentVolumeInformation {
            volume_flags: u32,
            flag_mask: u32,
            version: u32,
            reserved: u32,
        }

        let wide_path = path.to_wide_cstring(vm)?;
        let mut volume = [0u16; Foundation::MAX_PATH as usize];

        // Get volume path
        let ret = unsafe {
            GetVolumePathNameW(wide_path.as_ptr(), volume.as_mut_ptr(), volume.len() as _)
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }

        // Check if it's a fixed drive
        if unsafe { GetDriveTypeW(volume.as_ptr()) } != DRIVE_FIXED {
            return Ok(false);
        }

        // Open the volume
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
            return Err(vm.new_last_os_error());
        }

        // Query persistent volume state
        let mut volume_state = FileFsPersistentVolumeInformation {
            volume_flags: 0,
            flag_mask: PERSISTENT_VOLUME_STATE_DEV_VOLUME,
            version: 1,
            reserved: 0,
        };

        let ret = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
                &volume_state as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<FileFsPersistentVolumeInformation>() as u32,
                &mut volume_state as *mut _ as *mut core::ffi::c_void,
                core::mem::size_of::<FileFsPersistentVolumeInformation>() as u32,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };

        unsafe { CloseHandle(handle) };

        if ret == 0 {
            let err = io::Error::last_os_error();
            // ERROR_INVALID_PARAMETER means not supported on this platform
            if err.raw_os_error() == Some(Foundation::ERROR_INVALID_PARAMETER as i32) {
                return Ok(false);
            }
            return Err(err.to_pyexception(vm));
        }

        Ok((volume_state.volume_flags & PERSISTENT_VOLUME_STATE_DEV_VOLUME) != 0)
    }

    // cwait is available on MSVC only
    #[cfg(target_env = "msvc")]
    unsafe extern "C" {
        fn _cwait(termstat: *mut i32, procHandle: intptr_t, action: i32) -> intptr_t;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn waitpid(pid: intptr_t, opt: i32, vm: &VirtualMachine) -> PyResult<(intptr_t, u64)> {
        let mut status: i32 = 0;
        let pid = unsafe { suppress_iph!(_cwait(&mut status, pid, opt)) };
        if pid == -1 {
            Err(vm.new_last_errno_error())
        } else {
            // Cast to unsigned to handle large exit codes (like 0xC000013A)
            // then shift left by 8 to match POSIX waitpid format
            let ustatus = (status as u32) as u64;
            Ok((pid, ustatus << 8))
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(intptr_t, u64)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        let sig = sig as u32;
        let pid = pid as u32;

        if sig == Console::CTRL_C_EVENT || sig == Console::CTRL_BREAK_EVENT {
            let ret = unsafe { Console::GenerateConsoleCtrlEvent(sig, pid) };
            let res = if ret == 0 {
                Err(vm.new_last_os_error())
            } else {
                Ok(())
            };
            return res;
        }

        let h = unsafe { Threading::OpenProcess(Threading::PROCESS_ALL_ACCESS, 0, pid) };
        if h.is_null() {
            return Err(vm.new_last_os_error());
        }
        let ret = unsafe { Threading::TerminateProcess(h, sig) };
        let res = if ret == 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(())
        };
        unsafe { Foundation::CloseHandle(h) };
        res
    }

    #[pyfunction]
    fn get_terminal_size(
        fd: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<_os::TerminalSizeData> {
        let fd = fd.unwrap_or(1); // default to stdout

        // Use _get_osfhandle for all fds
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| e.to_pyexception(vm))?;
        let h = handle.as_raw_handle() as Foundation::HANDLE;

        let mut csbi = MaybeUninit::uninit();
        let ret = unsafe { Console::GetConsoleScreenBufferInfo(h, csbi.as_mut_ptr()) };
        if ret == 0 {
            // Check if error is due to lack of read access on a console handle
            // ERROR_ACCESS_DENIED (5) means it's a console but without read permission
            // In that case, try opening CONOUT$ directly with read access
            let err = unsafe { Foundation::GetLastError() };
            if err != Foundation::ERROR_ACCESS_DENIED {
                return Err(vm.new_last_os_error());
            }
            let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
            let console_handle = unsafe {
                FileSystem::CreateFileW(
                    conout.as_ptr(),
                    Foundation::GENERIC_READ | Foundation::GENERIC_WRITE,
                    FileSystem::FILE_SHARE_READ | FileSystem::FILE_SHARE_WRITE,
                    core::ptr::null(),
                    FileSystem::OPEN_EXISTING,
                    0,
                    core::ptr::null_mut(),
                )
            };
            if console_handle == INVALID_HANDLE_VALUE {
                return Err(vm.new_last_os_error());
            }
            let ret =
                unsafe { Console::GetConsoleScreenBufferInfo(console_handle, csbi.as_mut_ptr()) };
            unsafe { Foundation::CloseHandle(console_handle) };
            if ret == 0 {
                return Err(vm.new_last_os_error());
            }
        }
        let csbi = unsafe { csbi.assume_init() };
        let w = csbi.srWindow;
        let columns = (w.Right - w.Left + 1) as usize;
        let lines = (w.Bottom - w.Top + 1) as usize;
        Ok(_os::TerminalSizeData { columns, lines })
    }

    #[cfg(target_env = "msvc")]
    unsafe extern "C" {
        fn _wexecv(cmdname: *const u16, argv: *const *const u16) -> intptr_t;
        fn _wexecve(
            cmdname: *const u16,
            argv: *const *const u16,
            envp: *const *const u16,
        ) -> intptr_t;
        fn _wspawnv(mode: i32, cmdname: *const u16, argv: *const *const u16) -> intptr_t;
        fn _wspawnve(
            mode: i32,
            cmdname: *const u16,
            argv: *const *const u16,
            envp: *const *const u16,
        ) -> intptr_t;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn spawnv(
        mode: i32,
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<intptr_t> {
        use crate::function::FsPath;
        use core::iter::once;

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let fspath = FsPath::try_from_path_like(obj, true, vm)?;
            fspath.to_wide_cstring(vm)
        })?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("spawnv() arg 3 must not be empty"))?;

        if first.is_empty() {
            return Err(vm.new_value_error("spawnv() arg 3 first element cannot be empty"));
        }

        let argv_spawn: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        let result = unsafe { suppress_iph!(_wspawnv(mode, path.as_ptr(), argv_spawn.as_ptr())) };
        if result == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(result)
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn spawnve(
        mode: i32,
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        env: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<intptr_t> {
        use crate::function::FsPath;
        use core::iter::once;

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let fspath = FsPath::try_from_path_like(obj, true, vm)?;
            fspath.to_wide_cstring(vm)
        })?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("spawnve() arg 2 cannot be empty"))?;

        if first.is_empty() {
            return Err(vm.new_value_error("spawnve() arg 2 first element cannot be empty"));
        }

        let argv_spawn: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        // Build environment strings as "KEY=VALUE\0" wide strings
        let mut env_strings: Vec<widestring::WideCString> = Vec::new();
        for (key, value) in env.into_iter() {
            let key = FsPath::try_from_path_like(key, true, vm)?;
            let value = FsPath::try_from_path_like(value, true, vm)?;
            let key_str = key.to_string_lossy();
            let value_str = value.to_string_lossy();

            // Validate: empty key or '=' in key after position 0
            // (search from index 1 because on Windows starting '=' is allowed
            // for defining hidden environment variables)
            if key_str.is_empty() || key_str.get(1..).is_some_and(|s| s.contains('=')) {
                return Err(vm.new_value_error("illegal environment variable name"));
            }

            let env_str = format!("{}={}", key_str, value_str);
            env_strings.push(
                widestring::WideCString::from_os_str(&*std::ffi::OsString::from(env_str))
                    .map_err(|err| err.to_pyexception(vm))?,
            );
        }

        let envp: Vec<*const u16> = env_strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        let result = unsafe {
            suppress_iph!(_wspawnve(
                mode,
                path.as_ptr(),
                argv_spawn.as_ptr(),
                envp.as_ptr()
            ))
        };
        if result == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(result)
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn execv(
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use core::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.as_str())
        })?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty"))?;

        if first.is_empty() {
            return Err(vm.new_value_error("execv() arg 2 first element cannot be empty"));
        }

        let argv_execv: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        if (unsafe { suppress_iph!(_wexecv(path.as_ptr(), argv_execv.as_ptr())) } == -1) {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn execve(
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        env: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use core::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.as_str())
        })?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execve: argv must not be empty"))?;

        if first.is_empty() {
            return Err(vm.new_value_error("execve: argv first element cannot be empty"));
        }

        let argv_execve: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        // Build environment strings as "KEY=VALUE\0" wide strings
        let mut env_strings: Vec<widestring::WideCString> = Vec::new();
        for (key, value) in env.into_iter() {
            let key = PyStrRef::try_from_object(vm, key)?;
            let value = PyStrRef::try_from_object(vm, value)?;
            let key_str = key.as_str();
            let value_str = value.as_str();

            // Validate: no null characters in key or value
            if key_str.contains('\0') || value_str.contains('\0') {
                return Err(vm.new_value_error("embedded null character"));
            }
            // Validate: empty key or '=' in key after position 0
            // (search from index 1 because on Windows starting '=' is allowed
            // for defining hidden environment variables)
            if key_str.is_empty() || key_str.get(1..).is_some_and(|s| s.contains('=')) {
                return Err(vm.new_value_error("illegal environment variable name"));
            }

            let env_str = format!("{}={}", key_str, value_str);
            env_strings.push(make_widestring(&env_str)?);
        }

        let envp: Vec<*const u16> = env_strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(once(core::ptr::null()))
            .collect();

        if (unsafe { suppress_iph!(_wexecve(path.as_ptr(), argv_execve.as_ptr(), envp.as_ptr())) }
            == -1)
        {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn _getfinalpathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, GetFinalPathNameByHandleW, OPEN_EXISTING,
            VOLUME_NAME_DOS,
        };

        let wide = path.to_wide_cstring(vm)?;
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
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }

        let mut buffer: Vec<u16> = vec![0; Foundation::MAX_PATH as usize];
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
                let err = io::Error::last_os_error();
                let _ = unsafe { Foundation::CloseHandle(handle) };
                return Err(OSErrorBuilder::with_filename(&err, path, vm));
            }
            if (ret as usize) < buffer.len() {
                let final_path = std::ffi::OsString::from_wide(&buffer[..ret as usize]);
                break Ok(path.mode().process_path(final_path, vm));
            }
            buffer.resize(ret as usize, 0);
        };

        unsafe { Foundation::CloseHandle(handle) };
        result
    }

    #[pyfunction]
    fn _getfullpathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wpath = path.to_wide_cstring(vm)?;
        let mut buffer = vec![0u16; Foundation::MAX_PATH as usize];
        let ret = unsafe {
            FileSystem::GetFullPathNameW(
                wpath.as_ptr(),
                buffer.len() as _,
                buffer.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        };
        if ret == 0 {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path.clone(), vm));
        }
        if ret as usize > buffer.len() {
            buffer.resize(ret as usize, 0);
            let ret = unsafe {
                FileSystem::GetFullPathNameW(
                    wpath.as_ptr(),
                    buffer.len() as _,
                    buffer.as_mut_ptr(),
                    core::ptr::null_mut(),
                )
            };
            if ret == 0 {
                let err = io::Error::last_os_error();
                return Err(OSErrorBuilder::with_filename(&err, path.clone(), vm));
            }
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        Ok(path.mode().process_path(buffer.to_os_string(), vm))
    }

    #[pyfunction]
    fn _getvolumepathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wide = path.to_wide_cstring(vm)?;
        let buflen = core::cmp::max(wide.len(), Foundation::MAX_PATH as usize);
        if buflen > u32::MAX as usize {
            return Err(vm.new_overflow_error("path too long".to_owned()));
        }
        let mut buffer = vec![0u16; buflen];
        let ret = unsafe {
            FileSystem::GetVolumePathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buflen as _)
        };
        if ret == 0 {
            let err = io::Error::last_os_error();
            return Err(OSErrorBuilder::with_filename(&err, path, vm));
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        Ok(path.mode().process_path(buffer.to_os_string(), vm))
    }

    /// Implements _Py_skiproot logic for Windows paths
    /// Returns (drive_size, root_size) where:
    /// - drive_size: length of the drive/UNC portion
    /// - root_size: length of the root separator (0 or 1)
    fn skiproot(path: &[u16]) -> (usize, usize) {
        let len = path.len();
        if len == 0 {
            return (0, 0);
        }

        const SEP: u16 = b'\\' as u16;
        const ALTSEP: u16 = b'/' as u16;
        const COLON: u16 = b':' as u16;

        let is_sep = |c: u16| c == SEP || c == ALTSEP;
        let get = |i: usize| path.get(i).copied().unwrap_or(0);

        if is_sep(get(0)) {
            if is_sep(get(1)) {
                // UNC or device path: \\server\share or \\?\device
                // Check for \\?\UNC\server\share
                let idx = if len >= 8
                    && get(2) == b'?' as u16
                    && is_sep(get(3))
                    && (get(4) == b'U' as u16 || get(4) == b'u' as u16)
                    && (get(5) == b'N' as u16 || get(5) == b'n' as u16)
                    && (get(6) == b'C' as u16 || get(6) == b'c' as u16)
                    && is_sep(get(7))
                {
                    8
                } else {
                    2
                };

                // Find the end of server name
                let mut i = idx;
                while i < len && !is_sep(get(i)) {
                    i += 1;
                }

                if i >= len {
                    // No share part: \\server
                    return (i, 0);
                }

                // Skip separator and find end of share name
                i += 1;
                while i < len && !is_sep(get(i)) {
                    i += 1;
                }

                // drive = \\server\share, root = \ (if present)
                if i >= len { (i, 0) } else { (i, 1) }
            } else {
                // Relative path with root: \Windows
                (0, 1)
            }
        } else if len >= 2 && get(1) == COLON {
            // Drive letter path
            if len >= 3 && is_sep(get(2)) {
                // Absolute: X:\Windows
                (2, 1)
            } else {
                // Relative with drive: X:Windows
                (2, 0)
            }
        } else {
            // Relative path: Windows
            (0, 0)
        }
    }

    #[pyfunction]
    fn _path_splitroot_ex(path: crate::PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        use crate::builtins::{PyBytes, PyStr};
        use rustpython_common::wtf8::Wtf8Buf;

        // Handle path-like objects via os.fspath, but without null check (non_strict=True)
        let path = if let Some(fspath) = vm.get_method(path.clone(), identifier!(vm, __fspath__)) {
            fspath?.call((), vm)?
        } else {
            path
        };

        // Convert to wide string, validating UTF-8 for bytes input
        let (wide, is_bytes): (Vec<u16>, bool) = if let Some(s) = path.downcast_ref::<PyStr>() {
            // Use encode_wide which handles WTF-8 (including surrogates)
            let wide: Vec<u16> = s.as_wtf8().encode_wide().collect();
            (wide, false)
        } else if let Some(b) = path.downcast_ref::<PyBytes>() {
            // On Windows, bytes must be valid UTF-8 - this raises UnicodeDecodeError if not
            let s = core::str::from_utf8(b.as_bytes()).map_err(|e| {
                vm.new_exception_msg(
                    vm.ctx.exceptions.unicode_decode_error.to_owned(),
                    format!(
                        "'utf-8' codec can't decode byte {:#x} in position {}: invalid start byte",
                        b.as_bytes().get(e.valid_up_to()).copied().unwrap_or(0),
                        e.valid_up_to()
                    ),
                )
            })?;
            let wide: Vec<u16> = s.encode_utf16().collect();
            (wide, true)
        } else {
            return Err(vm.new_type_error(format!(
                "expected str or bytes, not {}",
                path.class().name()
            )));
        };

        // Normalize slashes for parsing
        let normalized: Vec<u16> = wide
            .iter()
            .map(|&c| if c == b'/' as u16 { b'\\' as u16 } else { c })
            .collect();

        let (drv_size, root_size) = skiproot(&normalized);

        // Return as bytes if input was bytes, preserving the original content
        if is_bytes {
            // Convert UTF-16 back to UTF-8 for bytes output
            let drv = String::from_utf16(&wide[..drv_size])
                .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?;
            let root = String::from_utf16(&wide[drv_size..drv_size + root_size])
                .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?;
            let tail = String::from_utf16(&wide[drv_size + root_size..])
                .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?;
            Ok(vm.ctx.new_tuple(vec![
                vm.ctx.new_bytes(drv.into_bytes()).into(),
                vm.ctx.new_bytes(root.into_bytes()).into(),
                vm.ctx.new_bytes(tail.into_bytes()).into(),
            ]))
        } else {
            // For str output, use WTF-8 to handle surrogates
            let drv = Wtf8Buf::from_wide(&wide[..drv_size]);
            let root = Wtf8Buf::from_wide(&wide[drv_size..drv_size + root_size]);
            let tail = Wtf8Buf::from_wide(&wide[drv_size + root_size..]);
            Ok(vm.ctx.new_tuple(vec![
                vm.ctx.new_str(drv).into(),
                vm.ctx.new_str(root).into(),
                vm.ctx.new_str(tail).into(),
            ]))
        }
    }

    #[pyfunction]
    fn _path_splitroot(
        path: OsPath,
        _vm: &VirtualMachine,
    ) -> (
        rustpython_common::wtf8::Wtf8Buf,
        rustpython_common::wtf8::Wtf8Buf,
    ) {
        use rustpython_common::wtf8::Wtf8Buf;

        let orig: Vec<_> = path.path.to_wide();
        if orig.is_empty() {
            return (Wtf8Buf::new(), Wtf8Buf::new());
        }
        let backslashed: Vec<_> = orig
            .iter()
            .copied()
            .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
            .chain(core::iter::once(0)) // null-terminated
            .collect();

        let mut end: *const u16 = core::ptr::null();
        let hr = unsafe {
            windows_sys::Win32::UI::Shell::PathCchSkipRoot(backslashed.as_ptr(), &mut end)
        };
        if hr >= 0 {
            assert!(!end.is_null());
            let len: usize = unsafe { end.offset_from(backslashed.as_ptr()) }
                .try_into()
                .expect("len must be non-negative");
            assert!(
                len < backslashed.len(), // backslashed is null-terminated
                "path: {:?} {} < {}",
                std::path::PathBuf::from(std::ffi::OsString::from_wide(&backslashed)),
                len,
                backslashed.len()
            );
            if len != 0 {
                (
                    Wtf8Buf::from_wide(&orig[..len]),
                    Wtf8Buf::from_wide(&orig[len..]),
                )
            } else {
                (Wtf8Buf::from_wide(&orig), Wtf8Buf::new())
            }
        } else {
            (Wtf8Buf::new(), Wtf8Buf::from_wide(&orig))
        }
    }

    /// Normalize a wide-char path (faithful port of _Py_normpath_and_size).
    /// Uses lastC tracking like the C implementation.
    fn normpath_wide(path: &[u16]) -> Vec<u16> {
        if path.is_empty() {
            return vec![b'.' as u16];
        }

        const SEP: u16 = b'\\' as u16;
        const ALTSEP: u16 = b'/' as u16;
        const DOT: u16 = b'.' as u16;

        let is_sep = |c: u16| c == SEP || c == ALTSEP;
        let sep_or_end = |input: &[u16], idx: usize| idx >= input.len() || is_sep(input[idx]);

        // Work on a mutable copy with normalized separators
        let mut buf: Vec<u16> = path
            .iter()
            .map(|&c| if c == ALTSEP { SEP } else { c })
            .collect();

        let (drv_size, root_size) = skiproot(&buf);
        let prefix_len = drv_size + root_size;

        // p1 = read cursor, p2 = write cursor
        let mut p1 = prefix_len;
        let mut p2 = prefix_len;
        let mut min_p2 = if prefix_len > 0 { prefix_len } else { 0 };
        let mut last_c: u16 = if prefix_len > 0 {
            min_p2 = prefix_len - 1;
            let c = buf[min_p2];
            // On Windows, if last char of prefix is not SEP, advance min_p2
            if c != SEP {
                min_p2 = prefix_len;
            }
            c
        } else {
            0
        };

        // Skip leading ".\" after prefix
        if p1 < buf.len() && buf[p1] == DOT && sep_or_end(&buf, p1 + 1) {
            p1 += 1;
            last_c = SEP; // treat as if we consumed a separator
            while p1 < buf.len() && buf[p1] == SEP {
                p1 += 1;
            }
        }

        while p1 < buf.len() {
            let c = buf[p1];

            if last_c == SEP {
                if c == DOT {
                    let sep_at_1 = sep_or_end(&buf, p1 + 1);
                    let sep_at_2 = !sep_at_1 && sep_or_end(&buf, p1 + 2);
                    if sep_at_2 && buf[p1 + 1] == DOT {
                        // ".." component
                        let mut p3 = p2;
                        while p3 != min_p2 && buf[p3 - 1] == SEP {
                            p3 -= 1;
                        }
                        while p3 != min_p2 && buf[p3 - 1] != SEP {
                            p3 -= 1;
                        }
                        if p2 == min_p2
                            || (buf[p3] == DOT
                                && p3 + 1 < buf.len()
                                && buf[p3 + 1] == DOT
                                && (p3 + 2 >= buf.len() || buf[p3 + 2] == SEP))
                        {
                            // Previous segment is also ../ or at minimum
                            buf[p2] = DOT;
                            p2 += 1;
                            buf[p2] = DOT;
                            p2 += 1;
                            last_c = DOT;
                        } else if buf[p3] == SEP {
                            // Absolute path - absorb segment
                            p2 = p3 + 1;
                            // last_c stays SEP
                        } else {
                            p2 = p3;
                            // last_c stays SEP
                        }
                        p1 += 1; // skip second dot (first dot is current p1)
                    } else if sep_at_1 {
                        // "." component - skip
                    } else {
                        buf[p2] = c;
                        p2 += 1;
                        last_c = c;
                    }
                } else if c == SEP {
                    // Collapse multiple separators - skip
                } else {
                    buf[p2] = c;
                    p2 += 1;
                    last_c = c;
                }
            } else {
                buf[p2] = c;
                p2 += 1;
                last_c = c;
            }

            p1 += 1;
        }

        // Null-terminate style: trim trailing separators
        if p2 != min_p2 {
            while p2 > min_p2 + 1 && buf[p2 - 1] == SEP {
                p2 -= 1;
            }
        }

        buf.truncate(p2);

        if buf.is_empty() { vec![DOT] } else { buf }
    }

    #[pyfunction]
    fn _path_normpath(path: crate::PyObjectRef, vm: &VirtualMachine) -> PyResult {
        use crate::builtins::{PyBytes, PyStr};
        use rustpython_common::wtf8::Wtf8Buf;

        // Handle path-like objects via os.fspath
        let path = if let Some(fspath) = vm.get_method(path.clone(), identifier!(vm, __fspath__)) {
            fspath?.call((), vm)?
        } else {
            path
        };

        let (wide, is_bytes): (Vec<u16>, bool) = if let Some(s) = path.downcast_ref::<PyStr>() {
            let wide: Vec<u16> = s.as_wtf8().encode_wide().collect();
            (wide, false)
        } else if let Some(b) = path.downcast_ref::<PyBytes>() {
            let s = core::str::from_utf8(b.as_bytes()).map_err(|e| {
                vm.new_exception_msg(
                    vm.ctx.exceptions.unicode_decode_error.to_owned(),
                    format!(
                        "'utf-8' codec can't decode byte {:#x} in position {}: invalid start byte",
                        b.as_bytes().get(e.valid_up_to()).copied().unwrap_or(0),
                        e.valid_up_to()
                    ),
                )
            })?;
            let wide: Vec<u16> = s.encode_utf16().collect();
            (wide, true)
        } else {
            return Err(vm.new_type_error(format!(
                "expected str or bytes, not {}",
                path.class().name()
            )));
        };

        let normalized = normpath_wide(&wide);

        if is_bytes {
            let s = String::from_utf16(&normalized)
                .map_err(|e| vm.new_unicode_decode_error(e.to_string()))?;
            Ok(vm.ctx.new_bytes(s.into_bytes()).into())
        } else {
            let s = Wtf8Buf::from_wide(&normalized);
            Ok(vm.ctx.new_str(s).into())
        }
    }

    #[pyfunction]
    fn _getdiskusage(path: OsPath, vm: &VirtualMachine) -> PyResult<(u64, u64)> {
        use FileSystem::GetDiskFreeSpaceExW;

        let wpath = path.to_wide_cstring(vm)?;
        let mut _free_to_me: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        let ret =
            unsafe { GetDiskFreeSpaceExW(wpath.as_ptr(), &mut _free_to_me, &mut total, &mut free) };
        if ret != 0 {
            return Ok((total, free));
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(Foundation::ERROR_DIRECTORY as i32)
            && let Some(parent) = path.as_ref().parent()
        {
            let parent = widestring::WideCString::from_os_str(parent).unwrap();

            let ret = unsafe {
                GetDiskFreeSpaceExW(parent.as_ptr(), &mut _free_to_me, &mut total, &mut free)
            };

            return if ret == 0 {
                Err(err.to_pyexception(vm))
            } else {
                Ok((total, free))
            };
        }
        Err(err.to_pyexception(vm))
    }

    #[pyfunction]
    fn get_handle_inheritable(handle: intptr_t, vm: &VirtualMachine) -> PyResult<bool> {
        let mut flags = 0;
        if unsafe { Foundation::GetHandleInformation(handle as _, &mut flags) } == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(flags & Foundation::HANDLE_FLAG_INHERIT != 0)
    }

    #[pyfunction]
    fn get_inheritable(fd: i32, vm: &VirtualMachine) -> PyResult<bool> {
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| e.to_pyexception(vm))?;
        get_handle_inheritable(handle.as_raw_handle() as _, vm)
    }

    #[pyfunction]
    fn getlogin(vm: &VirtualMachine) -> PyResult<String> {
        let mut buffer = [0u16; 257];
        let mut size = buffer.len() as u32;

        let success = unsafe {
            windows_sys::Win32::System::WindowsProgramming::GetUserNameW(
                buffer.as_mut_ptr(),
                &mut size,
            )
        };

        if success != 0 {
            // Convert the buffer (which is UTF-16) to a Rust String
            let username = std::ffi::OsString::from_wide(&buffer[..(size - 1) as usize]);
            Ok(username.to_str().unwrap().to_string())
        } else {
            Err(vm.new_os_error(format!("Error code: {success}")))
        }
    }

    pub fn raw_set_handle_inheritable(handle: intptr_t, inheritable: bool) -> std::io::Result<()> {
        let flags = if inheritable {
            Foundation::HANDLE_FLAG_INHERIT
        } else {
            0
        };
        let res = unsafe {
            Foundation::SetHandleInformation(handle as _, Foundation::HANDLE_FLAG_INHERIT, flags)
        };
        if res == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn listdrives(vm: &VirtualMachine) -> PyResult<PyListRef> {
        use windows_sys::Win32::Foundation::ERROR_MORE_DATA;

        let mut buffer = [0u16; 256];
        let len =
            unsafe { FileSystem::GetLogicalDriveStringsW(buffer.len() as _, buffer.as_mut_ptr()) };
        if len == 0 {
            return Err(vm.new_last_os_error());
        }
        if len as usize >= buffer.len() {
            return Err(std::io::Error::from_raw_os_error(ERROR_MORE_DATA as _).to_pyexception(vm));
        }
        let drives: Vec<_> = buffer[..(len - 1) as usize]
            .split(|&c| c == 0)
            .map(|drive| vm.new_pyobj(String::from_utf16_lossy(drive)))
            .collect();
        Ok(vm.ctx.new_list(drives))
    }

    #[pyfunction]
    fn listvolumes(vm: &VirtualMachine) -> PyResult<PyListRef> {
        use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;

        let mut result = Vec::new();
        let mut buffer = [0u16; Foundation::MAX_PATH as usize + 1];

        let find = unsafe { FileSystem::FindFirstVolumeW(buffer.as_mut_ptr(), buffer.len() as _) };
        if find == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }

        loop {
            // Find the null terminator
            let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            let volume = String::from_utf16_lossy(&buffer[..len]);
            result.push(vm.new_pyobj(volume));

            let ret = unsafe {
                FileSystem::FindNextVolumeW(find, buffer.as_mut_ptr(), buffer.len() as _)
            };
            if ret == 0 {
                let err = io::Error::last_os_error();
                unsafe { FileSystem::FindVolumeClose(find) };
                if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    break;
                }
                return Err(err.to_pyexception(vm));
            }
        }

        Ok(vm.ctx.new_list(result))
    }

    #[pyfunction]
    fn listmounts(volume: OsPath, vm: &VirtualMachine) -> PyResult<PyListRef> {
        use windows_sys::Win32::Foundation::ERROR_MORE_DATA;

        let wide = volume.to_wide_cstring(vm)?;
        let mut buflen: u32 = Foundation::MAX_PATH + 1;
        let mut buffer: Vec<u16> = vec![0; buflen as usize];

        loop {
            let success = unsafe {
                FileSystem::GetVolumePathNamesForVolumeNameW(
                    wide.as_ptr(),
                    buffer.as_mut_ptr(),
                    buflen,
                    &mut buflen,
                )
            };
            if success != 0 {
                break;
            }
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_MORE_DATA as i32) {
                buffer.resize(buflen as usize, 0);
                continue;
            }
            return Err(err.to_pyexception(vm));
        }

        // Parse null-separated strings
        let mut result = Vec::new();
        let mut start = 0;
        for (i, &c) in buffer.iter().enumerate() {
            if c == 0 {
                if i > start {
                    let mount = String::from_utf16_lossy(&buffer[start..i]);
                    result.push(vm.new_pyobj(mount));
                }
                start = i + 1;
                if start < buffer.len() && buffer[start] == 0 {
                    break; // Double null = end
                }
            }
        }

        Ok(vm.ctx.new_list(result))
    }

    #[pyfunction]
    fn set_handle_inheritable(
        handle: intptr_t,
        inheritable: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        raw_set_handle_inheritable(handle, inheritable).map_err(|e| e.to_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct MkdirArgs<'a> {
        #[pyarg(any)]
        path: OsPath,
        #[pyarg(any, default = 0o777)]
        mode: i32,
        #[pyarg(flatten)]
        dir_fd: DirFd<'a, { _os::MKDIR_DIR_FD as usize }>,
    }

    #[pyfunction]
    fn mkdir(args: MkdirArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

        let [] = args.dir_fd.0;
        let wide = args.path.to_wide_cstring(vm)?;

        // special case: mode 0o700 sets a protected ACL
        let res = if args.mode == 0o700 {
            let mut sec_attr = SECURITY_ATTRIBUTES {
                nLength: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: core::ptr::null_mut(),
                bInheritHandle: 0,
            };
            // Set a discretionary ACL (D) that is protected (P) and includes
            // inheritable (OICI) entries that allow (A) full control (FA) to
            // SYSTEM (SY), Administrators (BA), and the owner (OW).
            let sddl: Vec<u16> = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)\0"
                .encode_utf16()
                .collect();
            let convert_result = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sec_attr.lpSecurityDescriptor,
                    core::ptr::null_mut(),
                )
            };
            if convert_result == 0 {
                return Err(vm.new_last_os_error());
            }
            let res =
                unsafe { FileSystem::CreateDirectoryW(wide.as_ptr(), &sec_attr as *const _ as _) };
            unsafe { LocalFree(sec_attr.lpSecurityDescriptor) };
            res
        } else {
            unsafe { FileSystem::CreateDirectoryW(wide.as_ptr(), core::ptr::null_mut()) }
        };

        if res == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(())
    }

    unsafe extern "C" {
        fn _umask(mask: i32) -> i32;
    }

    /// Close fd and convert error to PyException (PEP 446 cleanup)
    #[cold]
    fn close_fd_and_raise(fd: i32, err: std::io::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let _ = unsafe { crt_fd::Owned::from_raw(fd) };
        err.to_pyexception(vm)
    }

    #[pyfunction]
    fn umask(mask: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let result = unsafe { _umask(mask) };
        if result < 0 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(result)
        }
    }

    #[pyfunction]
    fn pipe(vm: &VirtualMachine) -> PyResult<(i32, i32)> {
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::System::Pipes::CreatePipe;

        let mut attr = SECURITY_ATTRIBUTES {
            nLength: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: core::ptr::null_mut(),
            bInheritHandle: 0,
        };

        let (read_handle, write_handle) = unsafe {
            let mut read = MaybeUninit::<isize>::uninit();
            let mut write = MaybeUninit::<isize>::uninit();
            let res = CreatePipe(
                read.as_mut_ptr() as *mut _,
                write.as_mut_ptr() as *mut _,
                &mut attr as *mut _,
                0,
            );
            if res == 0 {
                return Err(vm.new_last_os_error());
            }
            (read.assume_init(), write.assume_init())
        };

        // Convert handles to file descriptors
        // O_NOINHERIT = 0x80 (MSVC CRT)
        const O_NOINHERIT: i32 = 0x80;
        let read_fd = unsafe { libc::open_osfhandle(read_handle, O_NOINHERIT) };
        let write_fd = unsafe { libc::open_osfhandle(write_handle, libc::O_WRONLY | O_NOINHERIT) };

        if read_fd == -1 || write_fd == -1 {
            unsafe {
                Foundation::CloseHandle(read_handle as _);
                Foundation::CloseHandle(write_handle as _);
            }
            return Err(vm.new_last_os_error());
        }

        Ok((read_fd, write_fd))
    }

    #[pyfunction]
    fn getppid() -> u32 {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, PROCESS_BASIC_INFORMATION};

        type NtQueryInformationProcessFn = unsafe extern "system" fn(
            process_handle: isize,
            process_information_class: u32,
            process_information: *mut core::ffi::c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> i32;

        let ntdll = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(windows_sys::w!(
                "ntdll.dll"
            ))
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
                0, // ProcessBasicInformation
                &mut info as *mut _ as *mut core::ffi::c_void,
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

    #[pyfunction]
    fn dup(fd: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let fd2 = unsafe { suppress_iph!(libc::dup(fd)) };
        if fd2 < 0 {
            return Err(vm.new_last_errno_error());
        }
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd2) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| close_fd_and_raise(fd2, e, vm))?;
        raw_set_handle_inheritable(handle.as_raw_handle() as _, false)
            .map_err(|e| close_fd_and_raise(fd2, e, vm))?;
        Ok(fd2)
    }

    #[derive(FromArgs)]
    struct Dup2Args {
        #[pyarg(positional)]
        fd: i32,
        #[pyarg(positional)]
        fd2: i32,
        #[pyarg(any, default = true)]
        inheritable: bool,
    }

    #[pyfunction]
    fn dup2(args: Dup2Args, vm: &VirtualMachine) -> PyResult<i32> {
        let result = unsafe { suppress_iph!(libc::dup2(args.fd, args.fd2)) };
        if result < 0 {
            return Err(vm.new_last_errno_error());
        }
        if !args.inheritable {
            let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(args.fd2) };
            let handle =
                crt_fd::as_handle(borrowed).map_err(|e| close_fd_and_raise(args.fd2, e, vm))?;
            raw_set_handle_inheritable(handle.as_raw_handle() as _, false)
                .map_err(|e| close_fd_and_raise(args.fd2, e, vm))?;
        }
        Ok(args.fd2)
    }

    /// Windows-specific readlink that preserves \\?\ prefix for junctions
    /// returns the substitute name from reparse data which includes the prefix
    #[pyfunction]
    fn readlink(path: OsPath, vm: &VirtualMachine) -> PyResult {
        use crate::common::windows::ToWideString;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::IO::DeviceIoControl;
        use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;

        let mode = path.mode();
        let wide_path = path.as_ref().to_wide_with_nul();

        // Open the file/directory with reparse point flag
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                0, // No access needed, just reading reparse data
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                core::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                core::ptr::null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(OSErrorBuilder::with_filename(
                &io::Error::last_os_error(),
                path.clone(),
                vm,
            ));
        }

        // Buffer for reparse data - MAXIMUM_REPARSE_DATA_BUFFER_SIZE is 16384
        const BUFFER_SIZE: usize = 16384;
        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut bytes_returned: u32 = 0;

        let result = unsafe {
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

        if result == 0 {
            return Err(OSErrorBuilder::with_filename(
                &io::Error::last_os_error(),
                path.clone(),
                vm,
            ));
        }

        // Parse the reparse data buffer
        // REPARSE_DATA_BUFFER structure:
        // DWORD ReparseTag
        // WORD ReparseDataLength
        // WORD Reserved
        // For symlinks/junctions (IO_REPARSE_TAG_SYMLINK/MOUNT_POINT):
        // WORD SubstituteNameOffset
        // WORD SubstituteNameLength
        // WORD PrintNameOffset
        // WORD PrintNameLength
        // (For symlinks only: DWORD Flags)
        // PathBuffer...

        let reparse_tag = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);

        // Check if it's a symlink or mount point (junction)
        use windows_sys::Win32::System::SystemServices::{
            IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
        };

        let (substitute_offset, substitute_length, path_buffer_start) =
            if reparse_tag == IO_REPARSE_TAG_SYMLINK {
                // Symlink has Flags field (4 bytes) before PathBuffer
                let sub_offset = u16::from_le_bytes([buffer[8], buffer[9]]) as usize;
                let sub_length = u16::from_le_bytes([buffer[10], buffer[11]]) as usize;
                // PathBuffer starts at offset 20 (after Flags at offset 16)
                (sub_offset, sub_length, 20usize)
            } else if reparse_tag == IO_REPARSE_TAG_MOUNT_POINT {
                // Mount point (junction) has no Flags field
                let sub_offset = u16::from_le_bytes([buffer[8], buffer[9]]) as usize;
                let sub_length = u16::from_le_bytes([buffer[10], buffer[11]]) as usize;
                // PathBuffer starts at offset 16
                (sub_offset, sub_length, 16usize)
            } else {
                return Err(vm.new_value_error("not a symbolic link".to_owned()));
            };

        // Extract the substitute name
        let path_start = path_buffer_start + substitute_offset;
        let path_end = path_start + substitute_length;

        if path_end > buffer.len() {
            return Err(vm.new_os_error("Invalid reparse data".to_owned()));
        }

        // Convert from UTF-16LE
        let path_slice = &buffer[path_start..path_end];
        let wide_chars: Vec<u16> = path_slice
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let mut wide_chars = wide_chars;
        // For mount points (junctions), the substitute name typically starts with \??\
        // Convert this to \\?\
        if wide_chars.len() > 4
            && wide_chars[0] == b'\\' as u16
            && wide_chars[1] == b'?' as u16
            && wide_chars[2] == b'?' as u16
            && wide_chars[3] == b'\\' as u16
        {
            wide_chars[1] = b'\\' as u16;
        }

        let result_path = std::ffi::OsString::from_wide(&wide_chars);

        Ok(mode.process_path(std::path::PathBuf::from(result_path), vm))
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        Vec::new()
    }

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::os::module_exec(vm, module)?;
        Ok(())
    }
}

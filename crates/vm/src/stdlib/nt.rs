// spell-checker:disable

use crate::{PyRef, VirtualMachine, builtins::PyModule};

pub use module::raw_set_handle_inheritable;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = module::make_module(vm);
    super::os::extend_module(vm, &module);
    module
}

#[pymodule(name = "nt", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        PyResult, TryFromObject, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyDictRef, PyListRef, PyStrRef, PyTupleRef},
        common::{crt_fd, suppress_iph, windows::ToWideString},
        convert::ToPyException,
        function::{Either, OptionalArg},
        ospath::OsPath,
        stdlib::os::{_os, DirFd, FollowSymlinks, SupportFunc, TargetIsDirectory},
    };

    use libc::intptr_t;
    use std::os::windows::io::AsRawHandle;
    use std::{env, fs, io, mem::MaybeUninit, os::windows::ffi::OsStringExt};
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
    pub(super) fn _supports_virtual_terminal() -> PyResult<bool> {
        // TODO: implement this
        Ok(true)
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
        use std::os::windows::fs as win_fs;
        let dir = args.target_is_directory.target_is_directory
            || args
                .dst
                .as_path()
                .parent()
                .and_then(|dst_parent| dst_parent.join(&args.src).symlink_metadata().ok())
                .is_some_and(|meta| meta.is_dir());
        let res = if dir {
            win_fs::symlink_dir(args.src.path, args.dst.path)
        } else {
            win_fs::symlink_file(args.src.path, args.dst.path)
        };
        res.map_err(|err| err.to_pyexception(vm))
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
            // current directories. They cannot be modified via _wputenv() and should
            // not be exposed to Python code.
            if key.starts_with('=') {
                continue;
            }
            environ.set_item(&key, vm.new_pyobj(value), vm).unwrap();
        }
        environ
    }

    #[pyfunction]
    fn chmod(
        path: OsPath,
        dir_fd: DirFd<'_, 0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        const S_IWRITE: u32 = 128;
        let [] = dir_fd.0;
        let metadata = if follow_symlinks.0 {
            fs::metadata(&path)
        } else {
            fs::symlink_metadata(&path)
        };
        let meta = metadata.map_err(|err| err.to_pyexception(vm))?;
        let mut permissions = meta.permissions();
        permissions.set_readonly(mode & S_IWRITE == 0);
        fs::set_permissions(&path, permissions).map_err(|err| err.to_pyexception(vm))
    }

    // cwait is available on MSVC only (according to CPython)
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
                    std::ptr::null(),
                    FileSystem::OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
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
        use std::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.as_str())
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
            .chain(once(std::ptr::null()))
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
        use std::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.as_str())
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
            .chain(once(std::ptr::null()))
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
            // Validate: no '=' in key (search from index 1 because on Windows
            // starting '=' is allowed for defining hidden environment variables)
            if key_str.get(1..).is_some_and(|s| s.contains('=')) {
                return Err(vm.new_value_error("illegal environment variable name"));
            }

            let env_str = format!("{}={}", key_str, value_str);
            env_strings.push(make_widestring(&env_str)?);
        }

        let envp: Vec<*const u16> = env_strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(once(std::ptr::null()))
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
        use std::iter::once;

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
            .chain(once(std::ptr::null()))
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
        use std::iter::once;

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
            .chain(once(std::ptr::null()))
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
            // Validate: no '=' in key (search from index 1 because on Windows
            // starting '=' is allowed for defining hidden environment variables)
            if key_str.get(1..).is_some_and(|s| s.contains('=')) {
                return Err(vm.new_value_error("illegal environment variable name"));
            }

            let env_str = format!("{}={}", key_str, value_str);
            env_strings.push(make_widestring(&env_str)?);
        }

        let envp: Vec<*const u16> = env_strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(once(std::ptr::null()))
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
        let real = path
            .as_ref()
            .canonicalize()
            .map_err(|e| e.to_pyexception(vm))?;
        Ok(path.mode.process_path(real, vm))
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
                std::ptr::null_mut(),
            )
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }
        if ret as usize > buffer.len() {
            buffer.resize(ret as usize, 0);
            let ret = unsafe {
                FileSystem::GetFullPathNameW(
                    wpath.as_ptr(),
                    buffer.len() as _,
                    buffer.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            };
            if ret == 0 {
                return Err(vm.new_last_os_error());
            }
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        Ok(path.mode.process_path(buffer.to_os_string(), vm))
    }

    #[pyfunction]
    fn _getvolumepathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wide = path.to_wide_cstring(vm)?;
        let buflen = std::cmp::max(wide.len(), Foundation::MAX_PATH as usize);
        let mut buffer = vec![0u16; buflen];
        let ret = unsafe {
            FileSystem::GetVolumePathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buflen as _)
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        Ok(path.mode.process_path(buffer.to_os_string(), vm))
    }

    #[pyfunction]
    fn _path_splitroot(path: OsPath, vm: &VirtualMachine) -> PyResult<(String, String)> {
        let orig: Vec<_> = path.path.to_wide();
        if orig.is_empty() {
            return Ok(("".to_owned(), "".to_owned()));
        }
        let backslashed: Vec<_> = orig
            .iter()
            .copied()
            .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
            .chain(std::iter::once(0)) // null-terminated
            .collect();

        fn from_utf16(wstr: &[u16], vm: &VirtualMachine) -> PyResult<String> {
            String::from_utf16(wstr).map_err(|e| vm.new_unicode_decode_error(e.to_string()))
        }

        let mut end: *const u16 = std::ptr::null();
        let hr = unsafe {
            windows_sys::Win32::UI::Shell::PathCchSkipRoot(backslashed.as_ptr(), &mut end)
        };
        let (root, path) = if hr == 0 {
            // S_OK
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
            (from_utf16(&orig[..len], vm)?, from_utf16(&orig[len..], vm)?)
        } else {
            ("".to_owned(), from_utf16(&orig, vm)?)
        };
        Ok((root, path))
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

        // CPython special case: mode 0o700 sets a protected ACL
        let res = if args.mode == 0o700 {
            let mut sec_attr = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
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
                    std::ptr::null_mut(),
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
            unsafe { FileSystem::CreateDirectoryW(wide.as_ptr(), std::ptr::null_mut()) }
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
        use windows_sys::Win32::System::Pipes::CreatePipe;

        let (read_handle, write_handle) = unsafe {
            let mut read = MaybeUninit::<isize>::uninit();
            let mut write = MaybeUninit::<isize>::uninit();
            let res = CreatePipe(
                read.as_mut_ptr() as *mut _,
                write.as_mut_ptr() as *mut _,
                std::ptr::null(),
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
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        #[repr(C)]
        struct ProcessBasicInformation {
            exit_status: isize,
            peb_base_address: *mut std::ffi::c_void,
            affinity_mask: usize,
            base_priority: i32,
            unique_process_id: usize,
            inherited_from_unique_process_id: usize,
        }

        type NtQueryInformationProcessFn = unsafe extern "system" fn(
            process_handle: isize,
            process_information_class: u32,
            process_information: *mut std::ffi::c_void,
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
        let nt_query: NtQueryInformationProcessFn = unsafe { std::mem::transmute(func) };

        let mut info = ProcessBasicInformation {
            exit_status: 0,
            peb_base_address: std::ptr::null_mut(),
            affinity_mask: 0,
            base_priority: 0,
            unique_process_id: 0,
            inherited_from_unique_process_id: 0,
        };

        let status = unsafe {
            nt_query(
                GetCurrentProcess() as isize,
                0, // ProcessBasicInformation
                &mut info as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<ProcessBasicInformation>() as u32,
                std::ptr::null_mut(),
            )
        };

        if status >= 0
            && info.inherited_from_unique_process_id != 0
            && info.inherited_from_unique_process_id < u32::MAX as usize
        {
            info.inherited_from_unique_process_id as u32
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

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        Vec::new()
    }
}

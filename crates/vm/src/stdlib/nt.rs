// spell-checker:disable

pub(crate) use module::module_def;
pub use module::raw_set_handle_inheritable;

#[pymodule(name = "nt", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        Py, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyBytes, PyDictRef, PyListRef, PyStr, PyStrRef, PyTupleRef},
        convert::ToPyException,
        exceptions::OSErrorBuilder,
        function::{ArgMapping, Either, OptionalArg},
        host_env::{crt_fd, windows::ToWideString},
        ospath::{OsPath, OsPathOrFd},
        stdlib::os::{_os, DirFd, SupportFunc, TargetIsDirectory},
    };
    use libc::intptr_t;
    use rustpython_common::wtf8::Wtf8Buf;
    use rustpython_host_env::nt as host_nt;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;

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
    use host_nt::{
        LOAD_LIBRARY_SEARCH_APPLICATION_DIR as _LOAD_LIBRARY_SEARCH_APPLICATION_DIR,
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS as _LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR as _LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32 as _LOAD_LIBRARY_SEARCH_SYSTEM32,
        LOAD_LIBRARY_SEARCH_USER_DIRS as _LOAD_LIBRARY_SEARCH_USER_DIRS,
    };

    #[pyfunction]
    pub(super) fn access(path: OsPath, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        let _ = path.to_wide_cstring(vm)?;
        Ok(host_nt::access(path.as_ref(), mode))
    }

    #[pyfunction]
    #[pyfunction(name = "unlink")]
    pub(super) fn remove(
        path: OsPath,
        dir_fd: DirFd<'static, 0>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let [] = dir_fd.0;
        let _ = path.to_wide_cstring(vm)?;
        host_nt::remove(path.as_ref()).map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
    }

    #[pyfunction]
    pub(super) fn _supports_virtual_terminal() -> PyResult<bool> {
        Ok(host_nt::supports_virtual_terminal())
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
        let src = args.src.to_wide_cstring(vm)?;
        let dst = args.dst.to_wide_cstring(vm)?;
        if let Err(err) = host_nt::symlink(
            args.src.as_ref(),
            args.dst.as_ref(),
            &src,
            &dst,
            args.target_is_directory.target_is_directory,
        ) {
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

        for (key, value) in host_nt::visible_env_vars() {
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
        for (key, value) in host_nt::visible_env_vars() {
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

    fn fchmod_impl(fd: i32, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        host_nt::fchmod(fd, mode, S_IWRITE).map_err(|e| e.to_pyexception(vm))
    }

    fn win32_lchmod(path: &OsPath, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        host_nt::win32_lchmod(path.path.as_os_str(), mode, S_IWRITE)
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))
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
                return Err(
                    vm.new_value_error("chmod: follow_symlinks is not supported with fd argument")
                );
            }
            return fchmod_impl(fd.as_raw(), mode, vm);
        }

        let OsPathOrFd::Path(path) = path else {
            unreachable!()
        };

        let follow_symlinks = follow_symlinks.into_option().unwrap_or(false);

        if follow_symlinks {
            let wide = path.to_wide_cstring(vm)?;
            host_nt::chmod_follow(&wide, mode, S_IWRITE)
                .map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
        } else {
            win32_lchmod(&path, mode, vm)
        }
    }

    /// Get the real file name (with correct case) without accessing the file.
    /// Uses FindFirstFileW to get the name as stored on the filesystem.
    #[pyfunction]
    fn _findfirstfile(path: OsPath, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let filename = host_nt::find_first_file_name(path.as_ref())
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
        let filename_str = filename
            .to_str()
            .ok_or_else(|| vm.new_unicode_decode_error("filename contains invalid UTF-8"))?;

        Ok(vm.ctx.new_str(filename_str))
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
        let tested_type = match tested_type {
            PY_IFREG => host_nt::TestType::RegularFile,
            PY_IFDIR => host_nt::TestType::Directory,
            PY_IFLNK => host_nt::TestType::Symlink,
            PY_IFMNT => host_nt::TestType::Junction,
            PY_IFLRP => host_nt::TestType::LinkReparsePoint,
            PY_IFRRP => host_nt::TestType::RegularReparsePoint,
            _ => return false,
        };
        host_nt::test_info(attributes, reparse_tag, disk_device, tested_type)
    }

    /// _testFileTypeByHandle - test file type using an open handle
    fn _test_file_type_by_handle(
        handle: host_nt::Handle,
        tested_type: u32,
        disk_only: bool,
    ) -> bool {
        let tested_type = match tested_type {
            PY_IFREG => host_nt::TestType::RegularFile,
            PY_IFDIR => host_nt::TestType::Directory,
            PY_IFLNK => host_nt::TestType::Symlink,
            PY_IFMNT => host_nt::TestType::Junction,
            PY_IFLRP => host_nt::TestType::LinkReparsePoint,
            PY_IFRRP => host_nt::TestType::RegularReparsePoint,
            _ => return false,
        };
        host_nt::test_file_type_by_handle(handle, tested_type, disk_only)
    }

    /// _testFileTypeByName - test file type by path name
    fn _test_file_type_by_name(path: &std::path::Path, tested_type: u32) -> bool {
        let tested_type = match tested_type {
            PY_IFREG => host_nt::TestType::RegularFile,
            PY_IFDIR => host_nt::TestType::Directory,
            PY_IFLNK => host_nt::TestType::Symlink,
            PY_IFMNT => host_nt::TestType::Junction,
            PY_IFLRP => host_nt::TestType::LinkReparsePoint,
            PY_IFRRP => host_nt::TestType::RegularReparsePoint,
            _ => return false,
        };
        host_nt::test_file_type_by_name(path, tested_type)
    }

    /// _testFileExistsByName - test if path exists
    fn _test_file_exists_by_name(path: &std::path::Path, follow_links: bool) -> bool {
        host_nt::test_file_exists_by_name(path, follow_links)
    }

    /// _testFileType wrapper - handles both fd and path
    fn _test_file_type(path_or_fd: &OsPathOrFd<'_>, tested_type: u32) -> bool {
        match path_or_fd {
            OsPathOrFd::Fd(fd) => {
                if let Ok(handle) = crate::host_env::crt_fd::as_handle(*fd) {
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
        match path_or_fd {
            OsPathOrFd::Fd(fd) => host_nt::fd_exists(*fd),
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
        let _ = path.to_wide_cstring(vm)?;
        host_nt::path_isdevdrive(path.as_ref()).map_err(|err| err.to_pyexception(vm))
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn waitpid(pid: intptr_t, opt: i32, vm: &VirtualMachine) -> PyResult<(intptr_t, u64)> {
        let (pid, status) = host_nt::cwait(pid, opt).map_err(|_| vm.new_last_errno_error())?;
        // Cast to unsigned to handle large exit codes (like 0xC000013A)
        // then shift left by 8 to match POSIX waitpid format
        let ustatus = (status as u32) as u64;
        Ok((pid, ustatus << 8))
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(intptr_t, u64)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_nt::kill(pid as u32, sig as u32).map_err(|err| err.to_pyexception(vm))
    }

    #[pyfunction]
    fn get_terminal_size(
        fd: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<_os::TerminalSizeData> {
        let fd = fd.unwrap_or(1); // default to stdout
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| e.to_pyexception(vm))?;
        let (columns, lines) = host_nt::get_terminal_size_handle(handle.as_raw_handle() as _)
            .map_err(|_| vm.new_last_os_error())?;
        Ok(_os::TerminalSizeData { columns, lines })
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

        host_nt::spawnv(mode, path.as_ptr(), argv_spawn.as_ptr())
            .map_err(|_| vm.new_last_errno_error())
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

        host_nt::spawnve(mode, path.as_ptr(), argv_spawn.as_ptr(), envp.as_ptr())
            .map_err(|_| vm.new_last_errno_error())
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
            make_widestring(arg.expect_str())
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

        host_nt::execv(path.as_ptr(), argv_execv.as_ptr()).map_err(|_| vm.new_last_errno_error())
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn execve(
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        env: ArgMapping,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use core::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = path.to_wide_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.expect_str())
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

        let env = crate::stdlib::os::envobj_to_dict(env, vm)?;
        // Build environment strings as "KEY=VALUE\0" wide strings
        let mut env_strings: Vec<widestring::WideCString> = Vec::new();
        for (key, value) in env.into_iter() {
            let key = PyStrRef::try_from_object(vm, key)?;
            let value = PyStrRef::try_from_object(vm, value)?;
            let key_str = key.expect_str();
            let value_str = value.expect_str();

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

        host_nt::execve(path.as_ptr(), argv_execve.as_ptr(), envp.as_ptr())
            .map_err(|_| vm.new_last_errno_error())
    }

    #[pyfunction]
    fn _getfinalpathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let _ = path.to_wide_cstring(vm)?;
        let final_path = host_nt::getfinalpathname(path.as_ref())
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
        Ok(path.mode().process_path(final_path, vm))
    }

    #[pyfunction]
    fn _getfullpathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let _ = path.to_wide_cstring(vm)?;
        let buffer = host_nt::getfullpathname(path.as_ref())
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
        Ok(path.mode().process_path(buffer, vm))
    }

    #[pyfunction]
    fn _getvolumepathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wide = path.to_wide_cstring(vm)?;
        let buflen = core::cmp::max(wide.len(), host_nt::MAX_PATH_USIZE);
        if buflen > u32::MAX as usize {
            return Err(vm.new_overflow_error("path too long"));
        }
        let buffer = host_nt::getvolumepathname(path.as_ref())
            .map_err(|err| OSErrorBuilder::with_filename(&err, path.clone(), vm))?;
        Ok(path.mode().process_path(buffer, vm))
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
                    )
                    .into(),
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
    fn _path_splitroot(path: OsPath, _vm: &VirtualMachine) -> (Wtf8Buf, Wtf8Buf) {
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

        if let Some(len) = host_nt::path_skip_root(backslashed.as_ptr()) {
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
                    )
                    .into(),
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
        let _ = path.to_wide_cstring(vm)?;
        host_nt::getdiskusage(path.as_ref()).map_err(|err| err.to_pyexception(vm))
    }

    #[pyfunction]
    fn get_handle_inheritable(handle: intptr_t, vm: &VirtualMachine) -> PyResult<bool> {
        host_nt::get_handle_inheritable(handle).map_err(|err| err.to_pyexception(vm))
    }

    #[pyfunction]
    fn get_inheritable(fd: i32, vm: &VirtualMachine) -> PyResult<bool> {
        let borrowed = unsafe { crt_fd::Borrowed::borrow_raw(fd) };
        let handle = crt_fd::as_handle(borrowed).map_err(|e| e.to_pyexception(vm))?;
        get_handle_inheritable(handle.as_raw_handle() as _, vm)
    }

    #[pyfunction]
    fn getlogin(vm: &VirtualMachine) -> PyResult<String> {
        host_nt::getlogin().map_err(|_| vm.new_os_error("Error code: 0".to_owned()))
    }

    pub fn raw_set_handle_inheritable(handle: intptr_t, inheritable: bool) -> std::io::Result<()> {
        host_nt::set_handle_inheritable(handle, inheritable)
    }

    #[pyfunction]
    fn listdrives(vm: &VirtualMachine) -> PyResult<PyListRef> {
        let drives: Vec<_> = host_nt::listdrives()
            .map_err(|err| err.to_pyexception(vm))?
            .into_iter()
            .map(|drive| vm.new_pyobj(drive.to_string_lossy().into_owned()))
            .collect();
        Ok(vm.ctx.new_list(drives))
    }

    #[pyfunction]
    fn listvolumes(vm: &VirtualMachine) -> PyResult<PyListRef> {
        let result = host_nt::listvolumes()
            .map_err(|err| err.to_pyexception(vm))?
            .into_iter()
            .map(|volume| vm.new_pyobj(volume.to_string_lossy().into_owned()))
            .collect();
        Ok(vm.ctx.new_list(result))
    }

    #[pyfunction]
    fn listmounts(volume: OsPath, vm: &VirtualMachine) -> PyResult<PyListRef> {
        let _ = volume.to_wide_cstring(vm)?;
        let result = host_nt::listmounts(volume.as_ref())
            .map_err(|err| err.to_pyexception(vm))?
            .into_iter()
            .map(|mount| vm.new_pyobj(mount.to_string_lossy().into_owned()))
            .collect();
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
        let [] = args.dir_fd.0;
        let wide = args.path.to_wide_cstring(vm)?;
        host_nt::mkdir(&wide, args.mode).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn umask(mask: i32, vm: &VirtualMachine) -> PyResult<i32> {
        host_nt::umask(mask).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn pipe(vm: &VirtualMachine) -> PyResult<(i32, i32)> {
        host_nt::pipe().map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn getppid() -> u32 {
        host_nt::getppid()
    }

    #[pyfunction]
    fn dup(fd: i32, vm: &VirtualMachine) -> PyResult<i32> {
        host_nt::dup(fd).map_err(|e| e.to_pyexception(vm))
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
        host_nt::dup2(args.fd, args.fd2, args.inheritable).map_err(|e| e.to_pyexception(vm))
    }

    /// Windows-specific readlink that preserves \\?\ prefix for junctions
    /// returns the substitute name from reparse data which includes the prefix
    #[pyfunction]
    fn readlink(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let mode = path.mode();
        match host_nt::readlink(path.as_ref()) {
            Ok(result_path) => Ok(mode.process_path(std::path::PathBuf::from(result_path), vm)),
            Err(host_nt::ReadlinkError::Io(err)) => {
                Err(OSErrorBuilder::with_filename(&err, path.clone(), vm))
            }
            Err(host_nt::ReadlinkError::NotSymbolicLink) => {
                Err(vm.new_value_error("not a symbolic link"))
            }
            Err(host_nt::ReadlinkError::InvalidReparseData) => {
                Err(vm.new_os_error("Invalid reparse data".to_owned()))
            }
        }
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

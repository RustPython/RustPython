use crate::{builtins::PyModule, PyRef, VirtualMachine};

pub use module::raw_set_handle_inheritable;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = module::make_module(vm);
    super::os::extend_module(vm, &module);
    module
}

#[pymodule(name = "nt", with(super::os::_os))]
pub(crate) mod module {
    use crate::{
        builtins::{PyStrRef, PyTupleRef},
        common::{crt_fd::Fd, os::errno, suppress_iph},
        convert::ToPyException,
        function::Either,
        function::OptionalArg,
        stdlib::os::{
            errno_err, DirFd, FollowSymlinks, OsPath, SupportFunc, TargetIsDirectory, _os,
        },
        PyResult, TryFromObject, VirtualMachine,
    };
    use std::{
        env, fs, io,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };

    use crate::builtins::PyDictRef;
    #[cfg(target_env = "msvc")]
    use crate::builtins::PyListRef;
    use winapi::{um, vc::vcruntime::intptr_t};

    #[pyattr]
    use libc::{O_BINARY, O_TEMPORARY};

    #[pyfunction]
    pub(super) fn access(path: OsPath, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        use um::{fileapi, winnt};
        let attr = unsafe { fileapi::GetFileAttributesW(path.to_widecstring(vm)?.as_ptr()) };
        Ok(attr != fileapi::INVALID_FILE_ATTRIBUTES
            && (mode & 2 == 0
                || attr & winnt::FILE_ATTRIBUTE_READONLY == 0
                || attr & winnt::FILE_ATTRIBUTE_DIRECTORY != 0))
    }

    #[derive(FromArgs)]
    pub(super) struct SymlinkArgs {
        src: OsPath,
        dst: OsPath,
        #[pyarg(flatten)]
        target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        _dir_fd: DirFd<{ _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(args: SymlinkArgs, vm: &VirtualMachine) -> PyResult<()> {
        use std::os::windows::fs as win_fs;
        let dir = args.target_is_directory.target_is_directory
            || args
                .dst
                .as_path()
                .parent()
                .and_then(|dst_parent| dst_parent.join(&args.src).symlink_metadata().ok())
                .map_or(false, |meta| meta.is_dir());
        let res = if dir {
            win_fs::symlink_dir(args.src.path, args.dst.path)
        } else {
            win_fs::symlink_file(args.src.path, args.dst.path)
        };
        res.map_err(|err| err.to_pyexception(vm))
    }

    #[pyfunction]
    fn set_inheritable(fd: i32, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
        let handle = Fd(fd).to_raw_handle().map_err(|e| e.to_pyexception(vm))?;
        set_handle_inheritable(handle as _, inheritable, vm)
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        let environ = vm.ctx.new_dict();

        for (key, value) in env::vars() {
            environ.set_item(&key, vm.new_pyobj(value), vm).unwrap();
        }
        environ
    }

    #[pyfunction]
    fn chmod(
        path: OsPath,
        dir_fd: DirFd<0>,
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
    extern "C" {
        fn _cwait(termstat: *mut i32, procHandle: intptr_t, action: i32) -> intptr_t;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn waitpid(pid: intptr_t, opt: i32, vm: &VirtualMachine) -> PyResult<(intptr_t, i32)> {
        let mut status = 0;
        let pid = unsafe { suppress_iph!(_cwait(&mut status, pid, opt)) };
        if pid == -1 {
            Err(errno_err(vm))
        } else {
            Ok((pid, status << 8))
        }
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(intptr_t, i32)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        {
            use um::{handleapi, processthreadsapi, wincon, winnt};
            let sig = sig as u32;
            let pid = pid as u32;

            if sig == wincon::CTRL_C_EVENT || sig == wincon::CTRL_BREAK_EVENT {
                let ret = unsafe { wincon::GenerateConsoleCtrlEvent(sig, pid) };
                let res = if ret == 0 { Err(errno_err(vm)) } else { Ok(()) };
                return res;
            }

            let h = unsafe { processthreadsapi::OpenProcess(winnt::PROCESS_ALL_ACCESS, 0, pid) };
            if h.is_null() {
                return Err(errno_err(vm));
            }
            let ret = unsafe { processthreadsapi::TerminateProcess(h, sig) };
            let res = if ret == 0 { Err(errno_err(vm)) } else { Ok(()) };
            unsafe { handleapi::CloseHandle(h) };
            res
        }
    }

    #[pyfunction]
    fn get_terminal_size(
        fd: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<_os::PyTerminalSize> {
        let (columns, lines) = {
            use um::{handleapi, processenv, winbase, wincon};
            let stdhandle = match fd {
                OptionalArg::Present(0) => winbase::STD_INPUT_HANDLE,
                OptionalArg::Present(1) | OptionalArg::Missing => winbase::STD_OUTPUT_HANDLE,
                OptionalArg::Present(2) => winbase::STD_ERROR_HANDLE,
                _ => return Err(vm.new_value_error("bad file descriptor".to_owned())),
            };
            let h = unsafe { processenv::GetStdHandle(stdhandle) };
            if h.is_null() {
                return Err(vm.new_os_error("handle cannot be retrieved".to_owned()));
            }
            if h == handleapi::INVALID_HANDLE_VALUE {
                return Err(errno_err(vm));
            }
            let mut csbi = wincon::CONSOLE_SCREEN_BUFFER_INFO::default();
            let ret = unsafe { wincon::GetConsoleScreenBufferInfo(h, &mut csbi) };
            if ret == 0 {
                return Err(errno_err(vm));
            }
            let w = csbi.srWindow;
            (
                (w.Right - w.Left + 1) as usize,
                (w.Bottom - w.Top + 1) as usize,
            )
        };
        Ok(_os::PyTerminalSize { columns, lines })
    }

    #[cfg(target_env = "msvc")]
    extern "C" {
        fn _wexecv(cmdname: *const u16, argv: *const *const u16) -> intptr_t;
    }

    #[cfg(target_env = "msvc")]
    #[pyfunction]
    fn execv(
        path: PyStrRef,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use std::iter::once;

        let make_widestring =
            |s: &str| widestring::WideCString::from_os_str(s).map_err(|err| err.to_pyexception(vm));

        let path = make_widestring(path.as_str())?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            let arg = PyStrRef::try_from_object(vm, obj)?;
            make_widestring(arg.as_str())
        })?;

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty".to_owned()))?;

        if first.is_empty() {
            return Err(
                vm.new_value_error("execv() arg 2 first element cannot be empty".to_owned())
            );
        }

        let argv_execv: Vec<*const u16> = argv
            .iter()
            .map(|v| v.as_ptr())
            .chain(once(std::ptr::null()))
            .collect();

        if (unsafe { suppress_iph!(_wexecv(path.as_ptr(), argv_execv.as_ptr())) } == -1) {
            Err(errno_err(vm))
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
        path.mode.process_path(real, vm)
    }

    #[pyfunction]
    fn _getfullpathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wpath = path.to_widecstring(vm)?;
        let mut buffer = vec![0u16; winapi::shared::minwindef::MAX_PATH];
        let ret = unsafe {
            um::fileapi::GetFullPathNameW(
                wpath.as_ptr(),
                buffer.len() as _,
                buffer.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if ret == 0 {
            return Err(errno_err(vm));
        }
        if ret as usize > buffer.len() {
            buffer.resize(ret as usize, 0);
            let ret = unsafe {
                um::fileapi::GetFullPathNameW(
                    wpath.as_ptr(),
                    buffer.len() as _,
                    buffer.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            };
            if ret == 0 {
                return Err(errno_err(vm));
            }
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        path.mode.process_path(buffer.to_os_string(), vm)
    }

    #[pyfunction]
    fn _getvolumepathname(path: OsPath, vm: &VirtualMachine) -> PyResult {
        let wide = path.to_widecstring(vm)?;
        let buflen = std::cmp::max(wide.len(), winapi::shared::minwindef::MAX_PATH);
        let mut buffer = vec![0u16; buflen];
        let ret = unsafe {
            um::fileapi::GetVolumePathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buflen as _)
        };
        if ret == 0 {
            return Err(errno_err(vm));
        }
        let buffer = widestring::WideCString::from_vec_truncate(buffer);
        path.mode.process_path(buffer.to_os_string(), vm)
    }

    #[pyfunction]
    fn _path_splitroot(path: OsPath, vm: &VirtualMachine) -> PyResult<(String, String)> {
        let orig: Vec<_> = path.path.encode_wide().collect();
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

        let wbuf = windows::core::PCWSTR::from_raw(backslashed.as_ptr());
        let (root, path) = match unsafe { windows::Win32::UI::Shell::PathCchSkipRoot(wbuf) } {
            Ok(end) => {
                assert!(!end.is_null());
                let len: usize = unsafe { end.as_ptr().offset_from(wbuf.as_ptr()) }
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
            }
            Err(_) => ("".to_owned(), from_utf16(&orig, vm)?),
        };
        Ok((root, path))
    }

    #[pyfunction]
    fn _getdiskusage(path: OsPath, vm: &VirtualMachine) -> PyResult<(u64, u64)> {
        use um::fileapi::GetDiskFreeSpaceExW;
        use winapi::shared::{ntdef::ULARGE_INTEGER, winerror};

        let wpath = path.to_widecstring(vm)?;
        let mut _free_to_me = ULARGE_INTEGER::default();
        let mut total = ULARGE_INTEGER::default();
        let mut free = ULARGE_INTEGER::default();
        let ret =
            unsafe { GetDiskFreeSpaceExW(wpath.as_ptr(), &mut _free_to_me, &mut total, &mut free) };
        if ret != 0 {
            return Ok(unsafe { (*total.QuadPart(), *free.QuadPart()) });
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(winerror::ERROR_DIRECTORY as i32) {
            if let Some(parent) = path.as_ref().parent() {
                let parent = widestring::WideCString::from_os_str(parent).unwrap();

                let ret = unsafe {
                    GetDiskFreeSpaceExW(parent.as_ptr(), &mut _free_to_me, &mut total, &mut free)
                };

                return if ret == 0 {
                    Err(errno_err(vm))
                } else {
                    Ok(unsafe { (*total.QuadPart(), *free.QuadPart()) })
                };
            }
        }
        Err(err.to_pyexception(vm))
    }

    #[pyfunction]
    fn get_handle_inheritable(handle: intptr_t, vm: &VirtualMachine) -> PyResult<bool> {
        let mut flags = 0;
        if unsafe { um::handleapi::GetHandleInformation(handle as _, &mut flags) } == 0 {
            Err(errno_err(vm))
        } else {
            Ok(flags & um::winbase::HANDLE_FLAG_INHERIT != 0)
        }
    }

    pub fn raw_set_handle_inheritable(handle: intptr_t, inheritable: bool) -> io::Result<()> {
        use um::winbase::HANDLE_FLAG_INHERIT;
        let flags = if inheritable { HANDLE_FLAG_INHERIT } else { 0 };
        let res =
            unsafe { um::handleapi::SetHandleInformation(handle as _, HANDLE_FLAG_INHERIT, flags) };
        if res == 0 {
            Err(errno())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn set_handle_inheritable(
        handle: intptr_t,
        inheritable: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        raw_set_handle_inheritable(handle, inheritable).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn mkdir(
        path: OsPath,
        mode: OptionalArg<i32>,
        dir_fd: DirFd<{ _os::MKDIR_DIR_FD as usize }>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mode = mode.unwrap_or(0o777);
        let [] = dir_fd.0;
        let _ = mode;
        let wide = path.to_widecstring(vm)?;
        let res = unsafe { um::fileapi::CreateDirectoryW(wide.as_ptr(), std::ptr::null_mut()) };
        if res == 0 {
            return Err(errno_err(vm));
        }
        Ok(())
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        Vec::new()
    }
}

pub fn init_winsock() {
    static WSA_INIT: parking_lot::Once = parking_lot::Once::new();
    WSA_INIT.call_once(|| unsafe {
        let mut wsa_data = std::mem::MaybeUninit::uninit();
        let _ = winapi::um::winsock2::WSAStartup(0x0101, wsa_data.as_mut_ptr());
    })
}

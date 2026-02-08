// spell-checker:disable

#![allow(non_snake_case)]
pub(crate) use _winapi::module_def;

#[pymodule]
mod _winapi {
    use crate::{
        Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
        builtins::PyStrRef,
        common::{lock::PyMutex, windows::ToWideString},
        convert::{ToPyException, ToPyResult},
        function::{ArgMapping, ArgSequence, OptionalArg},
        types::Constructor,
        windows::{WinHandle, WindowsSysResult},
    };
    use core::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, MAX_PATH};

    #[pyattr]
    use windows_sys::Win32::{
        Foundation::{
            DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE,
            ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NETNAME_DELETED, ERROR_NO_DATA,
            ERROR_NO_SYSTEM_RESOURCES, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
            ERROR_PIPE_CONNECTED, ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE, STILL_ACTIVE,
            WAIT_ABANDONED, WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Globalization::{
            LCMAP_FULLWIDTH, LCMAP_HALFWIDTH, LCMAP_HIRAGANA, LCMAP_KATAKANA,
            LCMAP_LINGUISTIC_CASING, LCMAP_LOWERCASE, LCMAP_SIMPLIFIED_CHINESE, LCMAP_TITLECASE,
            LCMAP_TRADITIONAL_CHINESE, LCMAP_UPPERCASE,
        },
        Storage::FileSystem::{
            COPY_FILE_ALLOW_DECRYPTED_DESTINATION,
            COPY_FILE_COPY_SYMLINK,
            COPY_FILE_FAIL_IF_EXISTS,
            COPY_FILE_NO_BUFFERING,
            COPY_FILE_NO_OFFLOAD,
            COPY_FILE_OPEN_SOURCE_FOR_WRITE,
            COPY_FILE_REQUEST_COMPRESSED_TRAFFIC,
            COPY_FILE_REQUEST_SECURITY_PRIVILEGES,
            COPY_FILE_RESTARTABLE,
            COPY_FILE_RESUME_FROM_PAUSE,
            COPYFILE2_CALLBACK_CHUNK_FINISHED,
            COPYFILE2_CALLBACK_CHUNK_STARTED,
            COPYFILE2_CALLBACK_ERROR,
            COPYFILE2_CALLBACK_POLL_CONTINUE,
            COPYFILE2_CALLBACK_STREAM_FINISHED,
            COPYFILE2_CALLBACK_STREAM_STARTED,
            COPYFILE2_PROGRESS_CANCEL,
            COPYFILE2_PROGRESS_CONTINUE,
            COPYFILE2_PROGRESS_PAUSE,
            COPYFILE2_PROGRESS_QUIET,
            COPYFILE2_PROGRESS_STOP,
            CREATE_ALWAYS,
            // CreateFile constants
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_DELETE_ON_CLOSE,
            FILE_FLAG_FIRST_PIPE_INSTANCE,
            FILE_FLAG_NO_BUFFERING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_FLAG_OVERLAPPED,
            FILE_FLAG_POSIX_SEMANTICS,
            FILE_FLAG_RANDOM_ACCESS,
            FILE_FLAG_SEQUENTIAL_SCAN,
            FILE_FLAG_WRITE_THROUGH,
            FILE_GENERIC_READ,
            FILE_GENERIC_WRITE,
            FILE_SHARE_DELETE,
            FILE_SHARE_READ,
            FILE_SHARE_WRITE,
            FILE_TYPE_CHAR,
            FILE_TYPE_DISK,
            FILE_TYPE_PIPE,
            FILE_TYPE_REMOTE,
            FILE_TYPE_UNKNOWN,
            OPEN_ALWAYS,
            OPEN_EXISTING,
            PIPE_ACCESS_DUPLEX,
            PIPE_ACCESS_INBOUND,
            SYNCHRONIZE,
            TRUNCATE_EXISTING,
        },
        System::{
            Console::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
            Memory::{
                FILE_MAP_ALL_ACCESS, MEM_COMMIT, MEM_FREE, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE,
                MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
                PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_NOCACHE, PAGE_READONLY,
                PAGE_READWRITE, PAGE_WRITECOMBINE, PAGE_WRITECOPY, SEC_COMMIT, SEC_IMAGE,
                SEC_LARGE_PAGES, SEC_NOCACHE, SEC_RESERVE, SEC_WRITECOMBINE,
            },
            Pipes::{
                NMPWAIT_NOWAIT, NMPWAIT_USE_DEFAULT_WAIT, NMPWAIT_WAIT_FOREVER,
                PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            SystemServices::LOCALE_NAME_MAX_LENGTH,
            Threading::{
                ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
                CREATE_BREAKAWAY_FROM_JOB, CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE,
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, HIGH_PRIORITY_CLASS,
                IDLE_PRIORITY_CLASS, INFINITE, NORMAL_PRIORITY_CLASS, PROCESS_DUP_HANDLE,
                REALTIME_PRIORITY_CLASS, STARTF_FORCEOFFFEEDBACK, STARTF_FORCEONFEEDBACK,
                STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
            },
        },
        UI::WindowsAndMessaging::SW_HIDE,
    };

    #[pyattr]
    const NULL: isize = 0;

    #[pyfunction]
    fn CloseHandle(handle: WinHandle) -> WindowsSysResult<i32> {
        WindowsSysResult(unsafe { windows_sys::Win32::Foundation::CloseHandle(handle.0) })
    }

    /// CreateFile - Create or open a file or I/O device.
    #[pyfunction]
    #[allow(
        clippy::too_many_arguments,
        reason = "matches Win32 CreateFile parameter structure"
    )]
    fn CreateFile(
        file_name: PyStrRef,
        desired_access: u32,
        share_mode: u32,
        _security_attributes: PyObjectRef, // Always NULL (0)
        creation_disposition: u32,
        flags_and_attributes: u32,
        _template_file: PyObjectRef, // Always NULL (0)
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        use windows_sys::Win32::Storage::FileSystem::CreateFileW;

        let file_name_wide = file_name.as_wtf8().to_wide_with_nul();

        let handle = unsafe {
            CreateFileW(
                file_name_wide.as_ptr(),
                desired_access,
                share_mode,
                null(),
                creation_disposition,
                flags_and_attributes,
                null_mut(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }

        Ok(WinHandle(handle))
    }

    #[pyfunction]
    fn GetStdHandle(
        std_handle: windows_sys::Win32::System::Console::STD_HANDLE,
        vm: &VirtualMachine,
    ) -> PyResult<Option<WinHandle>> {
        let handle = unsafe { windows_sys::Win32::System::Console::GetStdHandle(std_handle) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }
        Ok(if handle.is_null() {
            // NULL handle - return None
            None
        } else {
            Some(WinHandle(handle))
        })
    }

    #[pyfunction]
    fn CreatePipe(
        _pipe_attrs: PyObjectRef,
        size: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(WinHandle, WinHandle)> {
        use windows_sys::Win32::Foundation::HANDLE;
        let (read, write) = unsafe {
            let mut read = core::mem::MaybeUninit::<HANDLE>::uninit();
            let mut write = core::mem::MaybeUninit::<HANDLE>::uninit();
            WindowsSysResult(windows_sys::Win32::System::Pipes::CreatePipe(
                read.as_mut_ptr(),
                write.as_mut_ptr(),
                core::ptr::null(),
                size,
            ))
            .to_pyresult(vm)?;
            (read.assume_init(), write.assume_init())
        };
        Ok((WinHandle(read), WinHandle(write)))
    }

    #[pyfunction]
    fn DuplicateHandle(
        src_process: WinHandle,
        src: WinHandle,
        target_process: WinHandle,
        access: u32,
        inherit: i32,
        options: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        use windows_sys::Win32::Foundation::HANDLE;
        let target = unsafe {
            let mut target = core::mem::MaybeUninit::<HANDLE>::uninit();
            WindowsSysResult(windows_sys::Win32::Foundation::DuplicateHandle(
                src_process.0,
                src.0,
                target_process.0,
                target.as_mut_ptr(),
                access,
                inherit,
                options.unwrap_or(0),
            ))
            .to_pyresult(vm)?;
            target.assume_init()
        };
        Ok(WinHandle(target))
    }

    #[pyfunction]
    fn GetACP() -> u32 {
        unsafe { windows_sys::Win32::Globalization::GetACP() }
    }

    #[pyfunction]
    fn GetCurrentProcess() -> WinHandle {
        WinHandle(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
    }

    #[pyfunction]
    fn GetFileType(
        h: WinHandle,
        vm: &VirtualMachine,
    ) -> PyResult<windows_sys::Win32::Storage::FileSystem::FILE_TYPE> {
        let file_type = unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(h.0) };
        if file_type == 0 && unsafe { windows_sys::Win32::Foundation::GetLastError() } != 0 {
            Err(vm.new_last_os_error())
        } else {
            Ok(file_type)
        }
    }

    #[pyfunction]
    fn GetLastError() -> u32 {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    }

    #[pyfunction]
    fn GetVersion() -> u32 {
        unsafe { windows_sys::Win32::System::SystemInformation::GetVersion() }
    }

    #[derive(FromArgs)]
    struct CreateProcessArgs {
        #[pyarg(positional)]
        name: Option<PyStrRef>,
        #[pyarg(positional)]
        command_line: Option<PyStrRef>,
        #[pyarg(positional)]
        _proc_attrs: PyObjectRef,
        #[pyarg(positional)]
        _thread_attrs: PyObjectRef,
        #[pyarg(positional)]
        inherit_handles: i32,
        #[pyarg(positional)]
        creation_flags: u32,
        #[pyarg(positional)]
        env_mapping: Option<ArgMapping>,
        #[pyarg(positional)]
        current_dir: Option<PyStrRef>,
        #[pyarg(positional)]
        startup_info: PyObjectRef,
    }

    #[pyfunction]
    fn CreateProcess(
        args: CreateProcessArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(WinHandle, WinHandle, u32, u32)> {
        let mut si: windows_sys::Win32::System::Threading::STARTUPINFOEXW =
            unsafe { core::mem::zeroed() };
        si.StartupInfo.cb = core::mem::size_of_val(&si) as _;

        macro_rules! si_attr {
            ($attr:ident, $t:ty) => {{
                si.StartupInfo.$attr = <Option<$t>>::try_from_object(
                    vm,
                    args.startup_info.get_attr(stringify!($attr), vm)?,
                )?
                .unwrap_or(0) as _
            }};
            ($attr:ident) => {{
                si.StartupInfo.$attr = <Option<_>>::try_from_object(
                    vm,
                    args.startup_info.get_attr(stringify!($attr), vm)?,
                )?
                .unwrap_or(0)
            }};
        }
        si_attr!(dwFlags);
        si_attr!(wShowWindow);
        si_attr!(hStdInput, usize);
        si_attr!(hStdOutput, usize);
        si_attr!(hStdError, usize);

        let mut env = args
            .env_mapping
            .map(|m| getenvironment(m, vm))
            .transpose()?;
        let env = env.as_mut().map_or_else(null_mut, |v| v.as_mut_ptr());

        let mut attrlist =
            getattributelist(args.startup_info.get_attr("lpAttributeList", vm)?, vm)?;
        si.lpAttributeList = attrlist
            .as_mut()
            .map_or_else(null_mut, |l| l.attrlist.as_mut_ptr() as _);

        let wstr = |s: PyStrRef| {
            let ws = widestring::WideCString::from_str(s.as_str())
                .map_err(|err| err.to_pyexception(vm))?;
            Ok(ws.into_vec_with_nul())
        };

        // Validate no embedded null bytes in command name and command line
        if let Some(ref name) = args.name
            && name.as_str().contains('\0')
        {
            return Err(crate::exceptions::cstring_error(vm));
        }
        if let Some(ref cmd) = args.command_line
            && cmd.as_str().contains('\0')
        {
            return Err(crate::exceptions::cstring_error(vm));
        }

        let app_name = args.name.map(wstr).transpose()?;
        let app_name = app_name.as_ref().map_or_else(null, |w| w.as_ptr());

        let mut command_line = args.command_line.map(wstr).transpose()?;
        let command_line = command_line
            .as_mut()
            .map_or_else(null_mut, |w| w.as_mut_ptr());

        let mut current_dir = args.current_dir.map(wstr).transpose()?;
        let current_dir = current_dir
            .as_mut()
            .map_or_else(null_mut, |w| w.as_mut_ptr());

        let procinfo = unsafe {
            let mut procinfo = core::mem::MaybeUninit::uninit();
            WindowsSysResult(windows_sys::Win32::System::Threading::CreateProcessW(
                app_name,
                command_line,
                core::ptr::null(),
                core::ptr::null(),
                args.inherit_handles,
                args.creation_flags
                    | windows_sys::Win32::System::Threading::EXTENDED_STARTUPINFO_PRESENT
                    | windows_sys::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT,
                env as _,
                current_dir,
                &mut si as *mut _ as *mut _,
                procinfo.as_mut_ptr(),
            ))
            .into_pyresult(vm)?;
            procinfo.assume_init()
        };

        Ok((
            WinHandle(procinfo.hProcess),
            WinHandle(procinfo.hThread),
            procinfo.dwProcessId,
            procinfo.dwThreadId,
        ))
    }

    #[pyfunction]
    fn OpenProcess(
        desired_access: u32,
        inherit_handle: bool,
        process_id: u32,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        let handle = unsafe {
            windows_sys::Win32::System::Threading::OpenProcess(
                desired_access,
                i32::from(inherit_handle),
                process_id,
            )
        };
        if handle.is_null() {
            return Err(vm.new_last_os_error());
        }
        Ok(WinHandle(handle))
    }

    #[pyfunction]
    fn ExitProcess(exit_code: u32) {
        unsafe { windows_sys::Win32::System::Threading::ExitProcess(exit_code) }
    }

    #[pyfunction]
    fn NeedCurrentDirectoryForExePath(exe_name: PyStrRef) -> bool {
        let exe_name = exe_name.as_wtf8().to_wide_with_nul();
        let return_value = unsafe {
            windows_sys::Win32::System::Environment::NeedCurrentDirectoryForExePathW(
                exe_name.as_ptr(),
            )
        };
        return_value != 0
    }

    #[pyfunction]
    fn CreateJunction(
        src_path: PyStrRef,
        dest_path: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let src_path = std::path::Path::new(src_path.as_str());
        let dest_path = std::path::Path::new(dest_path.as_str());

        junction::create(src_path, dest_path).map_err(|e| e.to_pyexception(vm))
    }

    fn getenvironment(env: ArgMapping, vm: &VirtualMachine) -> PyResult<Vec<u16>> {
        let keys = env.mapping().keys(vm)?;
        let values = env.mapping().values(vm)?;

        let keys = ArgSequence::try_from_object(vm, keys)?.into_vec();
        let values = ArgSequence::try_from_object(vm, values)?.into_vec();

        if keys.len() != values.len() {
            return Err(vm.new_runtime_error("environment changed size during iteration"));
        }

        // Deduplicate case-insensitive keys, keeping the last value
        use std::collections::HashMap;
        let mut last_entry: HashMap<String, widestring::WideString> = HashMap::new();
        for (k, v) in keys.into_iter().zip(values.into_iter()) {
            let k = PyStrRef::try_from_object(vm, k)?;
            let k = k.as_str();
            let v = PyStrRef::try_from_object(vm, v)?;
            let v = v.as_str();
            if k.contains('\0') || v.contains('\0') {
                return Err(crate::exceptions::cstring_error(vm));
            }
            if k.is_empty() || k[1..].contains('=') {
                return Err(vm.new_value_error("illegal environment variable name"));
            }
            let key_upper = k.to_uppercase();
            let mut entry = widestring::WideString::new();
            entry.push_str(k);
            entry.push_str("=");
            entry.push_str(v);
            entry.push_str("\0");
            last_entry.insert(key_upper, entry);
        }

        // Sort by uppercase key for case-insensitive ordering
        let mut entries: Vec<(String, widestring::WideString)> = last_entry.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut out = widestring::WideString::new();
        for (_, entry) in entries {
            out.push(entry);
        }
        out.push_str("\0");
        Ok(out.into_vec())
    }

    struct AttrList {
        handlelist: Option<Vec<usize>>,
        attrlist: Vec<u8>,
    }
    impl Drop for AttrList {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(
                    self.attrlist.as_mut_ptr() as *mut _,
                )
            };
        }
    }

    fn getattributelist(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<AttrList>> {
        <Option<ArgMapping>>::try_from_object(vm, obj)?
            .map(|mapping| {
                let handlelist = mapping
                    .as_ref()
                    .get_item("handle_list", vm)
                    .ok()
                    .and_then(|obj| {
                        <Option<ArgSequence<usize>>>::try_from_object(vm, obj)
                            .map(|s| match s {
                                Some(s) if !s.is_empty() => Some(s.into_vec()),
                                _ => None,
                            })
                            .transpose()
                    })
                    .transpose()?;

                let attr_count = handlelist.is_some() as u32;
                let (result, mut size) = unsafe {
                    let mut size = core::mem::MaybeUninit::uninit();
                    let result = WindowsSysResult(
                        windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList(
                            core::ptr::null_mut(),
                            attr_count,
                            0,
                            size.as_mut_ptr(),
                        ),
                    );
                    (result, size.assume_init())
                };
                if !result.is_err()
                    || unsafe { windows_sys::Win32::Foundation::GetLastError() }
                        != windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER
                {
                    return Err(vm.new_last_os_error());
                }
                let mut attrlist = vec![0u8; size];
                WindowsSysResult(unsafe {
                    windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList(
                        attrlist.as_mut_ptr() as *mut _,
                        attr_count,
                        0,
                        &mut size,
                    )
                })
                .into_pyresult(vm)?;
                let mut attrs = AttrList {
                    handlelist,
                    attrlist,
                };
                if let Some(ref mut handlelist) = attrs.handlelist {
                    WindowsSysResult(unsafe {
                        windows_sys::Win32::System::Threading::UpdateProcThreadAttribute(
                            attrs.attrlist.as_mut_ptr() as _,
                            0,
                            (2 & 0xffff) | 0x20000, // PROC_THREAD_ATTRIBUTE_HANDLE_LIST
                            handlelist.as_mut_ptr() as _,
                            (handlelist.len() * core::mem::size_of::<usize>()) as _,
                            core::ptr::null_mut(),
                            core::ptr::null(),
                        )
                    })
                    .into_pyresult(vm)?;
                }
                Ok(attrs)
            })
            .transpose()
    }

    #[pyfunction]
    fn WaitForSingleObject(h: WinHandle, ms: i64, vm: &VirtualMachine) -> PyResult<u32> {
        // Negative values (e.g., -1) map to INFINITE (0xFFFFFFFF)
        let ms = if ms < 0 {
            windows_sys::Win32::System::Threading::INFINITE
        } else if ms > u32::MAX as i64 {
            return Err(vm.new_overflow_error("timeout value is too large".to_owned()));
        } else {
            ms as u32
        };
        let ret = unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(h.0, ms) };
        if ret == windows_sys::Win32::Foundation::WAIT_FAILED {
            Err(vm.new_last_os_error())
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn WaitForMultipleObjects(
        handle_seq: ArgSequence<isize>,
        wait_all: bool,
        milliseconds: u32,
        vm: &VirtualMachine,
    ) -> PyResult<u32> {
        use windows_sys::Win32::Foundation::WAIT_FAILED;
        use windows_sys::Win32::System::Threading::WaitForMultipleObjects as WinWaitForMultipleObjects;

        let handles: Vec<HANDLE> = handle_seq
            .into_vec()
            .into_iter()
            .map(|h| h as HANDLE)
            .collect();

        if handles.is_empty() {
            return Err(vm.new_value_error("handle_seq must not be empty".to_owned()));
        }

        if handles.len() > 64 {
            return Err(
                vm.new_value_error("WaitForMultipleObjects supports at most 64 handles".to_owned())
            );
        }

        let ret = unsafe {
            WinWaitForMultipleObjects(
                handles.len() as u32,
                handles.as_ptr(),
                if wait_all { 1 } else { 0 },
                milliseconds,
            )
        };

        if ret == WAIT_FAILED {
            Err(vm.new_last_os_error())
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn GetExitCodeProcess(h: WinHandle, vm: &VirtualMachine) -> PyResult<u32> {
        unsafe {
            let mut ec = core::mem::MaybeUninit::uninit();
            WindowsSysResult(windows_sys::Win32::System::Threading::GetExitCodeProcess(
                h.0,
                ec.as_mut_ptr(),
            ))
            .to_pyresult(vm)?;
            Ok(ec.assume_init())
        }
    }

    #[pyfunction]
    fn TerminateProcess(h: WinHandle, exit_code: u32) -> WindowsSysResult<i32> {
        WindowsSysResult(unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(h.0, exit_code)
        })
    }

    // TODO: ctypes.LibraryLoader.LoadLibrary
    #[allow(dead_code)]
    fn LoadLibrary(path: PyStrRef, vm: &VirtualMachine) -> PyResult<isize> {
        let path_wide = path.as_wtf8().to_wide_with_nul();
        let handle =
            unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(path_wide.as_ptr()) };
        if handle.is_null() {
            return Err(vm.new_runtime_error("LoadLibrary failed"));
        }
        Ok(handle as isize)
    }

    #[pyfunction]
    fn GetModuleFileName(handle: isize, vm: &VirtualMachine) -> PyResult<String> {
        let mut path: Vec<u16> = vec![0; MAX_PATH as usize];

        let length = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
                handle as windows_sys::Win32::Foundation::HMODULE,
                path.as_mut_ptr(),
                path.len() as u32,
            )
        };
        if length == 0 {
            return Err(vm.new_runtime_error("GetModuleFileName failed"));
        }

        let (path, _) = path.split_at(length as usize);
        Ok(String::from_utf16(path).unwrap())
    }

    #[pyfunction]
    fn OpenMutexW(
        desired_access: u32,
        inherit_handle: bool,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        let name_wide = name.as_wtf8().to_wide_with_nul();
        let handle = unsafe {
            windows_sys::Win32::System::Threading::OpenMutexW(
                desired_access,
                i32::from(inherit_handle),
                name_wide.as_ptr(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }
        Ok(handle as _)
    }

    #[pyfunction]
    fn ReleaseMutex(handle: isize) -> WindowsSysResult<i32> {
        WindowsSysResult(unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(handle as _)
        })
    }

    // LOCALE_NAME_INVARIANT is an empty string in Windows API
    #[pyattr]
    const LOCALE_NAME_INVARIANT: &str = "";

    /// LCMapStringEx - Map a string to another string using locale-specific rules
    /// This is used by ntpath.normcase() for proper Windows case conversion
    #[pyfunction]
    fn LCMapStringEx(
        locale: PyStrRef,
        flags: u32,
        src: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        use rustpython_common::wtf8::Wtf8Buf;
        use windows_sys::Win32::Globalization::{
            LCMAP_BYTEREV, LCMAP_HASH, LCMAP_SORTHANDLE, LCMAP_SORTKEY,
            LCMapStringEx as WinLCMapStringEx,
        };

        // Reject unsupported flags
        if flags & (LCMAP_SORTHANDLE | LCMAP_HASH | LCMAP_BYTEREV | LCMAP_SORTKEY) != 0 {
            return Err(vm.new_value_error("unsupported flags"));
        }

        // Use ToWideString which properly handles WTF-8 (including surrogates)
        let locale_wide = locale.as_wtf8().to_wide_with_nul();
        let src_wide = src.as_wtf8().to_wide();

        if src_wide.len() > i32::MAX as usize {
            return Err(vm.new_overflow_error("input string is too long".to_string()));
        }

        // First call to get required buffer size
        let dest_size = unsafe {
            WinLCMapStringEx(
                locale_wide.as_ptr(),
                flags,
                src_wide.as_ptr(),
                src_wide.len() as i32,
                null_mut(),
                0,
                null(),
                null(),
                0,
            )
        };

        if dest_size <= 0 {
            return Err(vm.new_last_os_error());
        }

        // Second call to perform the mapping
        let mut dest = vec![0u16; dest_size as usize];
        let nmapped = unsafe {
            WinLCMapStringEx(
                locale_wide.as_ptr(),
                flags,
                src_wide.as_ptr(),
                src_wide.len() as i32,
                dest.as_mut_ptr(),
                dest_size,
                null(),
                null(),
                0,
            )
        };

        if nmapped <= 0 {
            return Err(vm.new_last_os_error());
        }

        dest.truncate(nmapped as usize);

        // Convert UTF-16 back to WTF-8 (handles surrogates properly)
        let result = Wtf8Buf::from_wide(&dest);
        Ok(vm.ctx.new_str(result))
    }

    #[derive(FromArgs)]
    struct CreateNamedPipeArgs {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(positional)]
        open_mode: u32,
        #[pyarg(positional)]
        pipe_mode: u32,
        #[pyarg(positional)]
        max_instances: u32,
        #[pyarg(positional)]
        out_buffer_size: u32,
        #[pyarg(positional)]
        in_buffer_size: u32,
        #[pyarg(positional)]
        default_timeout: u32,
        #[pyarg(positional)]
        _security_attributes: PyObjectRef, // Ignored, can be None
    }

    /// CreateNamedPipe - Create a named pipe
    #[pyfunction]
    fn CreateNamedPipe(args: CreateNamedPipeArgs, vm: &VirtualMachine) -> PyResult<WinHandle> {
        use windows_sys::Win32::System::Pipes::CreateNamedPipeW;

        let name_wide = args.name.as_wtf8().to_wide_with_nul();

        let handle = unsafe {
            CreateNamedPipeW(
                name_wide.as_ptr(),
                args.open_mode,
                args.pipe_mode,
                args.max_instances,
                args.out_buffer_size,
                args.in_buffer_size,
                args.default_timeout,
                null(), // security_attributes - NULL for now
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }

        Ok(WinHandle(handle))
    }

    // ==================== Overlapped class ====================
    // Used for asynchronous I/O operations (ConnectNamedPipe, ReadFile, WriteFile)

    #[pyattr]
    #[pyclass(name = "Overlapped", module = "_winapi")]
    #[derive(Debug, PyPayload)]
    struct Overlapped {
        inner: PyMutex<OverlappedInner>,
    }

    struct OverlappedInner {
        overlapped: windows_sys::Win32::System::IO::OVERLAPPED,
        handle: HANDLE,
        pending: bool,
        completed: bool,
        read_buffer: Option<Vec<u8>>,
        write_buffer: Option<Vec<u8>>,
    }

    impl core::fmt::Debug for OverlappedInner {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedInner")
                .field("handle", &self.handle)
                .field("pending", &self.pending)
                .field("completed", &self.completed)
                .finish()
        }
    }

    unsafe impl Sync for OverlappedInner {}
    unsafe impl Send for OverlappedInner {}

    #[pyclass(with(Constructor))]
    impl Overlapped {
        fn new_with_handle(handle: HANDLE) -> Self {
            use windows_sys::Win32::System::Threading::CreateEventW;

            let event = unsafe { CreateEventW(null(), 1, 0, null()) };
            let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
                unsafe { core::mem::zeroed() };
            overlapped.hEvent = event;

            Overlapped {
                inner: PyMutex::new(OverlappedInner {
                    overlapped,
                    handle,
                    pending: false,
                    completed: false,
                    read_buffer: None,
                    write_buffer: None,
                }),
            }
        }

        #[pymethod]
        fn GetOverlappedResult(&self, wait: bool, vm: &VirtualMachine) -> PyResult<(u32, u32)> {
            use windows_sys::Win32::Foundation::{
                ERROR_IO_INCOMPLETE, ERROR_MORE_DATA, ERROR_OPERATION_ABORTED, ERROR_SUCCESS,
                GetLastError,
            };
            use windows_sys::Win32::System::IO::GetOverlappedResult;

            let mut inner = self.inner.lock();

            let mut transferred: u32 = 0;

            let ret = unsafe {
                GetOverlappedResult(
                    inner.handle,
                    &inner.overlapped,
                    &mut transferred,
                    if wait { 1 } else { 0 },
                )
            };

            let err = if ret == 0 {
                unsafe { GetLastError() }
            } else {
                ERROR_SUCCESS
            };

            match err {
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_OPERATION_ABORTED => {
                    inner.completed = true;
                    inner.pending = false;
                }
                ERROR_IO_INCOMPLETE => {}
                _ => {
                    inner.pending = false;
                    return Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm));
                }
            }

            if inner.completed
                && let Some(read_buffer) = &mut inner.read_buffer
                && transferred != read_buffer.len() as u32
            {
                read_buffer.truncate(transferred as usize);
            }

            Ok((transferred, err))
        }

        #[pymethod]
        fn getbuffer(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let inner = self.inner.lock();
            if !inner.completed {
                return Err(vm.new_value_error(
                    "can't get read buffer before GetOverlappedResult() signals the operation completed"
                        .to_owned(),
                ));
            }
            Ok(inner
                .read_buffer
                .as_ref()
                .map(|buf| vm.ctx.new_bytes(buf.clone()).into()))
        }

        #[pymethod]
        fn cancel(&self, vm: &VirtualMachine) -> PyResult<()> {
            use windows_sys::Win32::System::IO::CancelIoEx;

            let mut inner = self.inner.lock();
            let ret = if inner.pending {
                unsafe { CancelIoEx(inner.handle, &inner.overlapped) }
            } else {
                1
            };
            if ret == 0 {
                let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                if err != windows_sys::Win32::Foundation::ERROR_NOT_FOUND {
                    return Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm));
                }
            }
            inner.pending = false;
            Ok(())
        }

        #[pygetset]
        fn event(&self) -> isize {
            let inner = self.inner.lock();
            inner.overlapped.hEvent as isize
        }
    }

    impl Constructor for Overlapped {
        type Args = ();

        fn py_new(
            _cls: &Py<crate::builtins::PyType>,
            _args: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Overlapped::new_with_handle(null_mut()))
        }
    }

    impl Drop for OverlappedInner {
        fn drop(&mut self) {
            use windows_sys::Win32::Foundation::CloseHandle;
            if !self.overlapped.hEvent.is_null() {
                unsafe { CloseHandle(self.overlapped.hEvent) };
            }
        }
    }

    /// ConnectNamedPipe - Wait for a client to connect to a named pipe
    #[derive(FromArgs)]
    struct ConnectNamedPipeArgs {
        #[pyarg(positional)]
        handle: WinHandle,
        #[pyarg(named, optional)]
        overlapped: OptionalArg<bool>,
    }

    #[pyfunction]
    fn ConnectNamedPipe(args: ConnectNamedPipeArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        use windows_sys::Win32::Foundation::{
            ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GetLastError,
        };

        let handle = args.handle;
        let use_overlapped = args.overlapped.unwrap_or(false);

        if use_overlapped {
            // Overlapped (async) mode
            let ov = Overlapped::new_with_handle(handle.0);

            let _ret = {
                let mut inner = ov.inner.lock();
                unsafe {
                    windows_sys::Win32::System::Pipes::ConnectNamedPipe(
                        handle.0,
                        &mut inner.overlapped,
                    )
                }
            };

            let err = unsafe { GetLastError() };
            match err {
                ERROR_IO_PENDING => {
                    let mut inner = ov.inner.lock();
                    inner.pending = true;
                }
                ERROR_PIPE_CONNECTED => {
                    let inner = ov.inner.lock();
                    unsafe {
                        windows_sys::Win32::System::Threading::SetEvent(inner.overlapped.hEvent);
                    }
                }
                _ => {
                    return Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm));
                }
            }

            Ok(ov.into_pyobject(vm))
        } else {
            // Synchronous mode
            let ret = unsafe {
                windows_sys::Win32::System::Pipes::ConnectNamedPipe(handle.0, null_mut())
            };

            if ret == 0 {
                let err = unsafe { GetLastError() };
                if err != ERROR_PIPE_CONNECTED {
                    return Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm));
                }
            }

            Ok(vm.ctx.none())
        }
    }

    /// Helper for GetShortPathName and GetLongPathName
    fn get_path_name_impl(
        path: &PyStrRef,
        api_fn: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        use rustpython_common::wtf8::Wtf8Buf;

        let path_wide = path.as_wtf8().to_wide_with_nul();

        // First call to get required buffer size
        let size = unsafe { api_fn(path_wide.as_ptr(), null_mut(), 0) };

        if size == 0 {
            return Err(vm.new_last_os_error());
        }

        // Second call to get the actual path
        let mut buffer: Vec<u16> = vec![0; size as usize];
        let result =
            unsafe { api_fn(path_wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };

        if result == 0 {
            return Err(vm.new_last_os_error());
        }

        // Truncate to actual length (excluding null terminator)
        buffer.truncate(result as usize);

        // Convert UTF-16 back to WTF-8 (handles surrogates properly)
        let result_str = Wtf8Buf::from_wide(&buffer);
        Ok(vm.ctx.new_str(result_str))
    }

    /// GetShortPathName - Return the short version of the provided path.
    #[pyfunction]
    fn GetShortPathName(path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        get_path_name_impl(&path, GetShortPathNameW, vm)
    }

    /// GetLongPathName - Return the long version of the provided path.
    #[pyfunction]
    fn GetLongPathName(path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
        get_path_name_impl(&path, GetLongPathNameW, vm)
    }

    /// WaitNamedPipe - Wait for an instance of a named pipe to become available.
    #[pyfunction]
    fn WaitNamedPipe(name: PyStrRef, timeout: u32, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

        let name_wide = name.as_wtf8().to_wide_with_nul();

        let success = unsafe { WaitNamedPipeW(name_wide.as_ptr(), timeout) };

        if success == 0 {
            return Err(vm.new_last_os_error());
        }

        Ok(())
    }

    /// PeekNamedPipe - Peek at data in a named pipe without removing it.
    #[pyfunction]
    fn PeekNamedPipe(
        handle: WinHandle,
        size: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        use windows_sys::Win32::System::Pipes::PeekNamedPipe as WinPeekNamedPipe;

        let size = size.unwrap_or(0);

        if size < 0 {
            return Err(vm.new_value_error("negative size".to_string()));
        }

        let mut navail: u32 = 0;
        let mut nleft: u32 = 0;

        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            let mut nread: u32 = 0;

            let ret = unsafe {
                WinPeekNamedPipe(
                    handle.0,
                    buf.as_mut_ptr() as *mut _,
                    size as u32,
                    &mut nread,
                    &mut navail,
                    &mut nleft,
                )
            };

            if ret == 0 {
                return Err(vm.new_last_os_error());
            }

            buf.truncate(nread as usize);
            let bytes: PyObjectRef = vm.ctx.new_bytes(buf).into();
            Ok(vm
                .ctx
                .new_tuple(vec![
                    bytes,
                    vm.ctx.new_int(navail).into(),
                    vm.ctx.new_int(nleft).into(),
                ])
                .into())
        } else {
            let ret = unsafe {
                WinPeekNamedPipe(handle.0, null_mut(), 0, null_mut(), &mut navail, &mut nleft)
            };

            if ret == 0 {
                return Err(vm.new_last_os_error());
            }

            Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.new_int(navail).into(),
                    vm.ctx.new_int(nleft).into(),
                ])
                .into())
        }
    }

    /// CreateEventW - Create or open a named or unnamed event object.
    #[pyfunction]
    fn CreateEventW(
        security_attributes: isize, // Always NULL (0)
        manual_reset: bool,
        initial_state: bool,
        name: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<WinHandle>> {
        use windows_sys::Win32::System::Threading::CreateEventW as WinCreateEventW;

        let _ = security_attributes; // Ignored, always NULL

        let name_wide = name.map(|n| n.as_wtf8().to_wide_with_nul());
        let name_ptr = name_wide.as_ref().map_or(null(), |n| n.as_ptr());

        let handle = unsafe {
            WinCreateEventW(
                null(),
                i32::from(manual_reset),
                i32::from(initial_state),
                name_ptr,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(vm.new_last_os_error());
        }

        if handle.is_null() {
            return Ok(None);
        }

        Ok(Some(WinHandle(handle)))
    }

    /// SetEvent - Set the specified event object to the signaled state.
    #[pyfunction]
    fn SetEvent(event: WinHandle, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::System::Threading::SetEvent as WinSetEvent;

        let ret = unsafe { WinSetEvent(event.0) };

        if ret == 0 {
            return Err(vm.new_last_os_error());
        }

        Ok(())
    }

    /// WriteFile - Write data to a file or I/O device.
    #[pyfunction]
    fn WriteFile(
        handle: WinHandle,
        buffer: crate::function::ArgBytesLike,
        use_overlapped: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile as WinWriteFile;

        let use_overlapped = use_overlapped.unwrap_or(false);
        let buf = buffer.borrow_buf();
        let len = core::cmp::min(buf.len(), u32::MAX as usize) as u32;

        if use_overlapped {
            use windows_sys::Win32::Foundation::ERROR_IO_PENDING;

            let ov = Overlapped::new_with_handle(handle.0);
            let err = {
                let mut inner = ov.inner.lock();
                inner.write_buffer = Some(buf.to_vec());
                let write_buf = inner.write_buffer.as_ref().unwrap();
                let mut written: u32 = 0;
                let ret = unsafe {
                    WinWriteFile(
                        handle.0,
                        write_buf.as_ptr() as *const _,
                        len,
                        &mut written,
                        &mut inner.overlapped,
                    )
                };

                let err = if ret == 0 {
                    unsafe { windows_sys::Win32::Foundation::GetLastError() }
                } else {
                    0
                };

                if ret == 0 && err != ERROR_IO_PENDING {
                    return Err(vm.new_last_os_error());
                }
                if ret == 0 && err == ERROR_IO_PENDING {
                    inner.pending = true;
                }

                err
            };
            let result = vm
                .ctx
                .new_tuple(vec![ov.into_pyobject(vm), vm.ctx.new_int(err).into()]);
            return Ok(result.into());
        }

        let mut written: u32 = 0;
        let ret = unsafe {
            WinWriteFile(
                handle.0,
                buf.as_ptr() as *const _,
                len,
                &mut written,
                null_mut(),
            )
        };
        let err = if ret == 0 {
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        } else {
            0
        };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(written).into(),
                vm.ctx.new_int(err).into(),
            ])
            .into())
    }

    const MAXIMUM_WAIT_OBJECTS: usize = 64;

    /// BatchedWaitForMultipleObjects - Wait for multiple handles, supporting more than 64.
    #[pyfunction]
    fn BatchedWaitForMultipleObjects(
        handle_seq: PyObjectRef,
        wait_all: bool,
        milliseconds: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicU32, Ordering};
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_FAILED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::SystemInformation::GetTickCount64;
        use windows_sys::Win32::System::Threading::{
            CreateEventW as WinCreateEventW, CreateThread, GetExitCodeThread,
            INFINITE as WIN_INFINITE, ResumeThread, SetEvent as WinSetEvent, TerminateThread,
            WaitForMultipleObjects,
        };

        let milliseconds = milliseconds.unwrap_or(WIN_INFINITE);

        // Get handles from sequence
        let seq = ArgSequence::<isize>::try_from_object(vm, handle_seq)?;
        let handles: Vec<isize> = seq.into_vec();
        let nhandles = handles.len();

        if nhandles == 0 {
            return if wait_all {
                Ok(vm.ctx.none())
            } else {
                Ok(vm.ctx.new_list(vec![]).into())
            };
        }

        let max_total_objects = (MAXIMUM_WAIT_OBJECTS - 1) * (MAXIMUM_WAIT_OBJECTS - 1);
        if nhandles > max_total_objects {
            return Err(vm.new_value_error(format!(
                "need at most {} handles, got a sequence of length {}",
                max_total_objects, nhandles
            )));
        }

        // Create batches of handles
        let batch_size = MAXIMUM_WAIT_OBJECTS - 1; // Leave room for cancel_event
        let mut batches: Vec<Vec<isize>> = Vec::new();
        let mut i = 0;
        while i < nhandles {
            let end = core::cmp::min(i + batch_size, nhandles);
            batches.push(handles[i..end].to_vec());
            i = end;
        }

        #[cfg(feature = "threading")]
        let sigint_event = {
            let is_main = crate::stdlib::thread::get_ident() == vm.state.main_thread_ident.load();
            if is_main {
                let handle = crate::signal::get_sigint_event().unwrap_or_else(|| {
                    let handle = unsafe { WinCreateEventW(null(), 1, 0, null()) };
                    if !handle.is_null() {
                        crate::signal::set_sigint_event(handle as isize);
                    }
                    handle as isize
                });
                if handle == 0 { None } else { Some(handle) }
            } else {
                None
            }
        };
        #[cfg(not(feature = "threading"))]
        let sigint_event: Option<isize> = None;

        if wait_all {
            // For wait_all, we wait sequentially for each batch
            let mut err: Option<u32> = None;
            let deadline = if milliseconds != WIN_INFINITE {
                Some(unsafe { GetTickCount64() } + milliseconds as u64)
            } else {
                None
            };

            for batch in &batches {
                let timeout = if let Some(deadline) = deadline {
                    let now = unsafe { GetTickCount64() };
                    if now >= deadline {
                        err = Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT);
                        break;
                    }
                    (deadline - now) as u32
                } else {
                    WIN_INFINITE
                };

                let batch_handles: Vec<_> = batch.iter().map(|&h| h as _).collect();
                let result = unsafe {
                    WaitForMultipleObjects(
                        batch_handles.len() as u32,
                        batch_handles.as_ptr(),
                        1, // wait_all = TRUE
                        timeout,
                    )
                };

                if result == WAIT_FAILED {
                    err = Some(unsafe { windows_sys::Win32::Foundation::GetLastError() });
                    break;
                }
                if result == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                    err = Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT);
                    break;
                }

                if let Some(sigint_event) = sigint_event {
                    let sig_result = unsafe {
                        windows_sys::Win32::System::Threading::WaitForSingleObject(
                            sigint_event as _,
                            0,
                        )
                    };
                    if sig_result == WAIT_OBJECT_0 {
                        err = Some(windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT);
                        break;
                    }
                    if sig_result == WAIT_FAILED {
                        err = Some(unsafe { windows_sys::Win32::Foundation::GetLastError() });
                        break;
                    }
                }
            }

            if let Some(err) = err {
                if err == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                    return Err(vm.new_exception_empty(vm.ctx.exceptions.timeout_error.to_owned()));
                }
                if err == windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT {
                    return Err(vm
                        .new_errno_error(libc::EINTR, "Interrupted system call")
                        .upcast());
                }
                return Err(vm.new_os_error(err as i32));
            }

            Ok(vm.ctx.none())
        } else {
            // For wait_any, we use threads to wait on each batch in parallel
            let cancel_event = unsafe { WinCreateEventW(null(), 1, 0, null()) }; // Manual reset, not signaled
            if cancel_event.is_null() {
                return Err(vm.new_last_os_error());
            }

            struct BatchData {
                handles: Vec<isize>,
                cancel_event: isize,
                handle_base: usize,
                result: AtomicU32,
                thread: core::cell::UnsafeCell<isize>,
            }

            unsafe impl Send for BatchData {}
            unsafe impl Sync for BatchData {}

            let batch_data: Vec<Arc<BatchData>> = batches
                .iter()
                .enumerate()
                .map(|(idx, batch)| {
                    let base = idx * batch_size;
                    let mut handles_with_cancel = batch.clone();
                    handles_with_cancel.push(cancel_event as isize);
                    Arc::new(BatchData {
                        handles: handles_with_cancel,
                        cancel_event: cancel_event as isize,
                        handle_base: base,
                        result: AtomicU32::new(WAIT_FAILED),
                        thread: core::cell::UnsafeCell::new(0),
                    })
                })
                .collect();

            // Thread function
            extern "system" fn batch_wait_thread(param: *mut core::ffi::c_void) -> u32 {
                let data = unsafe { &*(param as *const BatchData) };
                let handles: Vec<_> = data.handles.iter().map(|&h| h as _).collect();
                let result = unsafe {
                    WaitForMultipleObjects(
                        handles.len() as u32,
                        handles.as_ptr(),
                        0, // wait_any
                        WIN_INFINITE,
                    )
                };
                data.result.store(result, Ordering::SeqCst);

                if result == WAIT_FAILED {
                    let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                    unsafe { WinSetEvent(data.cancel_event as _) };
                    return err;
                } else if result >= windows_sys::Win32::Foundation::WAIT_ABANDONED_0
                    && result
                        < windows_sys::Win32::Foundation::WAIT_ABANDONED_0
                            + MAXIMUM_WAIT_OBJECTS as u32
                {
                    data.result.store(WAIT_FAILED, Ordering::SeqCst);
                    unsafe { WinSetEvent(data.cancel_event as _) };
                    return windows_sys::Win32::Foundation::ERROR_ABANDONED_WAIT_0;
                }
                0
            }

            // Create threads
            let mut thread_handles: Vec<isize> = Vec::new();
            for data in &batch_data {
                let thread = unsafe {
                    CreateThread(
                        null(),
                        1, // Smallest stack
                        Some(batch_wait_thread),
                        Arc::as_ptr(data) as *const _ as *mut _,
                        4, // CREATE_SUSPENDED
                        null_mut(),
                    )
                };
                if thread.is_null() {
                    // Cleanup on error
                    for h in &thread_handles {
                        unsafe { TerminateThread(*h as _, 0) };
                        unsafe { CloseHandle(*h as _) };
                    }
                    unsafe { CloseHandle(cancel_event) };
                    return Err(vm.new_last_os_error());
                }
                unsafe { *data.thread.get() = thread as isize };
                thread_handles.push(thread as isize);
            }

            // Resume all threads
            for &thread in &thread_handles {
                unsafe { ResumeThread(thread as _) };
            }

            // Wait for any thread to complete
            let mut thread_handles_raw: Vec<_> = thread_handles.iter().map(|&h| h as _).collect();
            if let Some(sigint_event) = sigint_event {
                thread_handles_raw.push(sigint_event as _);
            }
            let result = unsafe {
                WaitForMultipleObjects(
                    thread_handles_raw.len() as u32,
                    thread_handles_raw.as_ptr(),
                    0, // wait_any
                    milliseconds,
                )
            };

            let err = if result == WAIT_FAILED {
                Some(unsafe { windows_sys::Win32::Foundation::GetLastError() })
            } else if result == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT)
            } else if sigint_event.is_some()
                && result == WAIT_OBJECT_0 + thread_handles_raw.len() as u32
            {
                Some(windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT)
            } else {
                None
            };

            // Signal cancel event to stop other threads
            unsafe { WinSetEvent(cancel_event) };

            // Wait for all threads to finish
            let thread_handles_only: Vec<_> = thread_handles.iter().map(|&h| h as _).collect();
            unsafe {
                WaitForMultipleObjects(
                    thread_handles_only.len() as u32,
                    thread_handles_only.as_ptr(),
                    1, // wait_all
                    WIN_INFINITE,
                )
            };

            // Check for errors from threads
            let mut thread_err = err;
            for data in &batch_data {
                if thread_err.is_none() && data.result.load(Ordering::SeqCst) == WAIT_FAILED {
                    let mut exit_code: u32 = 0;
                    let thread = unsafe { *data.thread.get() };
                    if unsafe { GetExitCodeThread(thread as _, &mut exit_code) } == 0 {
                        thread_err =
                            Some(unsafe { windows_sys::Win32::Foundation::GetLastError() });
                    } else if exit_code != 0 {
                        thread_err = Some(exit_code);
                    }
                }
                let thread = unsafe { *data.thread.get() };
                unsafe { CloseHandle(thread as _) };
            }

            unsafe { CloseHandle(cancel_event) };

            // Return result
            if let Some(e) = thread_err {
                if e == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                    return Err(vm.new_exception_empty(vm.ctx.exceptions.timeout_error.to_owned()));
                }
                if e == windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT {
                    return Err(vm
                        .new_errno_error(libc::EINTR, "Interrupted system call")
                        .upcast());
                }
                return Err(vm.new_os_error(e as i32));
            }

            // Collect triggered indices
            let mut triggered_indices: Vec<PyObjectRef> = Vec::new();
            for data in &batch_data {
                let result = data.result.load(Ordering::SeqCst);
                let triggered = result as i32 - WAIT_OBJECT_0 as i32;
                // Check if it's a valid handle index (not the cancel_event which is last)
                if triggered >= 0 && (triggered as usize) < data.handles.len() - 1 {
                    let index = data.handle_base + triggered as usize;
                    triggered_indices.push(vm.ctx.new_int(index).into());
                }
            }

            Ok(vm.ctx.new_list(triggered_indices).into())
        }
    }
}

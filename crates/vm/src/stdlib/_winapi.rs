// spell-checker:disable

#![allow(non_snake_case)]
pub(crate) use _winapi::module_def;

#[pymodule]
mod _winapi {
    use crate::{
        Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
        builtins::PyStrRef,
        common::lock::PyMutex,
        convert::ToPyException,
        function::{ArgMapping, ArgSequence, OptionalArg},
        types::Constructor,
        windows::{WinHandle, WindowsSysResult},
    };
    use core::ptr::{null, null_mut};
    use rustpython_common::wtf8::Wtf8Buf;
    use rustpython_host_env::overlapped as host_overlapped;
    use rustpython_host_env::winapi as host_winapi;
    use rustpython_host_env::windows::ToWideString;
    use windows_sys::Win32::Foundation::{HANDLE, MAX_PATH};

    #[pyattr]
    use windows_sys::Win32::{
        Foundation::{
            DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, ERROR_ACCESS_DENIED,
            ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA,
            ERROR_NETNAME_DELETED, ERROR_NO_DATA, ERROR_NO_SYSTEM_RESOURCES,
            ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
            ERROR_PRIVILEGE_NOT_HELD, ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE, STILL_ACTIVE,
            WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Globalization::{
            LCMAP_FULLWIDTH, LCMAP_HALFWIDTH, LCMAP_HIRAGANA, LCMAP_KATAKANA,
            LCMAP_LINGUISTIC_CASING, LCMAP_LOWERCASE, LCMAP_SIMPLIFIED_CHINESE, LCMAP_TITLECASE,
            LCMAP_TRADITIONAL_CHINESE, LCMAP_UPPERCASE,
        },
        Storage::FileSystem::{
            COPY_FILE_ALLOW_DECRYPTED_DESTINATION, COPY_FILE_COPY_SYMLINK,
            COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, COPY_FILE_NO_OFFLOAD,
            COPY_FILE_OPEN_SOURCE_FOR_WRITE, COPY_FILE_REQUEST_COMPRESSED_TRAFFIC,
            COPY_FILE_REQUEST_SECURITY_PRIVILEGES, COPY_FILE_RESTARTABLE,
            COPY_FILE_RESUME_FROM_PAUSE, COPYFILE2_CALLBACK_CHUNK_FINISHED,
            COPYFILE2_CALLBACK_CHUNK_STARTED, COPYFILE2_CALLBACK_ERROR,
            COPYFILE2_CALLBACK_POLL_CONTINUE, COPYFILE2_CALLBACK_STREAM_FINISHED,
            COPYFILE2_CALLBACK_STREAM_STARTED, COPYFILE2_PROGRESS_CANCEL,
            COPYFILE2_PROGRESS_CONTINUE, COPYFILE2_PROGRESS_PAUSE, COPYFILE2_PROGRESS_QUIET,
            COPYFILE2_PROGRESS_STOP, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
            FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE,
            FILE_TYPE_REMOTE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND, SYNCHRONIZE,
        },
        System::{
            Console::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
            Memory::{
                FILE_MAP_ALL_ACCESS, FILE_MAP_COPY, FILE_MAP_EXECUTE, FILE_MAP_READ,
                FILE_MAP_WRITE, MEM_COMMIT, MEM_FREE, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE,
                MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
                PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, PAGE_NOCACHE, PAGE_READONLY,
                PAGE_READWRITE, PAGE_WRITECOMBINE, PAGE_WRITECOPY, SEC_COMMIT, SEC_IMAGE,
                SEC_LARGE_PAGES, SEC_NOCACHE, SEC_RESERVE, SEC_WRITECOMBINE,
            },
            Pipes::{
                NMPWAIT_WAIT_FOREVER, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
                PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            SystemServices::LOCALE_NAME_MAX_LENGTH,
            Threading::{
                ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
                CREATE_BREAKAWAY_FROM_JOB, CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE,
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, HIGH_PRIORITY_CLASS,
                IDLE_PRIORITY_CLASS, INFINITE, NORMAL_PRIORITY_CLASS, PROCESS_ALL_ACCESS,
                PROCESS_DUP_HANDLE, REALTIME_PRIORITY_CLASS, STARTF_FORCEOFFFEEDBACK,
                STARTF_FORCEONFEEDBACK, STARTF_PREVENTPINNING, STARTF_RUNFULLSCREEN,
                STARTF_TITLEISAPPID, STARTF_TITLEISLINKNAME, STARTF_UNTRUSTEDSOURCE,
                STARTF_USECOUNTCHARS, STARTF_USEFILLATTRIBUTE, STARTF_USEHOTKEY,
                STARTF_USEPOSITION, STARTF_USESHOWWINDOW, STARTF_USESIZE, STARTF_USESTDHANDLES,
            },
        },
        UI::WindowsAndMessaging::SW_HIDE,
    };

    #[pyattr]
    const NULL: isize = 0;

    #[pyattr]
    const INVALID_HANDLE_VALUE: isize = -1;

    #[pyattr]
    const COPY_FILE_DIRECTORY: u32 = 0x00000080;

    #[pyfunction]
    fn CloseHandle(handle: WinHandle) -> WindowsSysResult<i32> {
        WindowsSysResult(host_winapi::close_handle(handle.0))
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
        let file_name_wide = file_name.as_wtf8().to_wide_with_nul();
        host_winapi::create_file_w(
            file_name_wide.as_ptr(),
            desired_access,
            share_mode,
            creation_disposition,
            flags_and_attributes,
        )
        .map(WinHandle)
        .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn GetStdHandle(
        std_handle: windows_sys::Win32::System::Console::STD_HANDLE,
        vm: &VirtualMachine,
    ) -> PyResult<Option<WinHandle>> {
        host_winapi::get_std_handle(std_handle)
            .map(|handle| handle.map(WinHandle))
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn CreatePipe(
        _pipe_attrs: PyObjectRef,
        size: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(WinHandle, WinHandle)> {
        host_winapi::create_pipe(size)
            .map(|(read, write)| (WinHandle(read), WinHandle(write)))
            .map_err(|e| e.to_pyexception(vm))
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
        host_winapi::duplicate_handle(
            src_process.0,
            src.0,
            target_process.0,
            access,
            inherit,
            options.unwrap_or(0),
        )
        .map(WinHandle)
        .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn GetACP() -> u32 {
        host_winapi::get_acp()
    }

    #[pyfunction]
    fn GetCurrentProcess() -> WinHandle {
        WinHandle(host_winapi::get_current_process())
    }

    #[pyfunction]
    fn GetFileType(
        h: WinHandle,
        vm: &VirtualMachine,
    ) -> PyResult<windows_sys::Win32::Storage::FileSystem::FILE_TYPE> {
        host_winapi::get_file_type(h.0).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn GetLastError() -> u32 {
        host_winapi::get_last_error()
    }

    #[pyfunction]
    fn GetVersion() -> u32 {
        host_winapi::get_version()
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
        si_attr!(hStdInput, isize);
        si_attr!(hStdOutput, isize);
        si_attr!(hStdError, isize);

        let mut env = args
            .env_mapping
            .map(|m| getenvironment(m, vm))
            .transpose()?;
        let env = env.as_mut().map_or_else(null_mut, |v| v.as_mut_ptr());

        let mut attrlist =
            getattributelist(args.startup_info.get_attr("lpAttributeList", vm)?, vm)?;
        si.lpAttributeList = attrlist
            .as_mut()
            .map_or_else(null_mut, |l| l.as_mut_ptr() as _);

        let wstr = |s: PyStrRef| {
            let ws = widestring::WideCString::from_str(s.expect_str())
                .map_err(|err| err.to_pyexception(vm))?;
            Ok(ws.into_vec_with_nul())
        };

        // Validate no embedded null bytes in command name and command line
        if let Some(ref name) = args.name
            && name.as_bytes().contains(&0)
        {
            return Err(crate::exceptions::cstring_error(vm));
        }
        if let Some(ref cmd) = args.command_line
            && cmd.as_bytes().contains(&0)
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
            host_winapi::create_process_w(
                app_name,
                command_line,
                args.inherit_handles,
                args.creation_flags
                    | windows_sys::Win32::System::Threading::EXTENDED_STARTUPINFO_PRESENT
                    | windows_sys::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT,
                env,
                current_dir,
                &mut si as *mut _ as *mut _,
            )
            .map_err(|e| e.to_pyexception(vm))?
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
        host_winapi::open_process(desired_access, inherit_handle, process_id)
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn ExitProcess(exit_code: u32) {
        host_winapi::exit_process(exit_code)
    }

    #[pyfunction]
    fn NeedCurrentDirectoryForExePath(exe_name: PyStrRef) -> bool {
        let exe_name = exe_name.as_wtf8().to_wide_with_nul();
        host_winapi::need_current_directory_for_exe_path_w(exe_name.as_ptr())
    }

    #[pyfunction]
    fn CreateJunction(
        src_path: PyStrRef,
        dest_path: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let src_path = std::path::Path::new(src_path.expect_str());
        let dest_path = std::path::Path::new(dest_path.expect_str());
        host_winapi::create_junction(src_path, dest_path).map_err(|e| e.to_pyexception(vm))
    }

    fn getenvironment(env: ArgMapping, vm: &VirtualMachine) -> PyResult<Vec<u16>> {
        let keys = env.mapping().keys(vm)?;
        let values = env.mapping().values(vm)?;

        let keys = ArgSequence::try_from_object(vm, keys)?.into_vec();
        let values = ArgSequence::try_from_object(vm, values)?.into_vec();

        if keys.len() != values.len() {
            return Err(vm.new_runtime_error("environment changed size during iteration"));
        }

        let mut entries = Vec::with_capacity(keys.len());
        for (k, v) in keys.into_iter().zip(values) {
            let k = PyStrRef::try_from_object(vm, k)?;
            let k = k.expect_str().to_owned();
            let v = PyStrRef::try_from_object(vm, v)?;
            let v = v.expect_str().to_owned();
            entries.push((k, v));
        }

        host_winapi::build_environment_block(entries).map_err(|err| match err {
            host_winapi::BuildEnvironmentBlockError::ContainsNul => {
                crate::exceptions::cstring_error(vm)
            }
            host_winapi::BuildEnvironmentBlockError::IllegalName => {
                vm.new_value_error("illegal environment variable name")
            }
        })
    }

    fn getattributelist(
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Option<host_winapi::AttrList>> {
        let Some(mapping) = <Option<ArgMapping>>::try_from_object(vm, obj)? else {
            return Ok(None);
        };
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

        host_winapi::create_handle_list_attribute_list(handlelist).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn WaitForSingleObject(h: WinHandle, ms: i64, vm: &VirtualMachine) -> PyResult<u32> {
        // Negative values (e.g., -1) map to INFINITE (0xFFFFFFFF)
        let ms = if ms < 0 {
            windows_sys::Win32::System::Threading::INFINITE
        } else if ms > u32::MAX as i64 {
            return Err(vm.new_overflow_error("timeout value is too large"));
        } else {
            ms as u32
        };
        host_winapi::wait_for_single_object(h.0, ms).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn WaitForMultipleObjects(
        handle_seq: ArgSequence<isize>,
        wait_all: bool,
        milliseconds: u32,
        vm: &VirtualMachine,
    ) -> PyResult<u32> {
        let handles: Vec<HANDLE> = handle_seq
            .into_vec()
            .into_iter()
            .map(|h| h as HANDLE)
            .collect();

        if handles.is_empty() {
            return Err(vm.new_value_error("handle_seq must not be empty"));
        }

        if handles.len() > 64 {
            return Err(vm.new_value_error("WaitForMultipleObjects supports at most 64 handles"));
        }

        host_winapi::wait_for_multiple_objects(&handles, wait_all, milliseconds)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn GetExitCodeProcess(h: WinHandle, vm: &VirtualMachine) -> PyResult<u32> {
        host_winapi::get_exit_code_process(h.0).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn TerminateProcess(h: WinHandle, exit_code: u32) -> WindowsSysResult<i32> {
        WindowsSysResult(host_winapi::terminate_process(h.0, exit_code))
    }

    #[pyfunction]
    fn CreateJobObject(
        _security_attributes: PyObjectRef,
        name: OptionalArg<Option<PyStrRef>>,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        let name = name.flatten().map(|name| name.as_wtf8().to_wide_with_nul());
        host_winapi::create_job_object_w(name.as_ref().map_or(null(), |name| name.as_ptr()))
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn AssignProcessToJobObject(
        job: WinHandle,
        process: WinHandle,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        host_winapi::assign_process_to_job_object(job.0, process.0)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn TerminateJobObject(job: WinHandle, exit_code: u32, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::terminate_job_object(job.0, exit_code).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn SetJobObjectKillOnClose(job: WinHandle, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::set_job_object_kill_on_close(job.0).map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn GetModuleFileName(handle: isize, vm: &VirtualMachine) -> PyResult<String> {
        let mut path: Vec<u16> = vec![0; MAX_PATH as usize];

        let length = host_winapi::get_module_file_name(handle as _, &mut path);
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
    ) -> PyResult<WinHandle> {
        let name_wide = name.as_wtf8().to_wide_with_nul();
        host_winapi::open_mutex_w(desired_access, inherit_handle, name_wide.as_ptr())
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    #[pyfunction]
    fn ReleaseMutex(handle: WinHandle) -> WindowsSysResult<i32> {
        WindowsSysResult(host_winapi::release_mutex(handle.0))
    }

    // LOCALE_NAME_INVARIANT is an empty string in Windows API
    #[pyattr]
    const LOCALE_NAME_INVARIANT: &str = "";

    #[pyattr]
    const LOCALE_NAME_SYSTEM_DEFAULT: &str = "!x-sys-default-locale";

    #[pyattr(name = "LOCALE_NAME_USER_DEFAULT")]
    fn locale_name_user_default(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }

    /// LCMapStringEx - Map a string to another string using locale-specific rules
    /// This is used by ntpath.normcase() for proper Windows case conversion
    #[pyfunction]
    fn LCMapStringEx(
        locale: PyStrRef,
        flags: u32,
        src: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        use windows_sys::Win32::Globalization::{
            LCMAP_BYTEREV, LCMAP_HASH, LCMAP_SORTHANDLE, LCMAP_SORTKEY,
        };

        // Reject unsupported flags
        if flags & (LCMAP_SORTHANDLE | LCMAP_HASH | LCMAP_BYTEREV | LCMAP_SORTKEY) != 0 {
            return Err(vm.new_value_error("unsupported flags"));
        }

        // Use ToWideString which properly handles WTF-8 (including surrogates)
        let locale_wide = locale.as_wtf8().to_wide_with_nul();
        let src_wide = src.as_wtf8().to_wide();

        if src_wide.len() > i32::MAX as usize {
            return Err(vm.new_overflow_error("input string is too long"));
        }

        let dest = host_winapi::lc_map_string_ex(
            locale_wide.as_ptr(),
            flags,
            src_wide.as_ptr(),
            src_wide.len() as i32,
        )
        .map_err(|e| e.to_pyexception(vm))?;

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
        let name_wide = args.name.as_wtf8().to_wide_with_nul();
        host_winapi::create_named_pipe_w(
            name_wide.as_ptr(),
            args.open_mode,
            args.pipe_mode,
            args.max_instances,
            args.out_buffer_size,
            args.in_buffer_size,
            args.default_timeout,
        )
        .map(WinHandle)
        .map_err(|e| e.to_pyexception(vm))
    }

    // ==================== Overlapped class ====================
    // Used for asynchronous I/O operations (ConnectNamedPipe, ReadFile, WriteFile)

    #[pyattr]
    #[pyclass(name = "Overlapped", module = "_winapi")]
    #[derive(Debug, PyPayload)]
    struct Overlapped {
        inner: PyMutex<host_overlapped::Operation>,
    }

    #[pyclass(with(Constructor))]
    impl Overlapped {
        fn new_with_handle(handle: HANDLE, vm: &VirtualMachine) -> PyResult<Self> {
            host_overlapped::Operation::new(handle)
                .map(|inner| Overlapped {
                    inner: PyMutex::new(inner),
                })
                .map_err(|e| e.to_pyexception(vm))
        }

        #[pymethod]
        fn GetOverlappedResult(&self, wait: bool, vm: &VirtualMachine) -> PyResult<(u32, u32)> {
            let mut inner = self.inner.lock();
            inner
                .get_result(wait)
                .map(|result| (result.transferred, result.error))
                .map_err(|e| e.to_pyexception(vm))
        }

        #[pymethod]
        fn getbuffer(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            let inner = self.inner.lock();
            if !inner.is_completed() {
                return Err(vm.new_value_error(
                    "can't get read buffer before GetOverlappedResult() signals the operation completed",
                ));
            }
            Ok(inner
                .read_buffer()
                .map(|buf| vm.ctx.new_bytes(buf.to_vec()).into()))
        }

        #[pymethod]
        fn cancel(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut inner = self.inner.lock();
            inner.cancel().map_err(|e| e.to_pyexception(vm))
        }

        #[pygetset]
        fn event(&self) -> isize {
            let inner = self.inner.lock();
            inner.event() as isize
        }
    }

    impl Constructor for Overlapped {
        type Args = ();

        fn py_new(
            _cls: &Py<crate::builtins::PyType>,
            _args: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Overlapped::new_with_handle(null_mut(), vm)
        }
    }

    /// ConnectNamedPipe - Wait for a client to connect to a named pipe
    #[derive(FromArgs)]
    struct ConnectNamedPipeArgs {
        #[pyarg(any)]
        handle: WinHandle,
        #[pyarg(any, optional)]
        overlapped: OptionalArg<bool>,
    }

    #[pyfunction]
    fn ConnectNamedPipe(args: ConnectNamedPipeArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let handle = args.handle;
        let use_overlapped = args.overlapped.unwrap_or(false);

        if use_overlapped {
            let ov = Overlapped::new_with_handle(handle.0, vm)?;
            {
                let mut inner = ov.inner.lock();
                inner
                    .connect_named_pipe()
                    .map_err(|e| e.to_pyexception(vm))?;
            }
            Ok(ov.into_pyobject(vm))
        } else {
            host_winapi::connect_named_pipe(handle.0).map_err(|e| e.to_pyexception(vm))?;
            Ok(vm.ctx.none())
        }
    }

    /// Helper for GetShortPathName and GetLongPathName
    fn path_name_result_to_pystr(wide: Vec<u16>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // Convert UTF-16 back to WTF-8 (handles surrogates properly)
        let result_str = Wtf8Buf::from_wide(&wide);
        Ok(vm.ctx.new_str(result_str))
    }

    /// GetShortPathName - Return the short version of the provided path.
    #[pyfunction]
    fn GetShortPathName(path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let path_wide = path.as_wtf8().to_wide_with_nul();
        let wide = host_winapi::get_short_path_name_w(path_wide.as_ptr())
            .map_err(|e| e.to_pyexception(vm))?;
        path_name_result_to_pystr(wide, vm)
    }

    /// GetLongPathName - Return the long version of the provided path.
    #[pyfunction]
    fn GetLongPathName(path: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let path_wide = path.as_wtf8().to_wide_with_nul();
        let wide = host_winapi::get_long_path_name_w(path_wide.as_ptr())
            .map_err(|e| e.to_pyexception(vm))?;
        path_name_result_to_pystr(wide, vm)
    }

    /// WaitNamedPipe - Wait for an instance of a named pipe to become available.
    #[pyfunction]
    fn WaitNamedPipe(name: PyStrRef, timeout: u32, vm: &VirtualMachine) -> PyResult<()> {
        let name_wide = name.as_wtf8().to_wide_with_nul();
        host_winapi::wait_named_pipe_w(name_wide.as_ptr(), timeout)
            .map_err(|e| e.to_pyexception(vm))
    }

    /// PeekNamedPipe - Peek at data in a named pipe without removing it.
    #[pyfunction]
    fn PeekNamedPipe(
        handle: WinHandle,
        size: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let size = size.unwrap_or(0);

        if size < 0 {
            return Err(vm.new_value_error("negative size"));
        }

        if size > 0 {
            let result = host_winapi::peek_named_pipe(handle.0, Some(size as u32))
                .map_err(|e| e.to_pyexception(vm))?;
            let buf = result.data.unwrap_or_default();
            let bytes: PyObjectRef = vm.ctx.new_bytes(buf).into();
            Ok(vm
                .ctx
                .new_tuple(vec![
                    bytes,
                    vm.ctx.new_int(result.available).into(),
                    vm.ctx.new_int(result.left_this_message).into(),
                ])
                .into())
        } else {
            let result =
                host_winapi::peek_named_pipe(handle.0, None).map_err(|e| e.to_pyexception(vm))?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.new_int(result.available).into(),
                    vm.ctx.new_int(result.left_this_message).into(),
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
    ) -> PyResult<WinHandle> {
        let _ = security_attributes; // Ignored, always NULL

        let name_wide = name.map(|n| n.as_wtf8().to_wide_with_nul());
        let name_ptr = name_wide.as_ref().map_or(null(), |n| n.as_ptr());
        host_winapi::create_event_w(manual_reset, initial_state, name_ptr)
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    /// SetEvent - Set the specified event object to the signaled state.
    #[pyfunction]
    fn SetEvent(event: WinHandle, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::set_event(event.0).map_err(|e| e.to_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct WriteFileArgs {
        #[pyarg(any)]
        handle: WinHandle,
        #[pyarg(any)]
        buffer: crate::function::ArgBytesLike,
        #[pyarg(any, default = false)]
        overlapped: bool,
    }

    /// WriteFile - Write data to a file or I/O device.
    #[pyfunction]
    fn WriteFile(args: WriteFileArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let handle = args.handle;
        let use_overlapped = args.overlapped;
        let buf = args.buffer.borrow_buf();

        if use_overlapped {
            let ov = Overlapped::new_with_handle(handle.0, vm)?;
            let err = {
                let mut inner = ov.inner.lock();
                inner.write(&buf).map_err(|e| e.to_pyexception(vm))?
            };

            let result = vm
                .ctx
                .new_tuple(vec![ov.into_pyobject(vm), vm.ctx.new_int(err).into()]);
            return Ok(result.into());
        }

        let result = host_winapi::write_file(handle.0, &buf).map_err(|e| e.to_pyexception(vm))?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(result.written).into(),
                vm.ctx.new_int(result.error).into(),
            ])
            .into())
    }

    #[derive(FromArgs)]
    struct ReadFileArgs {
        #[pyarg(any)]
        handle: WinHandle,
        #[pyarg(any)]
        size: u32,
        #[pyarg(any, default = false)]
        overlapped: bool,
    }

    /// ReadFile - Read data from a file or I/O device.
    #[pyfunction]
    fn ReadFile(args: ReadFileArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let handle = args.handle;
        let size = args.size;
        let use_overlapped = args.overlapped;

        if use_overlapped {
            let ov = Overlapped::new_with_handle(handle.0, vm)?;
            let err = {
                let mut inner = ov.inner.lock();
                inner.read(size).map_err(|e| e.to_pyexception(vm))?
            };
            let result = vm
                .ctx
                .new_tuple(vec![ov.into_pyobject(vm), vm.ctx.new_int(err).into()]);
            return Ok(result.into());
        }

        let result = host_winapi::read_file(handle.0, size).map_err(|e| e.to_pyexception(vm))?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_bytes(result.data).into(),
                vm.ctx.new_int(result.error).into(),
            ])
            .into())
    }

    /// SetNamedPipeHandleState - Set the read mode and other options of a named pipe.
    #[pyfunction]
    fn SetNamedPipeHandleState(
        named_pipe: WinHandle,
        mode: PyObjectRef,
        max_collection_count: PyObjectRef,
        collect_data_timeout: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let objs = [&mode, &max_collection_count, &collect_data_timeout];
        let mut values = [None; 3];
        for (index, obj) in objs.iter().enumerate() {
            if !vm.is_none(obj) {
                values[index] = Some(u32::try_from_object(vm, (*obj).clone())?);
            }
        }
        host_winapi::set_named_pipe_handle_state(named_pipe.0, values[0], values[1], values[2])
            .map_err(|e| e.to_pyexception(vm))
    }

    /// ResetEvent - Reset the specified event object to the nonsignaled state.
    #[pyfunction]
    fn ResetEvent(event: WinHandle, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::reset_event(event.0).map_err(|e| e.to_pyexception(vm))
    }

    /// CreateMutexW - Create or open a named or unnamed mutex object.
    #[pyfunction]
    fn CreateMutexW(
        security_attributes: isize,
        initial_owner: bool,
        name: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        let _ = security_attributes;
        let name_wide = name.map(|n| n.as_wtf8().to_wide_with_nul());
        let name_ptr = name_wide.as_ref().map_or(null(), |n| n.as_ptr());
        host_winapi::create_mutex_w(initial_owner, name_ptr)
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    /// OpenEventW - Open an existing named event object.
    #[pyfunction]
    fn OpenEventW(
        desired_access: u32,
        inherit_handle: bool,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        let name_wide = name.as_wtf8().to_wide_with_nul();
        host_winapi::open_event_w(desired_access, inherit_handle, name_wide.as_ptr())
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
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
        use windows_sys::Win32::System::Threading::INFINITE as WIN_INFINITE;

        let milliseconds = milliseconds.unwrap_or(WIN_INFINITE);

        // Get handles from sequence
        let seq = ArgSequence::<isize>::try_from_object(vm, handle_seq)?;
        let handles: Vec<HANDLE> = seq
            .into_vec()
            .into_iter()
            .map(|handle| handle as _)
            .collect();
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

        #[cfg(feature = "threading")]
        let sigint_event = {
            let is_main = crate::stdlib::_thread::get_ident() == vm.state.main_thread_ident.load();
            if is_main {
                let handle = crate::signal::get_sigint_event()
                    .map(|handle| handle as HANDLE)
                    .unwrap_or_else(|| {
                        let handle = host_winapi::create_event_w(true, false, null())
                            .unwrap_or(core::ptr::null_mut());
                        if !handle.is_null() {
                            crate::signal::set_sigint_event(handle as isize);
                        }
                        handle
                    });
                if handle.is_null() { None } else { Some(handle) }
            } else {
                None
            }
        };
        #[cfg(not(feature = "threading"))]
        let sigint_event: Option<HANDLE> = None;

        match host_winapi::batched_wait_for_multiple_objects(
            &handles,
            wait_all,
            milliseconds,
            sigint_event,
        ) {
            Ok(host_winapi::BatchedWaitResult::All) => Ok(vm.ctx.none()),
            Ok(host_winapi::BatchedWaitResult::Indices(indices)) => Ok(vm
                .ctx
                .new_list(
                    indices
                        .into_iter()
                        .map(|index| vm.ctx.new_int(index).into())
                        .collect(),
                )
                .into()),
            Err(host_winapi::BatchedWaitError::Timeout) => Err(vm
                .new_os_subtype_error(
                    vm.ctx.exceptions.timeout_error.to_owned(),
                    None,
                    "timed out",
                )
                .upcast()),
            Err(host_winapi::BatchedWaitError::Interrupted) => Err(vm
                .new_errno_error(libc::EINTR, "Interrupted system call")
                .upcast()),
            Err(host_winapi::BatchedWaitError::Os(err)) => Err(vm.new_os_error(err as i32)),
        }
    }

    /// CreateFileMapping - Create or open a named or unnamed file mapping object.
    #[pyfunction]
    fn CreateFileMapping(
        file_handle: WinHandle,
        _security_attributes: PyObjectRef,
        protect: u32,
        max_size_high: u32,
        max_size_low: u32,
        name: Option<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        if let Some(ref n) = name
            && n.as_bytes().contains(&0)
        {
            return Err(
                vm.new_value_error("CreateFileMapping: name must not contain null characters")
            );
        }
        let name_wide = name.as_ref().map(|n| n.as_wtf8().to_wide_with_nul());
        let name_ptr = name_wide.as_ref().map_or(null(), |n| n.as_ptr());
        host_winapi::create_file_mapping_w(
            file_handle.0,
            protect,
            max_size_high,
            max_size_low,
            name_ptr,
        )
        .map(WinHandle)
        .map_err(|e| e.to_pyexception(vm))
    }

    /// OpenFileMapping - Open a named file mapping object.
    #[pyfunction]
    fn OpenFileMapping(
        desired_access: u32,
        inherit_handle: bool,
        name: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<WinHandle> {
        if name.as_bytes().contains(&0) {
            return Err(
                vm.new_value_error("OpenFileMapping: name must not contain null characters")
            );
        }
        let name_wide = name.as_wtf8().to_wide_with_nul();
        host_winapi::open_file_mapping_w(desired_access, inherit_handle, name_wide.as_ptr())
            .map(WinHandle)
            .map_err(|e| e.to_pyexception(vm))
    }

    /// MapViewOfFile - Map a view of a file mapping into the address space.
    #[pyfunction]
    fn MapViewOfFile(
        file_map: WinHandle,
        desired_access: u32,
        file_offset_high: u32,
        file_offset_low: u32,
        number_bytes: usize,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        host_winapi::map_view_of_file(
            file_map.0,
            desired_access,
            file_offset_high,
            file_offset_low,
            number_bytes,
        )
        .map_err(|e| e.to_pyexception(vm))
    }

    /// UnmapViewOfFile - Unmap a mapped view of a file.
    #[pyfunction]
    fn UnmapViewOfFile(address: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::unmap_view_of_file(address).map_err(|e| e.to_pyexception(vm))
    }

    /// VirtualQuerySize - Return the size of a memory region.
    #[pyfunction]
    fn VirtualQuerySize(address: isize, vm: &VirtualMachine) -> PyResult<usize> {
        host_winapi::virtual_query_size(address).map_err(|e| e.to_pyexception(vm))
    }

    /// CopyFile2 - Copy a file with extended parameters.
    #[pyfunction]
    fn CopyFile2(
        existing_file_name: PyStrRef,
        new_file_name: PyStrRef,
        flags: u32,
        _progress_routine: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let src_wide = existing_file_name.as_wtf8().to_wide_with_nul();
        let dst_wide = new_file_name.as_wtf8().to_wide_with_nul();
        host_winapi::copy_file2(src_wide.as_ptr(), dst_wide.as_ptr(), flags)
            .map_err(|e| e.to_pyexception(vm))
    }

    /// _mimetypes_read_windows_registry - Read MIME type associations from registry.
    #[pyfunction]
    fn _mimetypes_read_windows_registry(
        on_type_read: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        host_winapi::read_windows_mimetype_registry_in_batches(|entries| {
            for (mime_type, ext) in entries.drain(..) {
                on_type_read.call((vm.ctx.new_str(mime_type), vm.ctx.new_str(ext)), vm)?;
            }
            Ok(())
        })
        .map_err(|err| match err {
            host_winapi::MimeRegistryReadError::Os(err) => vm.new_os_error(err as i32),
            host_winapi::MimeRegistryReadError::Callback(err) => err,
        })
    }
}

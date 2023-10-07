#![allow(non_snake_case)]
pub(crate) use _winapi::make_module;

#[pymodule]
mod _winapi {
    use crate::{
        builtins::PyStrRef,
        common::windows::ToWideString,
        convert::{ToPyException, ToPyResult},
        function::{ArgMapping, ArgSequence, OptionalArg},
        stdlib::os::errno_err,
        windows::WindowsSysResult,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use std::ptr::{null, null_mut};
    use windows::{
        core::PCWSTR,
        Win32::Foundation::{HANDLE, HINSTANCE, MAX_PATH},
    };
    use windows_sys::Win32::Foundation::{BOOL, HANDLE as RAW_HANDLE};

    #[pyattr]
    use windows_sys::Win32::{
        Foundation::{
            DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE,
            ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NETNAME_DELETED, ERROR_NO_DATA,
            ERROR_NO_SYSTEM_RESOURCES, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
            ERROR_PIPE_CONNECTED, ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE, STILL_ACTIVE,
            WAIT_ABANDONED, WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
            FILE_GENERIC_WRITE, FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_REMOTE,
            FILE_TYPE_UNKNOWN, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND, SYNCHRONIZE,
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
                PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            SystemServices::LOCALE_NAME_MAX_LENGTH,
            Threading::{
                ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
                CREATE_BREAKAWAY_FROM_JOB, CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE,
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, HIGH_PRIORITY_CLASS,
                IDLE_PRIORITY_CLASS, INFINITE, NORMAL_PRIORITY_CLASS, PROCESS_DUP_HANDLE,
                REALTIME_PRIORITY_CLASS, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
            },
        },
        UI::WindowsAndMessaging::SW_HIDE,
    };

    #[pyfunction]
    fn CloseHandle(handle: HANDLE) -> WindowsSysResult<BOOL> {
        WindowsSysResult(unsafe { windows_sys::Win32::Foundation::CloseHandle(handle.0) })
    }

    #[pyfunction]
    fn GetStdHandle(
        std_handle: windows_sys::Win32::System::Console::STD_HANDLE,
    ) -> WindowsSysResult<RAW_HANDLE> {
        WindowsSysResult(unsafe { windows_sys::Win32::System::Console::GetStdHandle(std_handle) })
    }

    #[pyfunction]
    fn CreatePipe(
        _pipe_attrs: PyObjectRef,
        size: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(HANDLE, HANDLE)> {
        let (read, write) = unsafe {
            let mut read = std::mem::MaybeUninit::<isize>::uninit();
            let mut write = std::mem::MaybeUninit::<isize>::uninit();
            WindowsSysResult(windows_sys::Win32::System::Pipes::CreatePipe(
                read.as_mut_ptr(),
                write.as_mut_ptr(),
                std::ptr::null(),
                size,
            ))
            .to_pyresult(vm)?;
            (read.assume_init(), write.assume_init())
        };
        Ok((HANDLE(read), HANDLE(write)))
    }

    #[pyfunction]
    fn DuplicateHandle(
        (src_process, src): (HANDLE, HANDLE),
        target_process: HANDLE,
        access: u32,
        inherit: BOOL,
        options: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<HANDLE> {
        let target = unsafe {
            let mut target = std::mem::MaybeUninit::<isize>::uninit();
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
        Ok(HANDLE(target))
    }

    #[pyfunction]
    fn GetCurrentProcess() -> HANDLE {
        unsafe { windows::Win32::System::Threading::GetCurrentProcess() }
    }

    #[pyfunction]
    fn GetFileType(
        h: HANDLE,
        vm: &VirtualMachine,
    ) -> PyResult<windows_sys::Win32::Storage::FileSystem::FILE_TYPE> {
        let file_type = unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(h.0) };
        if file_type == 0 && unsafe { windows_sys::Win32::Foundation::GetLastError() } != 0 {
            Err(errno_err(vm))
        } else {
            Ok(file_type)
        }
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
    ) -> PyResult<(HANDLE, HANDLE, u32, u32)> {
        let mut si: windows_sys::Win32::System::Threading::STARTUPINFOEXW =
            unsafe { std::mem::zeroed() };
        si.StartupInfo.cb = std::mem::size_of_val(&si) as _;

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
            let mut procinfo = std::mem::MaybeUninit::uninit();
            WindowsSysResult(windows_sys::Win32::System::Threading::CreateProcessW(
                app_name,
                command_line,
                std::ptr::null(),
                std::ptr::null(),
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
            HANDLE(procinfo.hProcess),
            HANDLE(procinfo.hThread),
            procinfo.dwProcessId,
            procinfo.dwThreadId,
        ))
    }

    fn getenvironment(env: ArgMapping, vm: &VirtualMachine) -> PyResult<Vec<u16>> {
        let keys = env.mapping().keys(vm)?;
        let values = env.mapping().values(vm)?;

        let keys = ArgSequence::try_from_object(vm, keys)?.into_vec();
        let values = ArgSequence::try_from_object(vm, values)?.into_vec();

        if keys.len() != values.len() {
            return Err(
                vm.new_runtime_error("environment changed size during iteration".to_owned())
            );
        }

        let mut out = widestring::WideString::new();
        for (k, v) in keys.into_iter().zip(values.into_iter()) {
            let k = PyStrRef::try_from_object(vm, k)?;
            let k = k.as_str();
            let v = PyStrRef::try_from_object(vm, v)?;
            let v = v.as_str();
            if k.contains('\0') || v.contains('\0') {
                return Err(crate::exceptions::cstring_error(vm));
            }
            if k.is_empty() || k[1..].contains('=') {
                return Err(vm.new_value_error("illegal environment variable name".to_owned()));
            }
            out.push_str(k);
            out.push_str("=");
            out.push_str(v);
            out.push_str("\0");
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
                    let mut size = std::mem::MaybeUninit::uninit();
                    let result = WindowsSysResult(
                        windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList(
                            std::ptr::null_mut(),
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
                    return Err(errno_err(vm));
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
                            (handlelist.len() * std::mem::size_of::<HANDLE>()) as _,
                            std::ptr::null_mut(),
                            std::ptr::null(),
                        )
                    })
                    .into_pyresult(vm)?;
                }
                Ok(attrs)
            })
            .transpose()
    }

    #[pyfunction]
    fn WaitForSingleObject(h: HANDLE, ms: u32, vm: &VirtualMachine) -> PyResult<u32> {
        let ret = unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(h.0, ms) };
        if ret == windows_sys::Win32::Foundation::WAIT_FAILED {
            Err(errno_err(vm))
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn GetExitCodeProcess(h: HANDLE, vm: &VirtualMachine) -> PyResult<u32> {
        unsafe {
            let mut ec = std::mem::MaybeUninit::uninit();
            WindowsSysResult(windows_sys::Win32::System::Threading::GetExitCodeProcess(
                h.0,
                ec.as_mut_ptr(),
            ))
            .to_pyresult(vm)?;
            Ok(ec.assume_init())
        }
    }

    #[pyfunction]
    fn TerminateProcess(h: HANDLE, exit_code: u32) -> WindowsSysResult<BOOL> {
        WindowsSysResult(unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(h.0, exit_code)
        })
    }

    // TODO: ctypes.LibraryLoader.LoadLibrary
    #[allow(dead_code)]
    fn LoadLibrary(path: PyStrRef, vm: &VirtualMachine) -> PyResult<isize> {
        let path = path.as_str().to_wides_with_nul();
        let handle = unsafe {
            windows::Win32::System::LibraryLoader::LoadLibraryW(PCWSTR::from_raw(path.as_ptr()))
                .unwrap()
        };
        if handle.is_invalid() {
            return Err(vm.new_runtime_error("LoadLibrary failed".to_owned()));
        }
        Ok(handle.0)
    }

    #[pyfunction]
    fn GetModuleFileName(handle: isize, vm: &VirtualMachine) -> PyResult<String> {
        let mut path: Vec<u16> = vec![0; MAX_PATH as usize];
        let handle = HINSTANCE(handle);

        let length =
            unsafe { windows::Win32::System::LibraryLoader::GetModuleFileNameW(handle, &mut path) };
        if length == 0 {
            return Err(vm.new_runtime_error("GetModuleFileName failed".to_owned()));
        }

        let (path, _) = path.split_at(length as usize);
        Ok(String::from_utf16(path).unwrap())
    }
}

#![allow(non_snake_case)]
pub(crate) use _winapi::make_module;

#[pymodule]
mod _winapi {
    use crate::{
        builtins::{PyInt, PyStrRef},
        common::windows::ToWideString,
        convert::{ToPyException, ToPyObject},
        function::{ArgMapping, ArgSequence, OptionalArg},
        stdlib::os::errno_err,
        PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject, VirtualMachine,
    };
    use num_traits::ToPrimitive;
    use std::ptr::{null, null_mut};
    use windows::{
        core::PCWSTR,
        Win32::Foundation,
        Win32::Foundation::BOOL,
        Win32::Foundation::DUPLICATE_HANDLE_OPTIONS,
        Win32::Foundation::NTSTATUS,
        Win32::Foundation::WIN32_ERROR,
        Win32::Foundation::{HINSTANCE, MAX_PATH},
        Win32::Storage::FileSystem,
        Win32::Storage::FileSystem::FILE_ACCESS_FLAGS,
        Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
        Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        Win32::System::Console::STD_HANDLE,
        Win32::System::LibraryLoader::{GetModuleFileNameW, LoadLibraryW},
        Win32::System::Memory::FILE_MAP,
        Win32::System::Memory::PAGE_PROTECTION_FLAGS,
        Win32::System::Memory::PAGE_TYPE,
        Win32::System::Memory::VIRTUAL_ALLOCATION_TYPE,
        Win32::System::Pipes,
        Win32::System::Pipes::NAMED_PIPE_MODE,
        Win32::System::Threading::PROCESS_ACCESS_RIGHTS,
        Win32::System::Threading::PROCESS_CREATION_FLAGS,
        Win32::System::Threading::STARTUPINFOW_FLAGS,
        Win32::System::{Console, Threading},
        Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD,
    };

    #[pyattr]
    use windows::Win32::{
        Foundation::{
            DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, ERROR_ALREADY_EXISTS, ERROR_BROKEN_PIPE,
            ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NETNAME_DELETED, ERROR_NO_DATA,
            ERROR_NO_SYSTEM_RESOURCES, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
            ERROR_PIPE_CONNECTED, ERROR_SEM_TIMEOUT, STILL_ACTIVE, WAIT_ABANDONED,
            WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
            FILE_GENERIC_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND,
            SYNCHRONIZE,
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
            SystemServices::{GENERIC_READ, GENERIC_WRITE, LOCALE_NAME_MAX_LENGTH},
            Threading::{
                ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
                CREATE_BREAKAWAY_FROM_JOB, CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE,
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, HIGH_PRIORITY_CLASS,
                IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_DUP_HANDLE,
                REALTIME_PRIORITY_CLASS, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
            },
            WindowsProgramming::{
                FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_REMOTE,
                FILE_TYPE_UNKNOWN, INFINITE,
            },
        },
        UI::WindowsAndMessaging::SW_HIDE,
    };

    fn GetLastError() -> u32 {
        unsafe { winapi::um::errhandlingapi::GetLastError() }
    }

    fn husize(h: Foundation::HANDLE) -> usize {
        h.0 as usize
    }

    trait Convertible {
        fn is_err(&self) -> bool;
    }

    impl Convertible for Foundation::HANDLE {
        fn is_err(&self) -> bool {
            *self == Foundation::INVALID_HANDLE_VALUE
        }
    }
    impl Convertible for i32 {
        fn is_err(&self) -> bool {
            *self == 0
        }
    }
    impl Convertible for BOOL {
        fn is_err(&self) -> bool {
            self.as_bool() == false
        }
    }

    macro_rules! impl_into_pyobject_int {
        ($($t:ty)*) => {$(
            impl ToPyObject for $t {
                fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                    vm.ctx.new_int(self.0).into()
                }
            }
        )*};
    }

    impl_into_pyobject_int!(STARTUPINFOW_FLAGS DUPLICATE_HANDLE_OPTIONS WIN32_ERROR NTSTATUS STD_HANDLE PROCESS_CREATION_FLAGS FILE_FLAGS_AND_ATTRIBUTES FILE_ACCESS_FLAGS FILE_CREATION_DISPOSITION FILE_MAP VIRTUAL_ALLOCATION_TYPE PAGE_TYPE PAGE_PROTECTION_FLAGS NAMED_PIPE_MODE SHOW_WINDOW_CMD PROCESS_ACCESS_RIGHTS);

    impl TryFromBorrowedObject for STARTUPINFOW_FLAGS {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
            let flags = obj
                .payload_if_subclass::<PyInt>(vm)
                .ok_or_else(|| vm.new_type_error("Expected int".to_owned()))?
                .as_bigint()
                .to_u32()
                .ok_or_else(|| vm.new_type_error("Expected int".to_owned()))?;
            Ok(STARTUPINFOW_FLAGS(flags))
        }
    }

    fn cvt<T: Convertible>(vm: &VirtualMachine, res: T) -> PyResult<T> {
        if res.is_err() {
            Err(errno_err(vm))
        } else {
            Ok(res)
        }
    }

    #[pyfunction]
    fn CloseHandle(handle: usize, vm: &VirtualMachine) -> PyResult<()> {
        cvt(vm, unsafe {
            Foundation::CloseHandle(Foundation::HANDLE(handle as isize))
        })
        .map(drop)
    }

    #[pyfunction]
    fn GetStdHandle(std_handle: u32, vm: &VirtualMachine) -> PyResult<usize> {
        let handle = unsafe { Console::GetStdHandle(Console::STD_HANDLE(std_handle)) }
            .map_err(|err| vm.new_type_error(format!("{}", err.message())))?;
        cvt(vm, handle).map(husize)
    }

    #[pyfunction]
    fn CreatePipe(
        _pipe_attrs: PyObjectRef,
        size: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, usize)> {
        let read = null_mut();
        let write = null_mut();
        cvt(vm, unsafe {
            Pipes::CreatePipe(read, write, null_mut(), size)
        })?;
        Ok((read as usize, write as usize))
    }

    #[pyfunction]
    fn DuplicateHandle(
        (src_process, src): (usize, usize),
        target_process: usize,
        access: u32,
        inherit: i32,
        options: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let target = null_mut();
        cvt(vm, unsafe {
            Foundation::DuplicateHandle::<
                Foundation::HANDLE,
                Foundation::HANDLE,
                Foundation::HANDLE,
                BOOL,
            >(
                Foundation::HANDLE(src_process as _),
                Foundation::HANDLE(src as _),
                Foundation::HANDLE(target_process as _),
                target,
                access,
                BOOL(inherit),
                Foundation::DUPLICATE_HANDLE_OPTIONS(options.unwrap_or(0)),
            )
        })?;
        Ok(target as usize)
    }

    #[pyfunction]
    fn GetCurrentProcess() -> usize {
        unsafe { Threading::GetCurrentProcess().0 as usize }
    }

    #[pyfunction]
    fn GetFileType(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
        let ret =
            unsafe { FileSystem::GetFileType::<Foundation::HANDLE>(Foundation::HANDLE(h as _)) };
        if ret == 0 && GetLastError() != 0 {
            Err(errno_err(vm))
        } else {
            Ok(ret)
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
    ) -> PyResult<(usize, usize, u32, u32)> {
        let mut si = Threading::STARTUPINFOEXW::default();
        si.StartupInfo.cb = std::mem::size_of_val(&si) as _;

        macro_rules! si_attr {
            ($attr:ident, $t:ty) => {{
                si.StartupInfo.$attr = <Option<$t>>::try_from_object(
                    vm,
                    args.startup_info.get_attr(stringify!($attr), vm)?,
                )?
                .unwrap_or(0)
            }};
            ($attr:ident, $t:ty, $tt:path) => {{
                si.StartupInfo.$attr = $tt(<Option<$t>>::try_from_object(
                    vm,
                    args.startup_info.get_attr(stringify!($attr), vm)?,
                )?
                .unwrap_or(0));
            }};
        }
        si_attr!(dwFlags, u32, STARTUPINFOW_FLAGS);
        si_attr!(wShowWindow, u16);
        si_attr!(hStdInput, isize, Foundation::HANDLE);
        si_attr!(hStdOutput, isize, Foundation::HANDLE);
        si_attr!(hStdError, isize, Foundation::HANDLE);

        let mut env = args
            .env_mapping
            .map(|m| getenvironment(m, vm))
            .transpose()?;
        let env = env.as_mut().map_or_else(null_mut, |v| v.as_mut_ptr());

        let mut attrlist =
            getattributelist(args.startup_info.get_attr("lpAttributeList", vm)?, vm)?;
        si.lpAttributeList = Threading::LPPROC_THREAD_ATTRIBUTE_LIST(
            attrlist
                .as_mut()
                .map_or_else(null_mut, |l| l.attrlist.as_mut_ptr() as _),
        );

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
            let ret = Threading::CreateProcessW(
                windows::core::PCWSTR(app_name),
                windows::core::PWSTR(command_line),
                null_mut(),
                null_mut(),
                BOOL(args.inherit_handles),
                PROCESS_CREATION_FLAGS(
                    args.creation_flags
                        | Threading::EXTENDED_STARTUPINFO_PRESENT.0
                        | Threading::CREATE_UNICODE_ENVIRONMENT.0,
                ),
                env as _,
                windows::core::PCWSTR(current_dir),
                &mut si as *mut Threading::STARTUPINFOEXW as _,
                procinfo.as_mut_ptr(),
            );
            if !ret.as_bool() {
                return Err(errno_err(vm));
            }
            procinfo.assume_init()
        };

        Ok((
            procinfo.hProcess.0 as usize,
            procinfo.hThread.0 as usize,
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
                Threading::DeleteProcThreadAttributeList::<Threading::LPPROC_THREAD_ATTRIBUTE_LIST>(
                    Threading::LPPROC_THREAD_ATTRIBUTE_LIST(self.attrlist.as_mut_ptr() as _),
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
                let mut size = 0;
                let ret = unsafe {
                    Threading::InitializeProcThreadAttributeList(
                        Threading::LPPROC_THREAD_ATTRIBUTE_LIST(null_mut()),
                        attr_count,
                        0,
                        &mut size,
                    )
                };
                if ret.as_bool() || GetLastError() != Foundation::ERROR_INSUFFICIENT_BUFFER.0 {
                    return Err(errno_err(vm));
                }
                let mut attrlist = vec![0u8; size];
                let ret = unsafe {
                    Threading::InitializeProcThreadAttributeList(
                        Threading::LPPROC_THREAD_ATTRIBUTE_LIST(attrlist.as_mut_ptr() as _),
                        attr_count,
                        0,
                        &mut size,
                    )
                };
                if !ret.as_bool() {
                    return Err(errno_err(vm));
                }
                let mut attrs = AttrList {
                    handlelist,
                    attrlist,
                };
                if let Some(ref mut handlelist) = attrs.handlelist {
                    let ret = unsafe {
                        Threading::UpdateProcThreadAttribute::<
                            Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
                        >(
                            Threading::LPPROC_THREAD_ATTRIBUTE_LIST(
                                attrs.attrlist.as_mut_ptr() as _
                            ),
                            0,
                            (2 & 0xffff) | 0x20000, // PROC_THREAD_ATTRIBUTE_HANDLE_LIST
                            handlelist.as_mut_ptr() as _,
                            (handlelist.len()
                                * std::mem::size_of::<std::os::windows::raw::HANDLE>())
                                as _,
                            null_mut(),
                            null_mut(),
                        )
                    };
                    if !ret.as_bool() {
                        return Err(errno_err(vm));
                    }
                }
                Ok(attrs)
            })
            .transpose()
    }

    #[pyfunction]
    fn WaitForSingleObject(h: usize, ms: u32, vm: &VirtualMachine) -> PyResult<u32> {
        let ret = unsafe {
            Threading::WaitForSingleObject::<Foundation::HANDLE>(Foundation::HANDLE(h as _), ms)
        };
        if ret == Foundation::WAIT_FAILED.0 {
            Err(errno_err(vm))
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn GetExitCodeProcess(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
        let mut ec = 0;
        cvt(vm, unsafe {
            Threading::GetExitCodeProcess::<Foundation::HANDLE>(Foundation::HANDLE(h as _), &mut ec)
        })?;
        Ok(ec)
    }

    #[pyfunction]
    fn TerminateProcess(h: usize, exit_code: u32, vm: &VirtualMachine) -> PyResult<()> {
        cvt(vm, unsafe {
            Threading::TerminateProcess::<Foundation::HANDLE>(Foundation::HANDLE(h as _), exit_code)
        })
        .map(drop)
    }

    // TODO: ctypes.LibraryLoader.LoadLibrary
    #[allow(dead_code)]
    fn LoadLibrary(path: PyStrRef, vm: &VirtualMachine) -> PyResult<isize> {
        let path = path.as_str().to_wides_with_nul();
        let handle = unsafe { LoadLibraryW(PCWSTR::from_raw(path.as_ptr())).unwrap() };
        if handle.is_invalid() {
            return Err(vm.new_runtime_error("LoadLibrary failed".to_owned()));
        }
        Ok(handle.0)
    }

    #[pyfunction]
    fn GetModuleFileName(handle: isize, vm: &VirtualMachine) -> PyResult<String> {
        let mut path: Vec<u16> = vec![0; MAX_PATH as usize];
        let handle = HINSTANCE(handle);

        let length = unsafe { GetModuleFileNameW(handle, &mut path) };
        if length == 0 {
            return Err(vm.new_runtime_error("GetModuleFileName failed".to_owned()));
        }

        let (path, _) = path.split_at(length as usize);
        Ok(String::from_utf16(&path).unwrap())
    }
}

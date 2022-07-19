#![allow(non_snake_case)]
pub(crate) use _winapi::make_module;

#[pymodule]
mod _winapi {
    use crate::{
        builtins::PyStrRef,
        convert::ToPyException,
        function::{ArgMapping, ArgSequence, OptionalArg},
        stdlib::os::errno_err,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use std::ptr::{null, null_mut};
    use winapi::shared::winerror;
    use winapi::um::{
        fileapi, handleapi, namedpipeapi, processenv, processthreadsapi, synchapi, winbase,
        winnt::HANDLE,
    };

    #[pyattr]
    use winapi::{
        shared::winerror::WAIT_TIMEOUT,
        um::{
            winbase::{
                ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
                CREATE_BREAKAWAY_FROM_JOB, CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE,
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS,
                FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_TYPE_CHAR,
                FILE_TYPE_DISK, FILE_TYPE_PIPE, FILE_TYPE_REMOTE, FILE_TYPE_UNKNOWN,
                HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, INFINITE, NORMAL_PRIORITY_CLASS,
                PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
                PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, REALTIME_PRIORITY_CLASS, STARTF_USESHOWWINDOW,
                STARTF_USESTDHANDLES, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
                WAIT_ABANDONED, WAIT_ABANDONED_0, WAIT_OBJECT_0,
            },
            winnt::{
                DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, FILE_GENERIC_READ,
                FILE_GENERIC_WRITE, GENERIC_READ, GENERIC_WRITE, LOCALE_NAME_MAX_LENGTH,
                MEM_COMMIT, MEM_FREE, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE, MEM_RESERVE,
                PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY,
                PAGE_GUARD, PAGE_NOACCESS, PAGE_NOCACHE, PAGE_READONLY, PAGE_READWRITE,
                PAGE_WRITECOMBINE, PAGE_WRITECOPY, PROCESS_DUP_HANDLE, SEC_COMMIT, SEC_IMAGE,
                SEC_LARGE_PAGES, SEC_NOCACHE, SEC_RESERVE, SEC_WRITECOMBINE, SYNCHRONIZE,
            },
            winuser::SW_HIDE,
        },
    };

    fn GetLastError() -> u32 {
        unsafe { winapi::um::errhandlingapi::GetLastError() }
    }

    fn husize(h: HANDLE) -> usize {
        h as usize
    }

    trait Convertible {
        fn is_err(&self) -> bool;
    }

    impl Convertible for HANDLE {
        fn is_err(&self) -> bool {
            *self == handleapi::INVALID_HANDLE_VALUE
        }
    }
    impl Convertible for i32 {
        fn is_err(&self) -> bool {
            *self == 0
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
        cvt(vm, unsafe { handleapi::CloseHandle(handle as HANDLE) }).map(drop)
    }

    #[pyfunction]
    fn GetStdHandle(std_handle: u32, vm: &VirtualMachine) -> PyResult<usize> {
        cvt(vm, unsafe { processenv::GetStdHandle(std_handle) }).map(husize)
    }

    #[pyfunction]
    fn CreatePipe(
        _pipe_attrs: PyObjectRef,
        size: u32,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, usize)> {
        let mut read = null_mut();
        let mut write = null_mut();
        cvt(vm, unsafe {
            namedpipeapi::CreatePipe(&mut read, &mut write, null_mut(), size)
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
        let mut target = null_mut();
        cvt(vm, unsafe {
            handleapi::DuplicateHandle(
                src_process as _,
                src as _,
                target_process as _,
                &mut target,
                access,
                inherit,
                options.unwrap_or(0),
            )
        })?;
        Ok(target as usize)
    }

    #[pyfunction]
    fn GetCurrentProcess() -> usize {
        unsafe { processthreadsapi::GetCurrentProcess() as usize }
    }

    #[pyfunction]
    fn GetFileType(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
        let ret = unsafe { fileapi::GetFileType(h as _) };
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
        let mut si = winbase::STARTUPINFOEXW::default();
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
            let ret = processthreadsapi::CreateProcessW(
                app_name,
                command_line,
                null_mut(),
                null_mut(),
                args.inherit_handles,
                args.creation_flags
                    | winbase::EXTENDED_STARTUPINFO_PRESENT
                    | winbase::CREATE_UNICODE_ENVIRONMENT,
                env as _,
                current_dir,
                &mut si as *mut winbase::STARTUPINFOEXW as _,
                procinfo.as_mut_ptr(),
            );
            if ret == 0 {
                return Err(errno_err(vm));
            }
            procinfo.assume_init()
        };

        Ok((
            procinfo.hProcess as usize,
            procinfo.hThread as usize,
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
                processthreadsapi::DeleteProcThreadAttributeList(self.attrlist.as_mut_ptr() as _)
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
                    processthreadsapi::InitializeProcThreadAttributeList(
                        null_mut(),
                        attr_count,
                        0,
                        &mut size,
                    )
                };
                if ret != 0 || GetLastError() != winerror::ERROR_INSUFFICIENT_BUFFER {
                    return Err(errno_err(vm));
                }
                let mut attrlist = vec![0u8; size];
                let ret = unsafe {
                    processthreadsapi::InitializeProcThreadAttributeList(
                        attrlist.as_mut_ptr() as _,
                        attr_count,
                        0,
                        &mut size,
                    )
                };
                if ret == 0 {
                    return Err(errno_err(vm));
                }
                let mut attrs = AttrList {
                    handlelist,
                    attrlist,
                };
                if let Some(ref mut handlelist) = attrs.handlelist {
                    let ret = unsafe {
                        processthreadsapi::UpdateProcThreadAttribute(
                            attrs.attrlist.as_mut_ptr() as _,
                            0,
                            (2 & 0xffff) | 0x20000, // PROC_THREAD_ATTRIBUTE_HANDLE_LIST
                            handlelist.as_mut_ptr() as _,
                            (handlelist.len() * std::mem::size_of::<HANDLE>()) as _,
                            null_mut(),
                            null_mut(),
                        )
                    };
                    if ret == 0 {
                        return Err(errno_err(vm));
                    }
                }
                Ok(attrs)
            })
            .transpose()
    }

    #[pyfunction]
    fn WaitForSingleObject(h: usize, ms: u32, vm: &VirtualMachine) -> PyResult<u32> {
        let ret = unsafe { synchapi::WaitForSingleObject(h as _, ms) };
        if ret == winbase::WAIT_FAILED {
            Err(errno_err(vm))
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn GetExitCodeProcess(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
        let mut ec = 0;
        cvt(vm, unsafe {
            processthreadsapi::GetExitCodeProcess(h as _, &mut ec)
        })?;
        Ok(ec)
    }

    #[pyfunction]
    fn TerminateProcess(h: usize, exit_code: u32, vm: &VirtualMachine) -> PyResult<()> {
        cvt(vm, unsafe {
            processthreadsapi::TerminateProcess(h as _, exit_code)
        })
        .map(drop)
    }
}

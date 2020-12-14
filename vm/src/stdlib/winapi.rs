#![allow(non_snake_case)]

use std::ptr::{null, null_mut};

use winapi::shared::winerror;
use winapi::um::winnt::HANDLE;
use winapi::um::{
    fileapi, handleapi, namedpipeapi, processenv, processthreadsapi, synchapi, winbase, winnt,
    winuser,
};

use super::os::errno_err;
use crate::builtins::dict::{PyDictRef, PyMapping};
use crate::builtins::pystr::PyStrRef;
use crate::function::OptionalArg;
use crate::pyobject::{BorrowValue, PyObjectRef, PyResult, PySequence, TryFromObject};
use crate::VirtualMachine;

fn GetLastError() -> u32 {
    unsafe { winapi::um::errhandlingapi::GetLastError() }
}

fn husize(h: HANDLE) -> usize {
    h as usize
}

trait Convertable {
    fn is_err(&self) -> bool;
}

impl Convertable for HANDLE {
    fn is_err(&self) -> bool {
        *self == handleapi::INVALID_HANDLE_VALUE
    }
}
impl Convertable for i32 {
    fn is_err(&self) -> bool {
        *self == 0
    }
}

fn cvt<T: Convertable>(vm: &VirtualMachine, res: T) -> PyResult<T> {
    if res.is_err() {
        Err(errno_err(vm))
    } else {
        Ok(res)
    }
}

fn _winapi_CloseHandle(handle: usize, vm: &VirtualMachine) -> PyResult<()> {
    cvt(vm, unsafe { handleapi::CloseHandle(handle as HANDLE) }).map(drop)
}

fn _winapi_GetStdHandle(std_handle: u32, vm: &VirtualMachine) -> PyResult<usize> {
    cvt(vm, unsafe { processenv::GetStdHandle(std_handle) }).map(husize)
}

fn _winapi_CreatePipe(
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

fn _winapi_DuplicateHandle(
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

fn _winapi_GetCurrentProcess() -> usize {
    unsafe { processthreadsapi::GetCurrentProcess() as usize }
}

fn _winapi_GetFileType(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
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
    env_mapping: Option<PyMapping>,
    #[pyarg(positional)]
    current_dir: Option<PyStrRef>,
    #[pyarg(positional)]
    startup_info: PyObjectRef,
}

fn _winapi_CreateProcess(
    args: CreateProcessArgs,
    vm: &VirtualMachine,
) -> PyResult<(usize, usize, u32, u32)> {
    let mut si = winbase::STARTUPINFOEXW::default();
    si.StartupInfo.cb = std::mem::size_of_val(&si) as _;

    macro_rules! si_attr {
        ($attr:ident, $t:ty) => {{
            si.StartupInfo.$attr = <Option<$t>>::try_from_object(
                vm,
                vm.get_attribute(args.startup_info.clone(), stringify!($attr))?,
            )?
            .unwrap_or(0) as _
        }};
        ($attr:ident) => {{
            si.StartupInfo.$attr = <Option<_>>::try_from_object(
                vm,
                vm.get_attribute(args.startup_info.clone(), stringify!($attr))?,
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
        .map(|m| getenvironment(m.into_dict(), vm))
        .transpose()?;
    let env = env.as_mut().map_or_else(null_mut, |v| v.as_mut_ptr());

    let mut attrlist = getattributelist(
        vm.get_attribute(args.startup_info.clone(), "lpAttributeList")?,
        vm,
    )?;
    si.lpAttributeList = attrlist
        .as_mut()
        .map_or_else(null_mut, |l| l.attrlist.as_mut_ptr() as _);

    let wstr = |s: PyStrRef| {
        if s.borrow_value().contains('\0') {
            Err(vm.new_value_error("embedded null character".to_owned()))
        } else {
            Ok(s.borrow_value()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>())
        }
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

    let mut procinfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        processthreadsapi::CreateProcessW(
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
            &mut procinfo,
        )
    };

    if ret == 0 {
        return Err(errno_err(vm));
    }

    Ok((
        procinfo.hProcess as usize,
        procinfo.hThread as usize,
        procinfo.dwProcessId,
        procinfo.dwThreadId,
    ))
}

fn getenvironment(env: PyDictRef, vm: &VirtualMachine) -> PyResult<Vec<u16>> {
    let mut out = vec![];
    for (k, v) in env {
        let k = PyStrRef::try_from_object(vm, k)?;
        let k = k.borrow_value();
        let v = PyStrRef::try_from_object(vm, v)?;
        let v = v.borrow_value();
        if k.contains('\0') || v.contains('\0') {
            return Err(vm.new_value_error("embedded null character".to_owned()));
        }
        if k.len() == 0 || k[1..].contains('=') {
            return Err(vm.new_value_error("illegal environment variable name".to_owned()));
        }
        out.extend(k.encode_utf16());
        out.push(b'=' as u16);
        out.extend(v.encode_utf16());
        out.push(b'\0' as u16);
    }
    out.push(b'\0' as u16);
    Ok(out)
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
    <Option<PyMapping>>::try_from_object(vm, obj)?
        .map(|d| {
            let d = d.into_dict();
            let handlelist = d
                .get_item_option("handle_list", vm)?
                .and_then(|obj| {
                    <Option<PySequence<usize>>>::try_from_object(vm, obj)
                        .and_then(|s| match s {
                            Some(s) if !s.as_slice().is_empty() => Ok(Some(s.into_vec())),
                            _ => Ok(None),
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

fn _winapi_WaitForSingleObject(h: usize, ms: u32, vm: &VirtualMachine) -> PyResult<u32> {
    let ret = unsafe { synchapi::WaitForSingleObject(h as _, ms) };
    if ret == winbase::WAIT_FAILED {
        Err(errno_err(vm))
    } else {
        Ok(ret)
    }
}

fn _winapi_GetExitCodeProcess(h: usize, vm: &VirtualMachine) -> PyResult<u32> {
    let mut ec = 0;
    cvt(vm, unsafe {
        processthreadsapi::GetExitCodeProcess(h as _, &mut ec)
    })?;
    Ok(ec)
}

fn _winapi_TerminateProcess(h: usize, exit_code: u32, vm: &VirtualMachine) -> PyResult<()> {
    cvt(vm, unsafe {
        processthreadsapi::TerminateProcess(h as _, exit_code)
    })
    .map(drop)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    py_module!(vm, "_winapi", {
        "CloseHandle" => named_function!(ctx, _winapi, CloseHandle),
        "GetStdHandle" => named_function!(ctx, _winapi, GetStdHandle),
        "CreatePipe" => named_function!(ctx, _winapi, CreatePipe),
        "DuplicateHandle" => named_function!(ctx, _winapi, DuplicateHandle),
        "GetCurrentProcess" => named_function!(ctx, _winapi, GetCurrentProcess),
        "CreateProcess" => named_function!(ctx, _winapi, CreateProcess),
        "WaitForSingleObject" => named_function!(ctx, _winapi, WaitForSingleObject),
        "GetExitCodeProcess" => named_function!(ctx, _winapi, GetExitCodeProcess),
        "TerminateProcess" => named_function!(ctx, _winapi, TerminateProcess),

        "WAIT_OBJECT_0" => ctx.new_int(winbase::WAIT_OBJECT_0),
        "WAIT_ABANDONED" => ctx.new_int(winbase::WAIT_ABANDONED),
        "WAIT_ABANDONED_0" => ctx.new_int(winbase::WAIT_ABANDONED_0),
        "WAIT_TIMEOUT" => ctx.new_int(winerror::WAIT_TIMEOUT),
        "INFINITE" => ctx.new_int(winbase::INFINITE),
        "CREATE_NEW_CONSOLE" => ctx.new_int(winbase::CREATE_NEW_CONSOLE),
        "CREATE_NEW_PROCESS_GROUP" => ctx.new_int(winbase::CREATE_NEW_PROCESS_GROUP),
        "STD_INPUT_HANDLE" => ctx.new_int(winbase::STD_INPUT_HANDLE),
        "STD_OUTPUT_HANDLE" => ctx.new_int(winbase::STD_OUTPUT_HANDLE),
        "STD_ERROR_HANDLE" => ctx.new_int(winbase::STD_ERROR_HANDLE),
        "SW_HIDE" => ctx.new_int(winuser::SW_HIDE),
        "STARTF_USESTDHANDLES" => ctx.new_int(winbase::STARTF_USESTDHANDLES),
        "STARTF_USESHOWWINDOW" => ctx.new_int(winbase::STARTF_USESHOWWINDOW),
        "ABOVE_NORMAL_PRIORITY_CLASS" => ctx.new_int(winbase::ABOVE_NORMAL_PRIORITY_CLASS),
        "BELOW_NORMAL_PRIORITY_CLASS" => ctx.new_int(winbase::BELOW_NORMAL_PRIORITY_CLASS),
        "HIGH_PRIORITY_CLASS" => ctx.new_int(winbase::HIGH_PRIORITY_CLASS),
        "IDLE_PRIORITY_CLASS" => ctx.new_int(winbase::IDLE_PRIORITY_CLASS),
        "NORMAL_PRIORITY_CLASS" => ctx.new_int(winbase::NORMAL_PRIORITY_CLASS),
        "REALTIME_PRIORITY_CLASS" => ctx.new_int(winbase::REALTIME_PRIORITY_CLASS),
        "CREATE_NO_WINDOW" => ctx.new_int(winbase::CREATE_NO_WINDOW),
        "DETACHED_PROCESS" => ctx.new_int(winbase::DETACHED_PROCESS),
        "CREATE_DEFAULT_ERROR_MODE" => ctx.new_int(winbase::CREATE_DEFAULT_ERROR_MODE),
        "CREATE_BREAKAWAY_FROM_JOB" => ctx.new_int(winbase::CREATE_BREAKAWAY_FROM_JOB),
        "DUPLICATE_SAME_ACCESS" => ctx.new_int(winnt::DUPLICATE_SAME_ACCESS),
        "FILE_TYPE_CHAR" => ctx.new_int(winbase::FILE_TYPE_CHAR),
        "FILE_TYPE_DISK" => ctx.new_int(winbase::FILE_TYPE_DISK),
        "FILE_TYPE_PIPE" => ctx.new_int(winbase::FILE_TYPE_PIPE),
        "FILE_TYPE_REMOTE" => ctx.new_int(winbase::FILE_TYPE_REMOTE),
        "FILE_TYPE_UNKNOWN" => ctx.new_int(winbase::FILE_TYPE_UNKNOWN),
    })
}

#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "This module mirrors Win32 APIs with raw handle and pointer parameters."
)]

use std::{io, path::Path};
use windows_sys::Win32::{
    Foundation::{HANDLE, HMODULE, WAIT_FAILED, WAIT_OBJECT_0},
    System::Threading::PROCESS_INFORMATION,
};

pub struct PeekNamedPipeResult {
    pub data: Option<Vec<u8>>,
    pub available: u32,
    pub left_this_message: u32,
}

pub struct ReadFileResult {
    pub data: Vec<u8>,
    pub error: u32,
}

pub struct WriteFileResult {
    pub written: u32,
    pub error: u32,
}

pub enum BatchedWaitResult {
    All,
    Indices(Vec<usize>),
}

pub enum BatchedWaitError {
    Timeout,
    Interrupted,
    Os(u32),
}

pub enum BuildEnvironmentBlockError {
    ContainsNul,
    IllegalName,
}

pub enum MimeRegistryReadError<E> {
    Os(u32),
    Callback(E),
}

pub struct AttrList {
    handlelist: Vec<usize>,
    attrlist: Vec<u8>,
}

#[must_use]
pub fn get_acp() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetACP() }
}

pub fn close_handle(handle: HANDLE) -> i32 {
    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) }
}

impl AttrList {
    pub fn as_mut_ptr(&mut self) -> *mut core::ffi::c_void {
        self.attrlist.as_mut_ptr().cast()
    }
}

impl Drop for AttrList {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(
                self.attrlist.as_mut_ptr().cast(),
            )
        };
    }
}

pub fn create_file_w(
    file_name: *const u16,
    desired_access: u32,
    share_mode: u32,
    creation_disposition: u32,
    flags_and_attributes: u32,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::Storage::FileSystem::CreateFileW(
            file_name,
            desired_access,
            share_mode,
            core::ptr::null(),
            creation_disposition,
            flags_and_attributes,
            core::ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

/// # Safety
/// The pointer arguments must follow the Win32 `CreateProcessW` contract.
pub unsafe fn create_process_w(
    app_name: *const u16,
    command_line: *mut u16,
    inherit_handles: i32,
    creation_flags: u32,
    env: *mut u16,
    current_dir: *mut u16,
    startup_info: *mut windows_sys::Win32::System::Threading::STARTUPINFOW,
) -> io::Result<PROCESS_INFORMATION> {
    let mut procinfo = core::mem::MaybeUninit::<PROCESS_INFORMATION>::uninit();
    let ok = unsafe {
        windows_sys::Win32::System::Threading::CreateProcessW(
            app_name,
            command_line,
            core::ptr::null(),
            core::ptr::null(),
            inherit_handles,
            creation_flags,
            env.cast(),
            current_dir,
            startup_info,
            procinfo.as_mut_ptr(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { procinfo.assume_init() })
    }
}

pub fn create_junction(src: &Path, dst: &Path) -> io::Result<()> {
    junction::create(src, dst)
}

pub fn build_environment_block(
    entries: Vec<(String, String)>,
) -> Result<Vec<u16>, BuildEnvironmentBlockError> {
    use std::collections::HashMap;

    let mut last_entry: HashMap<String, Vec<u16>> = HashMap::new();
    for (key, value) in entries {
        if key.contains('\0') || value.contains('\0') {
            return Err(BuildEnvironmentBlockError::ContainsNul);
        }
        if key.is_empty() || key[1..].contains('=') {
            return Err(BuildEnvironmentBlockError::IllegalName);
        }

        let key_upper = key.to_uppercase();
        let mut entry: Vec<u16> = key.encode_utf16().collect();
        entry.push(b'=' as u16);
        entry.extend(value.encode_utf16());
        entry.push(0);
        last_entry.insert(key_upper, entry);
    }

    let mut entries: Vec<(String, Vec<u16>)> = last_entry.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::new();
    for (_, entry) in entries {
        out.extend(entry);
    }
    if out.is_empty() {
        out.push(0);
    }
    out.push(0);
    Ok(out)
}

pub fn create_handle_list_attribute_list(
    handlelist: Option<Vec<usize>>,
) -> io::Result<Option<AttrList>> {
    let Some(handlelist) = handlelist else {
        return Ok(None);
    };

    let mut size = 0;
    let first = unsafe {
        windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList(
            core::ptr::null_mut(),
            1,
            0,
            &mut size,
        )
    };
    if first != 0
        || unsafe { windows_sys::Win32::Foundation::GetLastError() }
            != windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER
    {
        return Err(io::Error::last_os_error());
    }

    let mut attrs = AttrList {
        handlelist,
        attrlist: vec![0u8; size],
    };
    let ok = unsafe {
        windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList(
            attrs.attrlist.as_mut_ptr().cast(),
            1,
            0,
            &mut size,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    let ok = unsafe {
        windows_sys::Win32::System::Threading::UpdateProcThreadAttribute(
            attrs.attrlist.as_mut_ptr().cast(),
            0,
            (2 & 0xffff) | 0x20000,
            attrs.handlelist.as_mut_ptr().cast(),
            (attrs.handlelist.len() * core::mem::size_of::<usize>()) as _,
            core::ptr::null_mut(),
            core::ptr::null(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(Some(attrs))
}

pub fn get_std_handle(
    std_handle: windows_sys::Win32::System::Console::STD_HANDLE,
) -> io::Result<Option<HANDLE>> {
    let handle = unsafe { windows_sys::Win32::System::Console::GetStdHandle(std_handle) };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else if handle.is_null() {
        Ok(None)
    } else {
        Ok(Some(handle))
    }
}

pub fn open_process(
    desired_access: u32,
    inherit_handle: bool,
    process_id: u32,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Threading::OpenProcess(
            desired_access,
            i32::from(inherit_handle),
            process_id,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn create_pipe(size: u32) -> io::Result<(HANDLE, HANDLE)> {
    let (read, write) = unsafe {
        let mut read = core::mem::MaybeUninit::<HANDLE>::uninit();
        let mut write = core::mem::MaybeUninit::<HANDLE>::uninit();
        let ok = windows_sys::Win32::System::Pipes::CreatePipe(
            read.as_mut_ptr(),
            write.as_mut_ptr(),
            core::ptr::null(),
            size,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        (read.assume_init(), write.assume_init())
    };
    Ok((read, write))
}

pub fn create_event_w(
    manual_reset: bool,
    initial_state: bool,
    name: *const u16,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateEventW(
            core::ptr::null(),
            i32::from(manual_reset),
            i32::from(initial_state),
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn set_event(handle: HANDLE) -> io::Result<()> {
    let ok = unsafe { windows_sys::Win32::System::Threading::SetEvent(handle) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn reset_event(handle: HANDLE) -> io::Result<()> {
    let ok = unsafe { windows_sys::Win32::System::Threading::ResetEvent(handle) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn wait_for_single_object(handle: HANDLE, milliseconds: u32) -> io::Result<u32> {
    let ret =
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(handle, milliseconds) };
    if ret == WAIT_FAILED {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn wait_for_multiple_objects(
    handles: &[HANDLE],
    wait_all: bool,
    milliseconds: u32,
) -> io::Result<u32> {
    let ret = unsafe {
        windows_sys::Win32::System::Threading::WaitForMultipleObjects(
            handles.len() as u32,
            handles.as_ptr(),
            i32::from(wait_all),
            milliseconds,
        )
    };
    if ret == WAIT_FAILED {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn batched_wait_for_multiple_objects(
    handles: &[HANDLE],
    wait_all: bool,
    milliseconds: u32,
    sigint_event: Option<HANDLE>,
) -> Result<BatchedWaitResult, BatchedWaitError> {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_ABANDONED_0},
        System::{
            SystemInformation::GetTickCount64,
            Threading::{
                CreateThread, GetExitCodeThread, INFINITE, ResumeThread, TerminateThread,
                WaitForMultipleObjects,
            },
        },
    };

    const MAXIMUM_WAIT_OBJECTS: usize = 64;
    let batch_size = MAXIMUM_WAIT_OBJECTS - 1;
    let mut batches: Vec<&[HANDLE]> = Vec::new();
    let mut i = 0;
    while i < handles.len() {
        let end = core::cmp::min(i + batch_size, handles.len());
        batches.push(&handles[i..end]);
        i = end;
    }

    if wait_all {
        let mut err = None;
        let deadline = if milliseconds != INFINITE {
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
                INFINITE
            };

            let result =
                unsafe { WaitForMultipleObjects(batch.len() as u32, batch.as_ptr(), 1, timeout) };
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
                    windows_sys::Win32::System::Threading::WaitForSingleObject(sigint_event, 0)
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

        return match err {
            Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT) => Err(BatchedWaitError::Timeout),
            Some(windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT) => {
                Err(BatchedWaitError::Interrupted)
            }
            Some(err) => Err(BatchedWaitError::Os(err)),
            None => Ok(BatchedWaitResult::All),
        };
    }

    let cancel_event = create_event_w(true, false, core::ptr::null())
        .map_err(|err| BatchedWaitError::Os(err.raw_os_error().unwrap_or_default() as u32))?;

    struct BatchData {
        handles: Vec<HANDLE>,
        cancel_event: HANDLE,
        handle_base: usize,
        result: AtomicU32,
        thread: core::cell::UnsafeCell<HANDLE>,
    }

    unsafe impl Send for BatchData {}
    unsafe impl Sync for BatchData {}

    extern "system" fn batch_wait_thread(param: *mut core::ffi::c_void) -> u32 {
        let data = unsafe { &*(param as *const BatchData) };
        let result = unsafe {
            windows_sys::Win32::System::Threading::WaitForMultipleObjects(
                data.handles.len() as u32,
                data.handles.as_ptr(),
                0,
                windows_sys::Win32::System::Threading::INFINITE,
            )
        };
        data.result.store(result, Ordering::SeqCst);

        if result == WAIT_FAILED {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            let _ = set_event(data.cancel_event);
            err
        } else if (WAIT_ABANDONED_0..WAIT_ABANDONED_0 + MAXIMUM_WAIT_OBJECTS as u32)
            .contains(&result)
        {
            data.result.store(WAIT_FAILED, Ordering::SeqCst);
            let _ = set_event(data.cancel_event);
            windows_sys::Win32::Foundation::ERROR_ABANDONED_WAIT_0
        } else {
            0
        }
    }

    let batch_data: Vec<Arc<BatchData>> = batches
        .iter()
        .enumerate()
        .map(|(idx, batch)| {
            let base = idx * batch_size;
            let mut handles_with_cancel = batch.to_vec();
            handles_with_cancel.push(cancel_event);
            Arc::new(BatchData {
                handles: handles_with_cancel,
                cancel_event,
                handle_base: base,
                result: AtomicU32::new(WAIT_FAILED),
                thread: core::cell::UnsafeCell::new(core::ptr::null_mut()),
            })
        })
        .collect();

    let mut thread_handles: Vec<HANDLE> = Vec::new();
    for data in &batch_data {
        let thread = unsafe {
            CreateThread(
                core::ptr::null(),
                1,
                Some(batch_wait_thread),
                Arc::as_ptr(data) as *const _ as *mut _,
                4,
                core::ptr::null_mut(),
            )
        };
        if thread.is_null() {
            for &handle in &thread_handles {
                unsafe { TerminateThread(handle, 0) };
                unsafe { CloseHandle(handle) };
            }
            unsafe { CloseHandle(cancel_event) };
            return Err(BatchedWaitError::Os(
                io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or_default() as u32,
            ));
        }
        unsafe { *data.thread.get() = thread };
        thread_handles.push(thread);
    }

    for &thread in &thread_handles {
        unsafe { ResumeThread(thread) };
    }

    let mut thread_handles_raw = thread_handles.clone();
    if let Some(sigint_event) = sigint_event {
        thread_handles_raw.push(sigint_event);
    }
    let result = unsafe {
        WaitForMultipleObjects(
            thread_handles_raw.len() as u32,
            thread_handles_raw.as_ptr(),
            0,
            milliseconds,
        )
    };

    let err = if result == WAIT_FAILED {
        Some(unsafe { windows_sys::Win32::Foundation::GetLastError() })
    } else if result == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
        Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT)
    } else if sigint_event.is_some() && result == WAIT_OBJECT_0 + thread_handles_raw.len() as u32 {
        Some(windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT)
    } else {
        None
    };

    let _ = set_event(cancel_event);
    unsafe {
        WaitForMultipleObjects(
            thread_handles.len() as u32,
            thread_handles.as_ptr(),
            1,
            INFINITE,
        )
    };

    let mut thread_err = err;
    for data in &batch_data {
        if thread_err.is_none() && data.result.load(Ordering::SeqCst) == WAIT_FAILED {
            let mut exit_code = 0;
            let thread = unsafe { *data.thread.get() };
            if unsafe { GetExitCodeThread(thread, &mut exit_code) } == 0 {
                thread_err = Some(unsafe { windows_sys::Win32::Foundation::GetLastError() });
            } else if exit_code != 0 {
                thread_err = Some(exit_code);
            }
        }
        let thread = unsafe { *data.thread.get() };
        unsafe { CloseHandle(thread) };
    }
    unsafe { CloseHandle(cancel_event) };

    match thread_err {
        Some(windows_sys::Win32::Foundation::WAIT_TIMEOUT) => Err(BatchedWaitError::Timeout),
        Some(windows_sys::Win32::Foundation::ERROR_CONTROL_C_EXIT) => {
            Err(BatchedWaitError::Interrupted)
        }
        Some(err) => Err(BatchedWaitError::Os(err)),
        None => {
            let mut triggered_indices = Vec::new();
            for data in &batch_data {
                let result = data.result.load(Ordering::SeqCst);
                let triggered = result as i32 - WAIT_OBJECT_0 as i32;
                if triggered >= 0 && (triggered as usize) < data.handles.len() - 1 {
                    triggered_indices.push(data.handle_base + triggered as usize);
                }
            }
            Ok(BatchedWaitResult::Indices(triggered_indices))
        }
    }
}

pub fn duplicate_handle(
    src_process: HANDLE,
    src: HANDLE,
    target_process: HANDLE,
    access: u32,
    inherit: i32,
    options: u32,
) -> io::Result<HANDLE> {
    let target = unsafe {
        let mut target = core::mem::MaybeUninit::<HANDLE>::uninit();
        let ok = windows_sys::Win32::Foundation::DuplicateHandle(
            src_process,
            src,
            target_process,
            target.as_mut_ptr(),
            access,
            inherit,
            options,
        );
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        target.assume_init()
    };
    Ok(target)
}

#[must_use]
pub fn get_current_process() -> HANDLE {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() }
}

pub fn get_exit_code_process(handle: HANDLE) -> io::Result<u32> {
    let mut exit_code = core::mem::MaybeUninit::<u32>::uninit();
    let ok = unsafe {
        windows_sys::Win32::System::Threading::GetExitCodeProcess(handle, exit_code.as_mut_ptr())
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { exit_code.assume_init() })
    }
}

pub fn get_file_type(
    handle: HANDLE,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::FILE_TYPE> {
    let file_type = unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(handle) };
    if file_type == 0 && unsafe { windows_sys::Win32::Foundation::GetLastError() } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(file_type)
    }
}

pub fn terminate_process(handle: HANDLE, exit_code: u32) -> i32 {
    unsafe { windows_sys::Win32::System::Threading::TerminateProcess(handle, exit_code) }
}

pub fn exit_process(exit_code: u32) -> ! {
    unsafe { windows_sys::Win32::System::Threading::ExitProcess(exit_code) }
}

#[must_use]
pub fn get_last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

#[must_use]
pub fn get_version() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetVersion() }
}

pub fn create_job_object_w(name: *const u16) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::JobObjects::CreateJobObjectW(core::ptr::null(), name)
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn assign_process_to_job_object(job: HANDLE, process: HANDLE) -> io::Result<()> {
    let ok =
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(job, process) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn terminate_job_object(job: HANDLE, exit_code: u32) -> io::Result<()> {
    let ok = unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(job, exit_code) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn set_job_object_kill_on_close(job: HANDLE) -> io::Result<()> {
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { core::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn get_module_file_name(module: HMODULE, buffer: &mut [u16]) -> u32 {
    unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            module,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    }
}

pub fn get_short_path_name_w(path: *const u16) -> io::Result<Vec<u16>> {
    get_path_name_impl(
        path,
        windows_sys::Win32::Storage::FileSystem::GetShortPathNameW,
    )
}

pub fn get_long_path_name_w(path: *const u16) -> io::Result<Vec<u16>> {
    get_path_name_impl(
        path,
        windows_sys::Win32::Storage::FileSystem::GetLongPathNameW,
    )
}

fn get_path_name_impl(
    path: *const u16,
    api_fn: unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32,
) -> io::Result<Vec<u16>> {
    let size = unsafe { api_fn(path, core::ptr::null_mut(), 0) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0u16; size as usize];
    let result = unsafe { api_fn(path, buffer.as_mut_ptr(), buffer.len() as u32) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(result as usize);
    Ok(buffer)
}

pub fn open_mutex_w(
    desired_access: u32,
    inherit_handle: bool,
    name: *const u16,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Threading::OpenMutexW(
            desired_access,
            i32::from(inherit_handle),
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn release_mutex(handle: HANDLE) -> i32 {
    unsafe { windows_sys::Win32::System::Threading::ReleaseMutex(handle) }
}

pub fn create_named_pipe_w(
    name: *const u16,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_buffer_size: u32,
    in_buffer_size: u32,
    default_timeout: u32,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Pipes::CreateNamedPipeW(
            name,
            open_mode,
            pipe_mode,
            max_instances,
            out_buffer_size,
            in_buffer_size,
            default_timeout,
            core::ptr::null(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn create_file_mapping_w(
    file_handle: HANDLE,
    protect: u32,
    max_size_high: u32,
    max_size_low: u32,
    name: *const u16,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Memory::CreateFileMappingW(
            file_handle,
            core::ptr::null(),
            protect,
            max_size_high,
            max_size_low,
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn open_file_mapping_w(
    desired_access: u32,
    inherit_handle: bool,
    name: *const u16,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Memory::OpenFileMappingW(
            desired_access,
            i32::from(inherit_handle),
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn map_view_of_file(
    file_map: HANDLE,
    desired_access: u32,
    file_offset_high: u32,
    file_offset_low: u32,
    number_bytes: usize,
) -> io::Result<isize> {
    let address = unsafe {
        windows_sys::Win32::System::Memory::MapViewOfFile(
            file_map,
            desired_access,
            file_offset_high,
            file_offset_low,
            number_bytes,
        )
    };
    let ptr = address.Value;
    if ptr.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(ptr as isize)
    }
}

pub fn unmap_view_of_file(address: isize) -> io::Result<()> {
    let view = windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
        Value: address as *mut core::ffi::c_void,
    };
    let ok = unsafe { windows_sys::Win32::System::Memory::UnmapViewOfFile(view) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn virtual_query_size(address: isize) -> io::Result<usize> {
    let mut mbi: windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION =
        unsafe { core::mem::zeroed() };
    let ret = unsafe {
        windows_sys::Win32::System::Memory::VirtualQuery(
            address as *const core::ffi::c_void,
            &mut mbi,
            core::mem::size_of::<windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION>(),
        )
    };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(mbi.RegionSize)
    }
}

pub fn copy_file2(src: *const u16, dst: *const u16, flags: u32) -> io::Result<()> {
    let mut params: windows_sys::Win32::Storage::FileSystem::COPYFILE2_EXTENDED_PARAMETERS =
        unsafe { core::mem::zeroed() };
    params.dwSize = core::mem::size_of_val(&params) as u32;
    params.dwCopyFlags = flags;

    let hr = unsafe { windows_sys::Win32::Storage::FileSystem::CopyFile2(src, dst, &params) };
    if hr < 0 {
        let err = if (hr as u32 >> 16) == 0x8007 {
            (hr as u32) & 0xFFFF
        } else {
            hr as u32
        };
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        Ok(())
    }
}

pub fn read_windows_mimetype_registry_in_batches<F, E>(
    mut on_entries: F,
) -> Result<(), MimeRegistryReadError<E>>
where
    F: FnMut(&mut Vec<(String, String)>) -> Result<(), E>,
{
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_SZ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
        RegQueryValueExW,
    };

    let mut hkcr: HKEY = core::ptr::null_mut();
    let err =
        unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, core::ptr::null(), 0, KEY_READ, &mut hkcr) };
    if err != 0 {
        return Err(MimeRegistryReadError::Os(err));
    }

    let mut index = 0;
    let mut entries = Vec::new();
    loop {
        let mut ext_buf = [0u16; 128];
        let mut cch_ext = ext_buf.len() as u32;
        let err = unsafe {
            RegEnumKeyExW(
                hkcr,
                index,
                ext_buf.as_mut_ptr(),
                &mut cch_ext,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        index += 1;

        if err == windows_sys::Win32::Foundation::ERROR_NO_MORE_ITEMS {
            break;
        }
        if err != 0 && err != windows_sys::Win32::Foundation::ERROR_MORE_DATA {
            unsafe { RegCloseKey(hkcr) };
            return Err(MimeRegistryReadError::Os(err));
        }
        if cch_ext == 0 || ext_buf[0] != b'.' as u16 {
            continue;
        }

        let ext_wide = &ext_buf[..cch_ext as usize];
        let mut subkey: HKEY = core::ptr::null_mut();
        let err = unsafe { RegOpenKeyExW(hkcr, ext_buf.as_ptr(), 0, KEY_READ, &mut subkey) };
        if err == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND
            || err == windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED
        {
            continue;
        }
        if err != 0 {
            unsafe { RegCloseKey(hkcr) };
            return Err(MimeRegistryReadError::Os(err));
        }

        let content_type_key: Vec<u16> = "Content Type\0".encode_utf16().collect();
        let mut type_buf = [0u16; 256];
        let mut cb_type = (type_buf.len() * 2) as u32;
        let mut reg_type = 0;
        let err = unsafe {
            RegQueryValueExW(
                subkey,
                content_type_key.as_ptr(),
                core::ptr::null_mut(),
                &mut reg_type,
                type_buf.as_mut_ptr().cast(),
                &mut cb_type,
            )
        };
        unsafe { RegCloseKey(subkey) };

        if err != 0 || reg_type != REG_SZ || cb_type == 0 {
            continue;
        }

        let type_len = (cb_type as usize / 2).saturating_sub(1);
        let type_str = String::from_utf16_lossy(&type_buf[..type_len]);
        let ext_str = String::from_utf16_lossy(ext_wide);
        if type_str.is_empty() {
            continue;
        }

        entries.push((type_str, ext_str));
        if entries.len() >= 64 {
            on_entries(&mut entries).map_err(MimeRegistryReadError::Callback)?;
        }
    }

    unsafe { RegCloseKey(hkcr) };
    if !entries.is_empty() {
        on_entries(&mut entries).map_err(MimeRegistryReadError::Callback)?;
    }
    Ok(())
}

pub fn lc_map_string_ex(
    locale: *const u16,
    flags: u32,
    src: *const u16,
    src_len: i32,
) -> io::Result<Vec<u16>> {
    let dest_size = unsafe {
        windows_sys::Win32::Globalization::LCMapStringEx(
            locale,
            flags,
            src,
            src_len,
            core::ptr::null_mut(),
            0,
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    if dest_size <= 0 {
        return Err(io::Error::last_os_error());
    }

    let mut dest = vec![0u16; dest_size as usize];
    let nmapped = unsafe {
        windows_sys::Win32::Globalization::LCMapStringEx(
            locale,
            flags,
            src,
            src_len,
            dest.as_mut_ptr(),
            dest_size,
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    if nmapped <= 0 {
        return Err(io::Error::last_os_error());
    }
    dest.truncate(nmapped as usize);
    Ok(dest)
}

pub fn connect_named_pipe(handle: HANDLE) -> io::Result<()> {
    let ret = unsafe {
        windows_sys::Win32::System::Pipes::ConnectNamedPipe(handle, core::ptr::null_mut())
    };
    if ret == 0 {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        if err != windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
    Ok(())
}

pub fn wait_named_pipe_w(name: *const u16, timeout: u32) -> io::Result<()> {
    let ok = unsafe { windows_sys::Win32::System::Pipes::WaitNamedPipeW(name, timeout) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn peek_named_pipe(handle: HANDLE, size: Option<u32>) -> io::Result<PeekNamedPipeResult> {
    let mut available = 0;
    let mut left_this_message = 0;
    match size {
        Some(size) => {
            let mut data = vec![0u8; size as usize];
            let mut read = 0;
            let ok = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    handle,
                    data.as_mut_ptr().cast(),
                    size,
                    &mut read,
                    &mut available,
                    &mut left_this_message,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            data.truncate(read as usize);
            Ok(PeekNamedPipeResult {
                data: Some(data),
                available,
                left_this_message,
            })
        }
        None => {
            let ok = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    handle,
                    core::ptr::null_mut(),
                    0,
                    core::ptr::null_mut(),
                    &mut available,
                    &mut left_this_message,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(PeekNamedPipeResult {
                data: None,
                available,
                left_this_message,
            })
        }
    }
}

pub fn write_file(handle: HANDLE, buffer: &[u8]) -> io::Result<WriteFileResult> {
    let len = core::cmp::min(buffer.len(), u32::MAX as usize) as u32;
    let mut written = 0;
    let ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::WriteFile(
            handle,
            buffer.as_ptr().cast(),
            len,
            &mut written,
            core::ptr::null_mut(),
        )
    };
    let err = if ret == 0 {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    } else {
        0
    };
    if ret == 0 {
        Err(io::Error::from_raw_os_error(err as i32))
    } else {
        Ok(WriteFileResult {
            written,
            error: err,
        })
    }
}

pub fn read_file(handle: HANDLE, size: u32) -> io::Result<ReadFileResult> {
    let mut data = vec![0u8; size as usize];
    let mut read = 0;
    let ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReadFile(
            handle,
            data.as_mut_ptr().cast(),
            size,
            &mut read,
            core::ptr::null_mut(),
        )
    };
    let err = if ret == 0 {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    } else {
        0
    };
    if ret == 0 && err != windows_sys::Win32::Foundation::ERROR_MORE_DATA {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    data.truncate(read as usize);
    Ok(ReadFileResult { data, error: err })
}

pub fn set_named_pipe_handle_state(
    handle: HANDLE,
    mode: Option<u32>,
    max_collection_count: Option<u32>,
    collect_data_timeout: Option<u32>,
) -> io::Result<()> {
    let mut dw_args = [
        mode.unwrap_or_default(),
        max_collection_count.unwrap_or_default(),
        collect_data_timeout.unwrap_or_default(),
    ];
    let mut p_args = [core::ptr::null_mut(); 3];
    for (index, arg) in [mode, max_collection_count, collect_data_timeout]
        .into_iter()
        .enumerate()
    {
        if arg.is_some() {
            p_args[index] = &mut dw_args[index];
        }
    }
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::SetNamedPipeHandleState(
            handle, p_args[0], p_args[1], p_args[2],
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn create_mutex_w(initial_owner: bool, name: *const u16) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(
            core::ptr::null(),
            i32::from(initial_owner),
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn open_event_w(
    desired_access: u32,
    inherit_handle: bool,
    name: *const u16,
) -> io::Result<HANDLE> {
    let handle = unsafe {
        windows_sys::Win32::System::Threading::OpenEventW(
            desired_access,
            i32::from(inherit_handle),
            name,
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn need_current_directory_for_exe_path_w(exe_name: *const u16) -> bool {
    unsafe {
        windows_sys::Win32::System::Environment::NeedCurrentDirectoryForExePathW(exe_name) != 0
    }
}

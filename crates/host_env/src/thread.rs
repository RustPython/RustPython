#[cfg(any(target_os = "linux", target_os = "macos"))]
use alloc::ffi::CString;

#[cfg(unix)]
pub fn current_thread_id() -> u64 {
    unsafe { libc::pthread_self() as u64 }
}

#[cfg(windows)]
pub fn current_thread_id() -> u64 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() as u64 }
}

#[cfg(target_os = "linux")]
pub fn set_current_thread_name(name: &str) {
    if CString::new(name).is_ok() {
        let truncated = if name.len() > 15 {
            let mut end = 15;
            while !name.is_char_boundary(end) {
                end -= 1;
            }
            CString::new(&name[..end]).expect("slice of null-free string is null-free")
        } else {
            CString::new(name).expect("name was already checked for nul bytes")
        };
        unsafe {
            libc::pthread_setname_np(libc::pthread_self(), truncated.as_ptr());
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_current_thread_name(name: &str) {
    if let Ok(c_name) = CString::new(name) {
        unsafe {
            libc::pthread_setname_np(c_name.as_ptr());
        }
    }
}

#[cfg(windows)]
pub fn set_current_thread_name(name: &str) {
    let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let _ = set_current_thread_name_wide(&wide);
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn set_current_thread_name(_name: &str) {}

/// The current thread's description as UTF-16 code units, without the terminator.
#[cfg(windows)]
pub fn current_thread_name_wide() -> std::io::Result<Vec<u16>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        System::Threading::{GetCurrentThread, GetThreadDescription},
    };

    let mut raw = core::ptr::null_mut();
    let status = unsafe { GetThreadDescription(GetCurrentThread(), &mut raw) };
    if status < 0 {
        return Err(std::io::Error::from_raw_os_error(status));
    }
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let mut len = 0usize;
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    let name = unsafe { core::slice::from_raw_parts(raw, len) }.to_vec();
    unsafe { LocalFree(raw.cast()) };
    Ok(name)
}

/// `SetThreadDescription` on the calling thread. `name` must be NUL-terminated.
#[cfg(windows)]
pub fn set_current_thread_name_wide(name: &[u16]) -> std::io::Result<()> {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadDescription};

    let status = unsafe { SetThreadDescription(GetCurrentThread(), name.as_ptr()) };
    if status < 0 {
        Err(std::io::Error::from_raw_os_error(status))
    } else {
        Ok(())
    }
}

/// The current thread name as bytes, without the terminator.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn current_thread_name(buf_len: usize) -> std::io::Result<Vec<u8>> {
    let mut buffer = vec![0u8; buf_len];
    let status = unsafe {
        libc::pthread_getname_np(
            libc::pthread_self(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status));
    }
    let len = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    buffer.truncate(len);
    Ok(buffer)
}

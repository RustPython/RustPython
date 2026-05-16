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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn set_current_thread_name(_name: &str) {}

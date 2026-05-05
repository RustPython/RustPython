#[cfg(any(target_os = "linux", target_os = "macos"))]
use alloc::ffi::CString;

#[cfg(unix)]
pub fn current_thread_id() -> u64 {
    unsafe { libc::pthread_self() as u64 }
}

#[cfg(target_os = "linux")]
pub fn set_current_thread_name(name: &str) {
    if let Ok(c_name) = CString::new(name) {
        let truncated = if c_name.as_bytes().len() > 15 {
            CString::new(&c_name.as_bytes()[..15]).unwrap_or(c_name)
        } else {
            c_name
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

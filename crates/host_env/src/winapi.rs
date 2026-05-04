use windows_sys::Win32::Foundation::HANDLE;

#[must_use]
pub fn get_acp() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetACP() }
}

#[must_use]
pub fn get_current_process() -> HANDLE {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() }
}

#[must_use]
pub fn get_last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

#[must_use]
pub fn get_version() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetVersion() }
}

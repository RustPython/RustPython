use windows_sys::Win32::Foundation::HANDLE;

pub fn get_acp() -> u32 {
    unsafe { windows_sys::Win32::Globalization::GetACP() }
}

pub fn get_current_process() -> HANDLE {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() }
}

pub fn get_last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

pub fn get_version() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetVersion() }
}

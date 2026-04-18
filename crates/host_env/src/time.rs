use core::time::Duration;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

pub const SEC_TO_MS: i64 = 1000;
pub const MS_TO_US: i64 = 1000;
pub const SEC_TO_US: i64 = SEC_TO_MS * MS_TO_US;
pub const US_TO_NS: i64 = 1000;
pub const MS_TO_NS: i64 = MS_TO_US * US_TO_NS;
pub const SEC_TO_NS: i64 = SEC_TO_MS * MS_TO_NS;
pub const NS_TO_MS: i64 = 1000 * 1000;
pub const NS_TO_US: i64 = 1000;

pub fn duration_since_system_now() -> Result<Duration, SystemTimeError> {
    SystemTime::now().duration_since(UNIX_EPOCH)
}

#[cfg(target_env = "msvc")]
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn get_tz_info() -> windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION {
    let mut info = unsafe { core::mem::zeroed() };
    unsafe { windows_sys::Win32::System::Time::GetTimeZoneInformation(&mut info) };
    info
}

#[cfg(windows)]
fn u64_from_filetime(time: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    u64::from(time.dwLowDateTime) | (u64::from(time.dwHighDateTime) << 32)
}

#[cfg(windows)]
pub fn query_performance_frequency() -> Option<i64> {
    let mut freq = core::mem::MaybeUninit::uninit();
    (unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(freq.as_mut_ptr())
    } != 0)
        .then(|| unsafe { freq.assume_init() })
}

#[cfg(windows)]
pub fn query_performance_counter() -> i64 {
    let mut counter = core::mem::MaybeUninit::uninit();
    unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceCounter(counter.as_mut_ptr());
        counter.assume_init()
    }
}

#[cfg(windows)]
pub fn get_system_time_adjustment() -> Option<u32> {
    let mut time_adjustment = core::mem::MaybeUninit::uninit();
    let mut time_increment = core::mem::MaybeUninit::uninit();
    let mut is_time_adjustment_disabled = core::mem::MaybeUninit::uninit();
    (unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemTimeAdjustment(
            time_adjustment.as_mut_ptr(),
            time_increment.as_mut_ptr(),
            is_time_adjustment_disabled.as_mut_ptr(),
        )
    } != 0)
        .then(|| unsafe { time_increment.assume_init() })
}

#[cfg(windows)]
pub fn tick_count64() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(windows)]
pub fn get_thread_time_100ns() -> Option<u64> {
    let mut creation_time = core::mem::MaybeUninit::uninit();
    let mut exit_time = core::mem::MaybeUninit::uninit();
    let mut kernel_time = core::mem::MaybeUninit::uninit();
    let mut user_time = core::mem::MaybeUninit::uninit();
    (unsafe {
        windows_sys::Win32::System::Threading::GetThreadTimes(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            creation_time.as_mut_ptr(),
            exit_time.as_mut_ptr(),
            kernel_time.as_mut_ptr(),
            user_time.as_mut_ptr(),
        )
    } != 0)
        .then(|| unsafe {
            u64_from_filetime(kernel_time.assume_init())
                + u64_from_filetime(user_time.assume_init())
        })
}

#[cfg(windows)]
pub fn get_process_time_100ns() -> Option<u64> {
    let mut creation_time = core::mem::MaybeUninit::uninit();
    let mut exit_time = core::mem::MaybeUninit::uninit();
    let mut kernel_time = core::mem::MaybeUninit::uninit();
    let mut user_time = core::mem::MaybeUninit::uninit();
    (unsafe {
        windows_sys::Win32::System::Threading::GetProcessTimes(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            creation_time.as_mut_ptr(),
            exit_time.as_mut_ptr(),
            kernel_time.as_mut_ptr(),
            user_time.as_mut_ptr(),
        )
    } != 0)
        .then(|| unsafe {
            u64_from_filetime(kernel_time.assume_init())
                + u64_from_filetime(user_time.assume_init())
        })
}

#[cfg(any(unix, windows))]
#[must_use]
pub fn asctime_from_tm(tm: &libc::tm) -> String {
    const WDAY_NAME: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MON_NAME: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {}{:>3} {:02}:{:02}:{:02} {}",
        WDAY_NAME[tm.tm_wday as usize],
        MON_NAME[tm.tm_mon as usize],
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tm.tm_year + 1900
    )
}

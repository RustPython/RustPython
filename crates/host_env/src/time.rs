use core::time::Duration;
use std::time::{SystemTime, SystemTimeError, UNIX_EPOCH};

#[cfg(target_env = "msvc")]
use alloc::string::String;

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

#[cfg(unix)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ClockId(libc::clockid_t);

#[cfg(unix)]
impl ClockId {
    pub const fn from_raw(raw: libc::clockid_t) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> libc::clockid_t {
        self.0
    }

    pub const CLOCK_MONOTONIC: Self = Self(libc::CLOCK_MONOTONIC);
    pub const CLOCK_REALTIME: Self = Self(libc::CLOCK_REALTIME);

    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
        target_os = "wasi",
    )))]
    pub const CLOCK_PROCESS_CPUTIME_ID: Self = Self(libc::CLOCK_PROCESS_CPUTIME_ID);

    #[cfg(not(any(
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "openbsd",
        target_os = "redox",
    )))]
    pub const CLOCK_THREAD_CPUTIME_ID: Self = Self(libc::CLOCK_THREAD_CPUTIME_ID);
}

#[cfg(unix)]
fn nix_clock_id(id: ClockId) -> nix::time::ClockId {
    nix::time::ClockId::from_raw(id.as_raw())
}

#[cfg(unix)]
pub fn clock_gettime(id: ClockId) -> std::io::Result<Duration> {
    nix::time::clock_gettime(nix_clock_id(id))
        .map(Duration::from)
        .map_err(std::io::Error::from)
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn clock_getres(id: ClockId) -> std::io::Result<Duration> {
    nix::time::clock_getres(nix_clock_id(id))
        .map(Duration::from)
        .map_err(std::io::Error::from)
}

#[cfg(all(unix, not(target_os = "redox"), not(target_vendor = "apple")))]
pub fn clock_settime(id: ClockId, time: Duration) -> std::io::Result<()> {
    let ts = nix::sys::time::TimeSpec::from(time);
    nix::time::clock_settime(nix_clock_id(id), ts)
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(all(unix, not(target_os = "redox"), target_os = "macos"))]
pub fn clock_settime(id: ClockId, time: Duration) -> std::io::Result<()> {
    let ts = nix::sys::time::TimeSpec::from(time);
    let ret = unsafe { libc::clock_settime(id.as_raw(), ts.as_ref()) };
    if ret != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn nanosleep(duration: Duration) -> std::io::Result<()> {
    let ts = nix::sys::time::TimeSpec::from(duration);
    let ret = unsafe { libc::nanosleep(ts.as_ref(), core::ptr::null_mut()) };
    if ret != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_env = "msvc")]
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct WindowsTimeZoneInfo {
    pub bias: i32,
    pub standard_bias: i32,
    pub daylight_bias: i32,
    pub standard_name: String,
    pub daylight_name: String,
}

#[cfg(target_env = "msvc")]
#[cfg(not(target_arch = "wasm32"))]
fn decode_tz_name(name: &[u16]) -> String {
    widestring::decode_utf16_lossy(name.iter().copied())
        .take_while(|&c| c != '\0')
        .collect()
}

#[cfg(target_env = "msvc")]
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn get_tz_info() -> WindowsTimeZoneInfo {
    let mut info = unsafe { core::mem::zeroed() };
    unsafe { windows_sys::Win32::System::Time::GetTimeZoneInformation(&mut info) };
    WindowsTimeZoneInfo {
        bias: info.Bias as i32,
        standard_bias: info.StandardBias as i32,
        daylight_bias: info.DaylightBias as i32,
        standard_name: decode_tz_name(&info.StandardName),
        daylight_name: decode_tz_name(&info.DaylightName),
    }
}

#[cfg(windows)]
fn u64_from_filetime(time: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    u64::from(time.dwLowDateTime) | (u64::from(time.dwHighDateTime) << 32)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub struct ProcessTimes100ns {
    pub user: u64,
    pub system: u64,
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
    get_process_times_100ns().map(|times| times.user + times.system)
}

#[cfg(windows)]
pub fn get_process_times_100ns() -> Option<ProcessTimes100ns> {
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
            ProcessTimes100ns {
                user: u64_from_filetime(user_time.assume_init()),
                system: u64_from_filetime(kernel_time.assume_init()),
            }
        })
}

#[cfg(any(unix, windows))]
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

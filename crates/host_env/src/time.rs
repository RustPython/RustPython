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
pub fn get_tz_info() -> windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION {
    let mut info = unsafe { core::mem::zeroed() };
    unsafe { windows_sys::Win32::System::Time::GetTimeZoneInformation(&mut info) };
    info
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

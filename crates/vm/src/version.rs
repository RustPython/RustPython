//! Several function to retrieve version information.

use chrono::{Local, prelude::DateTime};
use core::time::Duration;
use std::time::UNIX_EPOCH;

// = 3.14.0alpha
pub const MAJOR: usize = 3;
pub const MINOR: usize = 14;
pub const MICRO: usize = 0;
pub const RELEASELEVEL: &str = "alpha";
pub const RELEASELEVEL_N: usize = 0xA;
pub const SERIAL: usize = 0;

pub const VERSION_HEX: usize =
    (MAJOR << 24) | (MINOR << 16) | (MICRO << 8) | (RELEASELEVEL_N << 4) | SERIAL;

pub fn get_version() -> String {
    // Windows: include MSC v. for compatibility with ctypes.util.find_library
    // MSC v.1929 = VS 2019, version 14+ makes find_msvcrt() return None
    #[cfg(windows)]
    let msc_info = {
        let arch = if cfg!(target_pointer_width = "64") {
            "64 bit (AMD64)"
        } else {
            "32 bit (Intel)"
        };
        // Include both RustPython identifier and MSC v. for compatibility
        format!(" MSC v.1929 {arch}",)
    };

    #[cfg(not(windows))]
    let msc_info = String::new();

    format!(
        "{:.80} ({:.80}) \n[RustPython {} with {:.80}{}]", // \n is PyPy convention
        get_version_number(),
        get_build_info(),
        env!("CARGO_PKG_VERSION"),
        COMPILER,
        msc_info,
    )
}

pub fn get_version_number() -> String {
    format!("{MAJOR}.{MINOR}.{MICRO}{RELEASELEVEL}")
}

pub fn get_winver_number() -> String {
    format!("{MAJOR}.{MINOR}")
}

const COMPILER: &str = env!("RUSTC_VERSION");

pub fn get_build_info() -> String {
    // See: https://reproducible-builds.org/docs/timestamps/
    let git_revision = get_git_revision();
    let separator = if git_revision.is_empty() { "" } else { ":" };

    let git_identifier = get_git_identifier();

    format!(
        "{id}{sep}{revision}, {date:.20}, {time:.9}",
        id = if git_identifier.is_empty() {
            "default".to_owned()
        } else {
            git_identifier
        },
        sep = separator,
        revision = git_revision,
        date = get_git_date(),
        time = get_git_time(),
    )
}

pub fn get_git_revision() -> String {
    option_env!("RUSTPYTHON_GIT_HASH").unwrap_or("").to_owned()
}

pub fn get_git_tag() -> String {
    option_env!("RUSTPYTHON_GIT_TAG").unwrap_or("").to_owned()
}

pub fn get_git_branch() -> String {
    option_env!("RUSTPYTHON_GIT_BRANCH")
        .unwrap_or("")
        .to_owned()
}

pub fn get_git_identifier() -> String {
    let git_tag = get_git_tag();
    let git_branch = get_git_branch();

    if git_tag.is_empty() || git_tag == "undefined" {
        git_branch
    } else {
        git_tag
    }
}

fn get_git_timestamp_datetime() -> DateTime<Local> {
    let timestamp = option_env!("RUSTPYTHON_GIT_TIMESTAMP")
        .unwrap_or("")
        .to_owned();
    let timestamp = timestamp.parse::<u64>().unwrap_or(0);

    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);

    datetime.into()
}

pub fn get_git_date() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%b %e %Y").to_string()
}

pub fn get_git_time() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%H:%M:%S").to_string()
}

pub fn get_git_datetime() -> String {
    let date = get_git_date();
    let time = get_git_time();

    format!("{date} {time}")
}

// Must be aligned to Lib/importlib/_bootstrap_external.py
pub const PYC_MAGIC_NUMBER: u16 = 2997;

// CPython format: magic_number | ('\r' << 16) | ('\n' << 24)
// This protects against text-mode file reads
pub const PYC_MAGIC_NUMBER_TOKEN: u32 =
    (PYC_MAGIC_NUMBER as u32) | ((b'\r' as u32) << 16) | ((b'\n' as u32) << 24);

/// Magic number as little-endian bytes for .pyc files
pub const PYC_MAGIC_NUMBER_BYTES: [u8; 4] = PYC_MAGIC_NUMBER_TOKEN.to_le_bytes();

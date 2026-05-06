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

pub const GIT_REVISION: &str = env!("RUSTPYTHON_GIT_HASH");
const GIT_TAG: &str = env!("RUSTPYTHON_GIT_TAG");
const GIT_BRANCH: &str = env!("RUSTPYTHON_GIT_BRANCH");

// RustPython version
pub const MAJOR_IMPL: usize = match usize::from_str_radix(env!("CARGO_PKG_VERSION_MAJOR"), 10) {
    Ok(v) => v,
    Err(_) => panic!("Compile with Cargo to get 'CARGO_PKG_VERSION_MAJOR'"),
};
pub const MINOR_IMPL: usize = match usize::from_str_radix(env!("CARGO_PKG_VERSION_MINOR"), 10) {
    Ok(v) => v,
    Err(_) => panic!("Compile with Cargo to get 'CARGO_PKG_VERSION_MINOR'"),
};
pub const MICRO_IMPL: usize = match usize::from_str_radix(env!("CARGO_PKG_VERSION_PATCH"), 10) {
    Ok(v) => v,
    Err(_) => panic!("Compile with Cargo to get 'CARGO_PKG_VERSION_PATCH'"),
};
pub const RELEASELEVEL_IMPL: &str = env!("RUSTPYTHON_RELEASE_LEVEL");
pub const SERIAL_IMPL: usize = match usize::from_str_radix(env!("RUSTPYTHON_RELEASE_SERIAL"), 10) {
    Ok(v) => v,
    Err(_) => panic!("Compile with Cargo to get 'RUSTPYTHON_RELEASE_SERIAL'"),
};
pub const VERSION_HEX_IMPL: usize = (MAJOR_IMPL << 24)
    | (MINOR_IMPL << 16)
    | (MICRO_IMPL << 8)
    | (RELEASELEVEL_N << 4)
    | SERIAL_IMPL;

#[must_use]
pub fn get_version() -> String {
    // Windows: include MSC v. for compatibility with ctypes.util.find_library
    // MSC v.1929 = VS 2019, version 14+ makes find_msvcrt() return None
    let msc_info = cfg_select! {
        windows => {{
            let arch = if cfg!(target_pointer_width = "64") {
                "64 bit (AMD64)"
            } else {
                "32 bit (Intel)"
            };
            // Include both RustPython identifier and MSC v. for compatibility
            format!(" MSC v.1929 {arch}",)
        }},
        _ => String::new(),
    };

    format!(
        "{:.80} ({:.80}) \n[RustPython {} with {:.80}{}]", // \n is PyPy convention
        get_version_number(),
        get_build_info(),
        env!("CARGO_PKG_VERSION"),
        COMPILER,
        msc_info,
    )
}

#[must_use]
pub fn get_version_number() -> String {
    format!("{MAJOR}.{MINOR}.{MICRO}{RELEASELEVEL}")
}

#[must_use]
pub fn get_winver_number() -> String {
    format!("{MAJOR}.{MINOR}")
}

const COMPILER: &str = env!("RUSTC_VERSION");

#[must_use]
pub fn get_build_info() -> String {
    // See: https://reproducible-builds.org/docs/timestamps/
    let separator = if GIT_REVISION.is_empty() { "" } else { ":" };
    let git_identifier = get_git_identifier();

    format!(
        "{id}{sep}{revision}, {date:.20}, {time:.9}",
        id = if git_identifier.is_empty() {
            "default"
        } else {
            git_identifier
        },
        sep = separator,
        revision = GIT_REVISION,
        date = get_git_date(),
        time = get_git_time(),
    )
}

#[must_use]
pub const fn get_git_identifier() -> &'static str {
    if GIT_TAG.is_empty() || GIT_TAG.eq_ignore_ascii_case("undefined") {
        GIT_BRANCH
    } else {
        GIT_TAG
    }
}

fn get_git_timestamp_datetime() -> DateTime<Local> {
    let timestamp = option_env!("RUSTPYTHON_GIT_TIMESTAMP").unwrap_or_default();
    let timestamp = timestamp.parse::<u64>().unwrap_or_default();

    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);

    datetime.into()
}

#[must_use]
pub fn get_git_date() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%b %e %Y").to_string()
}

#[must_use]
pub fn get_git_time() -> String {
    let datetime = get_git_timestamp_datetime();

    datetime.format("%H:%M:%S").to_string()
}

#[must_use]
pub fn get_git_datetime() -> String {
    let date = get_git_date();
    let time = get_git_time();

    format!("{date} {time}")
}

// Must be aligned to Lib/importlib/_bootstrap_external.py
// Bumped to 2994 for new CommonConstant discriminants (BuiltinList, BuiltinSet)
pub const PYC_MAGIC_NUMBER: u16 = 2994;

// CPython format: magic_number | ('\r' << 16) | ('\n' << 24)
// This protects against text-mode file reads
pub const PYC_MAGIC_NUMBER_TOKEN: u32 =
    (PYC_MAGIC_NUMBER as u32) | ((b'\r' as u32) << 16) | ((b'\n' as u32) << 24);

/// Magic number as little-endian bytes for .pyc files
pub const PYC_MAGIC_NUMBER_BYTES: [u8; 4] = PYC_MAGIC_NUMBER_TOKEN.to_le_bytes();

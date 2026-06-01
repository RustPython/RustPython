//! Version info constants.
//!
//! Most of the constants are auto calculated at compile time. The main exception is the
//! target CPython version. This is defined and updated in `build.rs`.

macro_rules! parse_consts {
    ($name: ident, $var: literal) => {
        pub const $name: usize = match usize::from_str_radix(env!($var), 10) {
            Ok(v) => v,
            Err(_) => panic!(concat!("Compile with Cargo to get '", $var, "'")),
        };
    };
}

// CPython target version info
parse_consts!(MAJOR, "MAJOR_CPY");
parse_consts!(MINOR, "MINOR_CPY");
parse_consts!(MICRO, "MICRO_CPY");
pub const RELEASELEVEL: &str = env!("RELEASE_LEVEL_CPY");
parse_consts!(RELEASELEVEL_N, "RELEASE_LEVEL_N_CPY");
parse_consts!(SERIAL, "SERIAL_CPY");
pub const VERSION_HEX: usize =
    (MAJOR << 24) | (MINOR << 16) | (MICRO << 8) | (RELEASELEVEL_N << 4) | SERIAL;

#[cfg(windows)]
pub const WINVER: &str = env!("WINVER_CPY");

pub const GIT_REVISION: &str = env!("RUSTPYTHON_GIT_HASH");
pub const GIT_IDENTIFIER: &str = env!("RUSTPYTHON_GIT_IDENTIFIER");
// const GIT_TAG: &str = env!("RUSTPYTHON_GIT_TAG");
// const GIT_BRANCH: &str = env!("RUSTPYTHON_GIT_BRANCH");

// RustPython version
parse_consts!(MAJOR_IMPL, "CARGO_PKG_VERSION_MAJOR");
parse_consts!(MINOR_IMPL, "CARGO_PKG_VERSION_MINOR");
parse_consts!(MICRO_IMPL, "CARGO_PKG_VERSION_PATCH");
pub const RELEASELEVEL_IMPL: &str = env!("RUSTPYTHON_RELEASE_LEVEL");
parse_consts!(RELEASELEVEL_N_IMPL, "RUSTPYTHON_RELEASE_LEVEL_N");
parse_consts!(SERIAL_IMPL, "RUSTPYTHON_RELEASE_SERIAL");
pub const VERSION_HEX_IMPL: usize = (MAJOR_IMPL << 24)
    | (MINOR_IMPL << 16)
    | (MICRO_IMPL << 8)
    | (RELEASELEVEL_N_IMPL << 4)
    | SERIAL_IMPL;

pub const RUSTPYTHON_BUILD_INFO: &str = env!("RUSTPYTHON_BUILD_INFO");
pub const RUSTPYTHON_VERSION: &str = const {
    const LEFT: &str = env!("RUSTPYTHON_VERSION_LEFT");
    const RIGHT: &str = env!("RUSTPYTHON_VERSION_RIGHT");
    const LEN: usize = LEFT.len() + RIGHT.len() + 1;

    const fn concat() -> [u8; LEN] {
        let mut bytes_temp = [0u8; LEN];

        let (left, _) = bytes_temp.split_at_mut(LEFT.len());
        left.copy_from_slice(LEFT.as_bytes());
        let (_, right) = bytes_temp.split_at_mut(LEFT.len() + 1);
        right.copy_from_slice(RIGHT.as_bytes());
        bytes_temp[LEFT.len()] = b'\n';

        bytes_temp
    }

    const BUF: [u8; LEN] = concat();
    match str::from_utf8(&BUF) {
        Ok(v) => v,
        Err(_) => unreachable!(),
    }
};

// Must be aligned to Lib/importlib/_bootstrap_external.py
// Matches CPython 3.14 (Include/internal/pycore_magic_number.h).
pub const PYC_MAGIC_NUMBER: u16 = 3627;

// CPython format: magic_number | ('\r' << 16) | ('\n' << 24)
// This protects against text-mode file reads
pub const PYC_MAGIC_NUMBER_TOKEN: u32 =
    (PYC_MAGIC_NUMBER as u32) | ((b'\r' as u32) << 16) | ((b'\n' as u32) << 24);

/// Magic number as little-endian bytes for .pyc files
pub const PYC_MAGIC_NUMBER_BYTES: [u8; 4] = PYC_MAGIC_NUMBER_TOKEN.to_le_bytes();

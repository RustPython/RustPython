// to allow `mod foo {}` in foo.rs; clippy thinks this is a mistake/misunderstanding of
// how `mod` works, but we want this sometimes for pymodule declarations

#![allow(clippy::module_inception)]

#[macro_use]
extern crate rustpython_derive;
extern crate alloc;

#[macro_use]
pub(crate) mod macros;

mod _asyncio;
pub mod array;
mod binascii;
mod bisect;
mod bz2;
mod cmath;
mod compression; // internal module
mod contextvars;
mod csv;
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
mod lzma;
mod zlib;

mod blake2;
mod hashlib;
mod md5;
mod sha1;
mod sha256;
mod sha3;
mod sha512;

mod json;

#[cfg(not(any(target_os = "ios", target_arch = "wasm32")))]
mod locale;

mod _opcode;
mod math;
#[cfg(any(unix, windows))]
mod mmap;
mod pyexpat;
mod pystruct;
mod random;
mod statistics;
mod suggestions;
// TODO: maybe make this an extension module, if we ever get those
// mod re;
#[cfg(not(target_arch = "wasm32"))]
pub mod socket;
#[cfg(all(unix, not(target_os = "redox")))]
mod syslog;
mod unicodedata;

mod faulthandler;
#[cfg(any(unix, target_os = "wasi"))]
mod fcntl;
#[cfg(not(target_arch = "wasm32"))]
mod multiprocessing;
#[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
mod posixshmem;
#[cfg(unix)]
mod posixsubprocess;
// libc is missing constants on redox
#[cfg(all(
    feature = "sqlite",
    not(any(target_os = "android", target_arch = "wasm32"))
))]
mod _sqlite3;
#[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
mod grp;
#[cfg(windows)]
mod overlapped;
#[cfg(all(unix, not(target_os = "redox")))]
mod resource;
#[cfg(target_os = "macos")]
mod scproxy;
#[cfg(any(unix, windows, target_os = "wasi"))]
mod select;

#[cfg(all(not(target_arch = "wasm32"), feature = "ssl-openssl"))]
mod openssl;
#[cfg(all(not(target_arch = "wasm32"), feature = "ssl-rustls"))]
mod ssl;
#[cfg(all(feature = "ssl-openssl", feature = "ssl-rustls"))]
compile_error!("features \"ssl-openssl\" and \"ssl-rustls\" are mutually exclusive");

#[cfg(all(unix, not(target_os = "redox"), not(target_os = "ios")))]
mod termios;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "windows",
    target_arch = "wasm32",
    target_os = "redox",
)))]
mod uuid;

#[cfg(feature = "tkinter")]
mod tkinter;

use rustpython_common as common;
use rustpython_vm as vm;

use crate::vm::{Context, builtins};

/// Returns module definitions for multi-phase init modules.
/// These modules are added to sys.modules BEFORE their exec function runs,
/// allowing safe circular imports.
pub fn stdlib_module_defs(ctx: &Context) -> Vec<&'static builtins::PyModuleDef> {
    vec![
        _asyncio::module_def(ctx),
        _opcode::module_def(ctx),
        array::module_def(ctx),
        binascii::module_def(ctx),
        bisect::module_def(ctx),
        blake2::module_def(ctx),
        bz2::module_def(ctx),
        cmath::module_def(ctx),
        contextvars::module_def(ctx),
        csv::module_def(ctx),
        faulthandler::module_def(ctx),
        #[cfg(any(unix, target_os = "wasi"))]
        fcntl::module_def(ctx),
        #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
        grp::module_def(ctx),
        hashlib::module_def(ctx),
        json::module_def(ctx),
        #[cfg(not(any(target_os = "ios", target_arch = "wasm32")))]
        locale::module_def(ctx),
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        lzma::module_def(ctx),
        math::module_def(ctx),
        md5::module_def(ctx),
        #[cfg(any(unix, windows))]
        mmap::module_def(ctx),
        #[cfg(not(target_arch = "wasm32"))]
        multiprocessing::module_def(ctx),
        #[cfg(all(not(target_arch = "wasm32"), feature = "ssl-openssl"))]
        openssl::module_def(ctx),
        #[cfg(windows)]
        overlapped::module_def(ctx),
        #[cfg(unix)]
        posixsubprocess::module_def(ctx),
        #[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
        posixshmem::module_def(ctx),
        pyexpat::module_def(ctx),
        pystruct::module_def(ctx),
        random::module_def(ctx),
        #[cfg(all(unix, not(target_os = "redox")))]
        resource::module_def(ctx),
        #[cfg(target_os = "macos")]
        scproxy::module_def(ctx),
        #[cfg(any(unix, windows, target_os = "wasi"))]
        select::module_def(ctx),
        sha1::module_def(ctx),
        sha256::module_def(ctx),
        sha3::module_def(ctx),
        sha512::module_def(ctx),
        #[cfg(not(target_arch = "wasm32"))]
        socket::module_def(ctx),
        #[cfg(all(
            feature = "sqlite",
            not(any(target_os = "android", target_arch = "wasm32"))
        ))]
        _sqlite3::module_def(ctx),
        #[cfg(all(not(target_arch = "wasm32"), feature = "ssl-rustls"))]
        ssl::module_def(ctx),
        statistics::module_def(ctx),
        suggestions::module_def(ctx),
        #[cfg(all(unix, not(target_os = "redox")))]
        syslog::module_def(ctx),
        #[cfg(all(unix, not(any(target_os = "ios", target_os = "redox"))))]
        termios::module_def(ctx),
        #[cfg(feature = "tkinter")]
        tkinter::module_def(ctx),
        unicodedata::module_def(ctx),
        #[cfg(not(any(
            target_os = "android",
            target_os = "ios",
            target_os = "windows",
            target_arch = "wasm32",
            target_os = "redox"
        )))]
        uuid::module_def(ctx),
        zlib::module_def(ctx),
    ]
}

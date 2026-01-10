// to allow `mod foo {}` in foo.rs; clippy thinks this is a mistake/misunderstanding of
// how `mod` works, but we want this sometimes for pymodule declarations

#![allow(clippy::module_inception)]
#![cfg_attr(all(target_os = "wasi", target_env = "p2"), feature(wasip2))]

#[macro_use]
extern crate rustpython_derive;
extern crate alloc;

pub mod array;
mod binascii;
mod bisect;
mod cmath;
mod contextvars;
mod csv;
mod gc;

mod bz2;
mod compression; // internal module
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

mod math;
#[cfg(any(unix, windows))]
mod mmap;
mod opcode;
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
#[cfg(all(
    feature = "sqlite",
    not(any(target_os = "android", target_arch = "wasm32"))
))]
mod sqlite;

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

use crate::vm::{builtins, stdlib::StdlibInitFunc};
use alloc::borrow::Cow;

pub fn get_module_inits() -> impl Iterator<Item = (Cow<'static, str>, StdlibInitFunc)> {
    macro_rules! modules {
        {
            $(
                #[cfg($cfg:meta)]
                { $( $key:expr => $val:expr),* $(,)? }
            )*
        } => {{
            [
                $(
                    $(#[cfg($cfg)] (Cow::<'static, str>::from($key), Box::new($val) as StdlibInitFunc),)*
                )*
            ]
            .into_iter()
        }};
    }
    modules! {
        #[cfg(all())]
        {
            "array" => array::make_module,
            "binascii" => binascii::make_module,
            "_bisect" => bisect::make_module,
            "_bz2" => bz2::make_module,
            "cmath" => cmath::make_module,
            "_contextvars" => contextvars::make_module,
            "_csv" => csv::make_module,
            "faulthandler" => faulthandler::make_module,
            "gc" => gc::make_module,
            "_hashlib" => hashlib::make_module,
            "_sha1" => sha1::make_module,
            "_sha3" => sha3::make_module,
            "_sha256" => sha256::make_module,
            "_sha512" => sha512::make_module,
            "_md5" => md5::make_module,
            "_blake2" => blake2::make_module,
            "_json" => json::make_module,
            "math" => math::make_module,
            "pyexpat" => pyexpat::make_module,
            "_opcode" => opcode::make_module,
            "_random" => random::make_module,
            "_statistics" => statistics::make_module,
            "_struct" => pystruct::make_module,
            "unicodedata" => unicodedata::make_module,
            "zlib" => zlib::make_module,
            "_statistics" => statistics::make_module,
            "_suggestions" => suggestions::make_module,
            // crate::vm::sysmodule::sysconfigdata_name() => sysconfigdata::make_module,
        }
        #[cfg(any(unix, target_os = "wasi"))]
        {
            "fcntl" => fcntl::make_module,
        }
        #[cfg(any(unix, windows, target_os = "wasi"))]
        {
            "select" => select::make_module,
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            "_multiprocessing" => multiprocessing::make_module,
            "_socket" => socket::make_module,
        }
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        {
            "_lzma" => lzma::make_module,
        }
        #[cfg(all(feature = "sqlite", not(any(target_os = "android", target_arch = "wasm32"))))]
        {
            "_sqlite3" => sqlite::make_module,
        }
        #[cfg(all(not(target_arch = "wasm32"), feature = "ssl-rustls"))]
        {
            "_ssl" => ssl::make_module,
        }
        #[cfg(all(not(target_arch = "wasm32"), feature = "ssl-openssl"))]
        {
            "_ssl" => openssl::make_module,
        }
        #[cfg(windows)]
        {
            "_overlapped" => overlapped::make_module,
        }
        // Unix-only
        #[cfg(unix)]
        {
            "_posixsubprocess" => posixsubprocess::make_module,
        }
        #[cfg(all(unix, not(target_os = "redox"), not(target_os = "android")))]
        {
            "_posixshmem" => posixshmem::make_module,
        }
        #[cfg(any(unix, windows))]
        {
            "mmap" => mmap::make_module,
        }
        #[cfg(all(unix, not(target_os = "redox")))]
        {
            "syslog" => syslog::make_module,
            "resource" => resource::make_module,
        }
        #[cfg(all(unix, not(any(target_os = "ios", target_os = "redox"))))]
        {
            "termios" => termios::make_module,
        }
        #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
        {
            "grp" => grp::make_module,
        }
        #[cfg(target_os = "macos")]
        {
            "_scproxy" => scproxy::make_module,
        }
        #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows", target_arch = "wasm32", target_os = "redox")))]
        {
            "_uuid" => uuid::make_module,
        }
        #[cfg(not(any(target_os = "ios", target_arch = "wasm32")))]
        {
            "_locale" => locale::make_module,
        }
        #[cfg(feature = "tkinter")]
        {
            "_tkinter" => tkinter::make_module,
        }
    }
}

// to allow `mod foo {}` in foo.rs; clippy thinks this is a mistake/misunderstanding of
// how `mod` works, but we want this sometimes for pymodule declarations
#![allow(clippy::module_inception)]

#[macro_use]
extern crate rustpython_derive;

pub mod array;
mod binascii;
mod bisect;
mod cmath;
mod contextvars;
mod csv;
mod dis;
mod gc;
mod hashlib;
mod json;
mod math;
#[cfg(unix)]
mod mmap;
mod pyexpat;
mod pystruct;
mod random;
mod statistics;
// TODO: maybe make this an extension module, if we ever get those
// mod re;
#[cfg(feature = "bz2")]
mod bz2;
#[cfg(not(target_arch = "wasm32"))]
pub mod socket;
#[cfg(unix)]
mod syslog;
mod unicodedata;
mod zlib;

#[cfg(not(target_arch = "wasm32"))]
mod faulthandler;
#[cfg(any(unix, target_os = "wasi"))]
mod fcntl;
#[cfg(not(target_arch = "wasm32"))]
mod multiprocessing;
#[cfg(unix)]
mod posixsubprocess;
// libc is missing constants on redox
#[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
mod grp;
#[cfg(all(unix, not(target_os = "redox")))]
mod resource;
#[cfg(target_os = "macos")]
mod scproxy;
#[cfg(not(target_arch = "wasm32"))]
mod select;
#[cfg(all(not(target_arch = "wasm32"), feature = "ssl"))]
mod ssl;
#[cfg(all(unix, not(target_os = "redox"), not(target_os = "ios")))]
mod termios;
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "windows",
    target_arch = "wasm32"
)))]
mod uuid;

use rustpython_common as common;
use rustpython_vm as vm;

use crate::vm::{builtins, stdlib::StdlibInitFunc};
use std::borrow::Cow;

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
            "cmath" => cmath::make_module,
            "_contextvars" => contextvars::make_module,
            "_csv" => csv::make_module,
            "_dis" => dis::make_module,
            "gc" => gc::make_module,
            "hashlib" => hashlib::make_module,
            "_json" => json::make_module,
            "math" => math::make_module,
            "pyexpat" => pyexpat::make_module,
            "_random" => random::make_module,
            "_statistics" => statistics::make_module,
            "_struct" => pystruct::make_module,
            "unicodedata" => unicodedata::make_module,
            "zlib" => zlib::make_module,
            "_statistics" => statistics::make_module,
            // crate::vm::sysmodule::sysconfigdata_name() => sysconfigdata::make_module,
        }
        #[cfg(any(unix, target_os = "wasi"))]
        {
            "fcntl" => fcntl::make_module,
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            "_multiprocessing" => multiprocessing::make_module,
            "select" => select::make_module,
            "_socket" => socket::make_module,
            "faulthandler" => faulthandler::make_module,
        }
        #[cfg(feature = "ssl")]
        {
            "_ssl" => ssl::make_module,
        }
        #[cfg(feature = "bz2")]
        {
            "_bz2" => bz2::make_module,
        }
        // Unix-only
        #[cfg(unix)]
        {
            "_posixsubprocess" => posixsubprocess::make_module,
            "syslog" => syslog::make_module,
            "mmap" => mmap::make_module,
        }
        #[cfg(all(unix, not(target_os = "redox")))]
        {
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
        #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows", target_arch = "wasm32")))]
        {
            "_uuid" => uuid::make_module,
        }
    }
}

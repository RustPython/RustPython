mod array;
#[cfg(feature = "rustpython-ast")]
pub(crate) mod ast;
mod atexit;
mod binascii;
mod bisect;
mod cmath;
mod codecs;
mod collections;
mod csv;
mod dis;
mod errno;
mod functools;
mod hashlib;
mod imp;
pub(crate) mod io;
mod itertools;
mod json;
#[cfg(feature = "rustpython-parser")]
mod keyword;
mod marshal;
mod math;
mod operator;
mod platform;
mod pyexpat;
pub(crate) mod pystruct;
mod random;
// TODO: maybe make this an extension module, if we ever get those
// mod re;
#[cfg(not(target_arch = "wasm32"))]
mod socket;
mod sre;
mod string;
#[cfg(feature = "rustpython-compiler")]
mod symtable;
mod sysconfigdata;
#[cfg(unix)]
mod syslog;
#[cfg(feature = "threading")]
mod thread;
mod time;
mod unicodedata;
mod warnings;
mod weakref;
mod zlib;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[macro_use]
pub(crate) mod os;
#[cfg(windows)]
pub(crate) mod nt;
#[cfg(unix)]
pub(crate) mod posix;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[cfg(not(any(unix, windows)))]
pub(crate) mod posix_compat;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[cfg(not(any(unix, windows)))]
pub(crate) use posix_compat as posix;

#[cfg(not(target_arch = "wasm32"))]
mod faulthandler;
#[cfg(any(unix, target_os = "wasi"))]
mod fcntl;
#[cfg(windows)]
pub(crate) mod msvcrt;
#[cfg(not(target_arch = "wasm32"))]
mod multiprocessing;
#[cfg(unix)]
mod posixsubprocess;
#[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
mod pwd;
// libc is missing constants on redox
#[cfg(all(unix, not(target_os = "redox")))]
mod resource;
#[cfg(target_os = "macos")]
mod scproxy;
#[cfg(not(target_arch = "wasm32"))]
mod select;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod signal;
#[cfg(all(not(target_arch = "wasm32"), feature = "ssl"))]
mod ssl;
#[cfg(all(unix, not(target_os = "redox")))]
mod termios;
#[cfg(windows)]
mod winapi;
#[cfg(windows)]
mod winreg;

use crate::vm::VirtualMachine;
use crate::PyObjectRef;
use std::borrow::Cow;
use std::collections::HashMap;

pub type StdlibInitFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine) -> PyObjectRef)>;

pub type StdlibMap = HashMap<Cow<'static, str>, StdlibInitFunc, ahash::RandomState>;

pub fn get_module_inits() -> StdlibMap {
    macro_rules! modules {
        {
            $(
                #[cfg($cfg:meta)]
                { $( $key:expr => $val:expr),* $(,)? }
            )*
        } => {{
            let iter = std::array::IntoIter::new([
                $(
                    $(#[cfg($cfg)] (Cow::<'static, str>::from($key), Box::new($val) as StdlibInitFunc),)*
                )*
            ]);
            iter.collect()
        }};
    }
    modules! {
        #[cfg(all())]
        {
            "array" => array::make_module,
            "atexit" => atexit::make_module,
            "binascii" => binascii::make_module,
            "_bisect" => bisect::make_module,
            "cmath" => cmath::make_module,
            "_codecs" => codecs::make_module,
            "_collections" => collections::make_module,
            "_csv" => csv::make_module,
            "dis" => dis::make_module,
            "errno" => errno::make_module,
            "_functools" => functools::make_module,
            "hashlib" => hashlib::make_module,
            "itertools" => itertools::make_module,
            "_io" => io::make_module,
            "_json" => json::make_module,
            "marshal" => marshal::make_module,
            "math" => math::make_module,
            "_operator" => operator::make_module,
            "pyexpat" => pyexpat::make_module,
            "_platform" => platform::make_module,
            "_random" => random::make_module,
            "_sre" => sre::make_module,
            "_string" => string::make_module,
            "_struct" => pystruct::make_module,
            "time" => time::make_module,
            "_weakref" => weakref::make_module,
            "_imp" => imp::make_module,
            "unicodedata" => unicodedata::make_module,
            "_warnings" => warnings::make_module,
            "zlib" => zlib::make_module,
            crate::sysmodule::sysconfigdata_name() => sysconfigdata::make_module,
        }
        // parser related modules:
        #[cfg(feature = "rustpython-ast")]
        {
            "_ast" => ast::make_module,
        }
        #[cfg(feature = "rustpython-parser")]
        {
            "keyword" => keyword::make_module,
        }
        // compiler related modules:
        #[cfg(feature = "rustpython-compiler")]
        {
            "symtable" => symtable::make_module,
        }
        #[cfg(any(unix, target_os = "wasi"))]
        {
            "posix" => posix::make_module,
            "fcntl" => fcntl::make_module,
        }
        // disable some modules on WASM
        #[cfg(not(target_arch = "wasm32"))]
        {
            "_socket" => socket::make_module,
            "_multiprocessing" => multiprocessing::make_module,
            "_signal" => signal::make_module,
            "select" => select::make_module,
            "faulthandler" => faulthandler::make_module,
        }
        #[cfg(feature = "ssl")]
        {
            "_ssl" => ssl::make_module,
        }
        #[cfg(all(feature = "threading", not(target_arch = "wasm32")))]
        {
            "_thread" => thread::make_module,
        }
        // Unix-only
        #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
        {
            "pwd" => pwd::make_module,
        }
        #[cfg(all(unix, not(target_os = "redox")))]
        {
            "termios" => termios::make_module,
            "resource" => resource::make_module,
        }
        #[cfg(unix)]
        {
            "_posixsubprocess" => posixsubprocess::make_module,
            "syslog" => syslog::make_module,
        }
        // Windows-only
        #[cfg(windows)]
        {
            "nt" => nt::make_module,
            "msvcrt" => msvcrt::make_module,
            "_winapi" => winapi::make_module,
            "winreg" => winreg::make_module,
        }
        #[cfg(target_os = "macos")]
        {
            "_scproxy" => scproxy::make_module,
        }
    }
}

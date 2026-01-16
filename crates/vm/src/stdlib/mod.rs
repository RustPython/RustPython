mod _abc;
#[cfg(feature = "ast")]
pub(crate) mod ast;
pub mod atexit;
pub mod builtins;
mod codecs;
mod collections;
pub mod errno;
mod functools;
mod imp;
pub mod io;
mod itertools;
mod marshal;
mod operator;
// TODO: maybe make this an extension module, if we ever get those
// mod re;
mod sre;
mod stat;
mod string;
#[cfg(feature = "compiler")]
mod symtable;
mod sysconfig;
mod sysconfigdata;
#[cfg(feature = "threading")]
pub mod thread;
pub mod time;
mod typevar;
pub mod typing;
pub mod warnings;
mod weakref;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[macro_use]
pub mod os;
#[cfg(windows)]
pub mod nt;
#[cfg(unix)]
pub mod posix;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[cfg(not(any(unix, windows)))]
#[path = "posix_compat.rs"]
pub mod posix;

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(any(target_env = "musl", target_env = "sgx"))
))]
mod ctypes;
#[cfg(windows)]
pub(crate) mod msvcrt;

#[cfg(all(
    unix,
    not(any(target_os = "ios", target_os = "wasi", target_os = "redox"))
))]
mod pwd;

pub(crate) mod signal;
pub mod sys;
#[cfg(windows)]
mod winapi;
#[cfg(windows)]
mod winreg;

use crate::{Context, PyRef, VirtualMachine, builtins::PyModule, builtins::PyModuleDef};
use alloc::borrow::Cow;
use std::collections::HashMap;

/// Legacy single-phase init: function that creates and populates a module in one step
pub type StdlibInitFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine) -> PyRef<PyModule>)>;

/// Multi-phase init: function that returns a module definition
/// The import machinery will:
/// 1. Create module from def
/// 2. Add to sys.modules
/// 3. Call exec slot (which can safely import other modules)
pub type StdlibDefFunc = fn(&Context) -> &'static PyModuleDef;

pub type StdlibMap = HashMap<Cow<'static, str>, StdlibInitFunc, ahash::RandomState>;
pub type StdlibDefMap = HashMap<Cow<'static, str>, StdlibDefFunc, ahash::RandomState>;

pub fn get_module_inits() -> StdlibMap {
    macro_rules! modules {
        {
            $(
                #[cfg($cfg:meta)]
                { $( $key:expr => $val:expr),* $(,)? }
            )*
        } => {{
            let modules = [
                $(
                    $(#[cfg($cfg)] (Cow::<'static, str>::from($key), Box::new($val) as StdlibInitFunc),)*
                )*
            ];
            modules.into_iter().collect()
        }};
    }
    modules! {
        #[cfg(all())]
        {
            "_abc" => _abc::make_module,
            "atexit" => atexit::make_module,
            "_codecs" => codecs::make_module,
            "_collections" => collections::make_module,
            "errno" => errno::make_module,
            "_functools" => functools::make_module,
            "itertools" => itertools::make_module,
            "_io" => io::make_module,
            "marshal" => marshal::make_module,
            "_operator" => operator::make_module,
            "_signal" => signal::make_module,
            "_sre" => sre::make_module,
            "_stat" => stat::make_module,
            "_sysconfig" => sysconfig::make_module,
            "_string" => string::make_module,
            "time" => time::make_module,
            "_typing" => typing::make_module,
            "_weakref" => weakref::make_module,
            "_imp" => imp::make_module,
            "_warnings" => warnings::make_module,
            sys::sysconfigdata_name() => sysconfigdata::make_module,
        }
        // parser related modules:
        #[cfg(feature = "ast")]
        {
            "_ast" => ast::make_module,
        }
        // compiler related modules:
        #[cfg(feature = "compiler")]
        {
            "_symtable" => symtable::make_module,
        }
        #[cfg(any(unix, target_os = "wasi"))]
        {
            "posix" => posix::make_module,
            // "fcntl" => fcntl::make_module,
        }
        #[cfg(feature = "threading")]
        {
            "_thread" => thread::make_module,
        }
        // Unix-only
        #[cfg(all(
            unix,
            not(any(target_os = "ios", target_os = "wasi", target_os = "redox"))
        ))]
        {
            "pwd" => pwd::make_module,
        }
        // Windows-only
        #[cfg(windows)]
        {
            "nt" => nt::make_module,
            "msvcrt" => msvcrt::make_module,
            "_winapi" => winapi::make_module,
            "winreg" => winreg::make_module,
        }
        #[cfg(all(
            any(target_os = "linux", target_os = "macos", target_os = "windows"),
            not(any(target_env = "musl", target_env = "sgx"))
        ))]
        {
            "_ctypes" => ctypes::make_module,
        }
    }
}

/// Returns module definitions for multi-phase init modules.
/// These modules use CPython's two-phase initialization pattern:
/// 1. Create module from def and add to sys.modules
/// 2. Call exec slot (can safely import other modules without circular import issues)
pub fn get_module_defs() -> StdlibDefMap {
    // Currently empty - modules will be migrated to multi-phase init as needed
    HashMap::default()
}

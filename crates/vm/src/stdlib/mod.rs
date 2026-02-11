mod _abc;
mod _types;
#[cfg(feature = "ast")]
pub(crate) mod ast;
pub mod atexit;
pub mod builtins;
mod codecs;
mod collections;
pub mod errno;
mod functools;
mod gc;
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

#[cfg(feature = "host_env")]
#[macro_use]
pub mod os;
#[cfg(all(feature = "host_env", windows))]
pub mod nt;
#[cfg(all(feature = "host_env", unix))]
pub mod posix;
#[cfg(all(feature = "host_env", not(any(unix, windows))))]
#[path = "posix_compat.rs"]
pub mod posix;

#[cfg(all(
    feature = "host_env",
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(any(target_env = "musl", target_env = "sgx"))
))]
mod ctypes;
#[cfg(all(feature = "host_env", windows))]
pub(crate) mod msvcrt;

#[cfg(all(
    feature = "host_env",
    unix,
    not(any(target_os = "ios", target_os = "wasi", target_os = "redox"))
))]
mod pwd;

#[cfg(feature = "host_env")]
pub(crate) mod signal;
pub mod sys;
#[cfg(all(feature = "host_env", windows))]
mod winapi;
#[cfg(all(feature = "host_env", windows))]
mod winreg;

use crate::{Context, builtins::PyModuleDef};

/// Returns module definitions for multi-phase init modules.
///
/// These modules use multi-phase initialization pattern:
/// 1. Create module from def and add to sys.modules
/// 2. Call exec slot (can safely import other modules without circular import issues)
pub fn builtin_module_defs(ctx: &Context) -> Vec<&'static PyModuleDef> {
    vec![
        _abc::module_def(ctx),
        _types::module_def(ctx),
        #[cfg(feature = "ast")]
        ast::module_def(ctx),
        atexit::module_def(ctx),
        codecs::module_def(ctx),
        collections::module_def(ctx),
        #[cfg(all(
            feature = "host_env",
            any(target_os = "linux", target_os = "macos", target_os = "windows"),
            not(any(target_env = "musl", target_env = "sgx"))
        ))]
        ctypes::module_def(ctx),
        errno::module_def(ctx),
        functools::module_def(ctx),
        gc::module_def(ctx),
        imp::module_def(ctx),
        io::module_def(ctx),
        itertools::module_def(ctx),
        marshal::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        msvcrt::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        nt::module_def(ctx),
        operator::module_def(ctx),
        #[cfg(all(feature = "host_env", any(unix, target_os = "wasi")))]
        posix::module_def(ctx),
        #[cfg(all(feature = "host_env", not(any(unix, windows, target_os = "wasi"))))]
        posix::module_def(ctx),
        #[cfg(all(
            feature = "host_env",
            unix,
            not(any(target_os = "ios", target_os = "wasi", target_os = "redox"))
        ))]
        pwd::module_def(ctx),
        #[cfg(feature = "host_env")]
        signal::module_def(ctx),
        sre::module_def(ctx),
        stat::module_def(ctx),
        string::module_def(ctx),
        #[cfg(feature = "compiler")]
        symtable::module_def(ctx),
        sysconfigdata::module_def(ctx),
        sysconfig::module_def(ctx),
        #[cfg(feature = "threading")]
        thread::module_def(ctx),
        time::module_def(ctx),
        typing::module_def(ctx),
        warnings::module_def(ctx),
        weakref::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        winapi::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        winreg::module_def(ctx),
    ]
}

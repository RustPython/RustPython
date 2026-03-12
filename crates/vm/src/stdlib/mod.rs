mod _abc;
#[cfg(feature = "ast")]
pub(crate) mod _ast;
mod _codecs;
mod _collections;
mod _functools;
mod _imp;
pub mod _io;
mod _operator;
mod _sre;
mod _stat;
mod _string;
#[cfg(feature = "compiler")]
mod _symtable;
mod _sysconfig;
mod _sysconfigdata;
mod _types;
pub mod _typing;
pub mod _warnings;
mod _weakref;
pub mod atexit;
pub mod builtins;
pub mod errno;
mod gc;
mod itertools;
mod marshal;
pub mod time;
mod typevar;

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
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ),
    not(any(target_env = "musl", target_env = "sgx"))
))]
mod _ctypes;
#[cfg(all(feature = "host_env", windows))]
pub(crate) mod msvcrt;

#[cfg(all(
    feature = "host_env",
    unix,
    not(any(target_os = "ios", target_os = "wasi", target_os = "redox"))
))]
mod pwd;

#[cfg(feature = "host_env")]
pub(crate) mod _signal;
#[cfg(feature = "threading")]
pub mod _thread;
#[cfg(all(feature = "host_env", windows))]
mod _wmi;
pub mod sys;
#[cfg(all(feature = "host_env", windows))]
#[path = "_winapi.rs"]
mod winapi;
#[cfg(all(feature = "host_env", windows))]
mod winreg;
#[cfg(all(feature = "host_env", windows))]
mod winsound;

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
        _ast::module_def(ctx),
        atexit::module_def(ctx),
        _codecs::module_def(ctx),
        _collections::module_def(ctx),
        #[cfg(all(
            feature = "host_env",
            any(
                target_os = "linux",
                target_os = "macos",
                target_os = "windows",
                target_os = "android"
            ),
            not(any(target_env = "musl", target_env = "sgx"))
        ))]
        _ctypes::module_def(ctx),
        errno::module_def(ctx),
        _functools::module_def(ctx),
        gc::module_def(ctx),
        _imp::module_def(ctx),
        _io::module_def(ctx),
        itertools::module_def(ctx),
        marshal::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        msvcrt::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        nt::module_def(ctx),
        _operator::module_def(ctx),
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
        _signal::module_def(ctx),
        _sre::module_def(ctx),
        _stat::module_def(ctx),
        _string::module_def(ctx),
        #[cfg(feature = "compiler")]
        _symtable::module_def(ctx),
        _sysconfigdata::module_def(ctx),
        _sysconfig::module_def(ctx),
        #[cfg(feature = "threading")]
        _thread::module_def(ctx),
        time::module_def(ctx),
        _typing::module_def(ctx),
        _warnings::module_def(ctx),
        _weakref::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        winapi::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        winreg::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        winsound::module_def(ctx),
        #[cfg(all(feature = "host_env", windows))]
        _wmi::module_def(ctx),
    ]
}

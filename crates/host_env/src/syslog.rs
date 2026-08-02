use alloc::boxed::Box;
use core::ffi::CStr;
use parking_lot::RwLock;
use std::{os::raw::c_char, sync::OnceLock};

pub use libc::{
    LOG_ALERT, LOG_AUTH, LOG_CONS, LOG_CRIT, LOG_DAEMON, LOG_DEBUG, LOG_EMERG, LOG_ERR, LOG_INFO,
    LOG_KERN, LOG_LOCAL0, LOG_LOCAL1, LOG_LOCAL2, LOG_LOCAL3, LOG_LOCAL4, LOG_LOCAL5, LOG_LOCAL6,
    LOG_LOCAL7, LOG_LPR, LOG_MAIL, LOG_NDELAY, LOG_NEWS, LOG_NOTICE, LOG_NOWAIT, LOG_ODELAY,
    LOG_PID, LOG_SYSLOG, LOG_USER, LOG_UUCP, LOG_WARNING,
};

#[cfg(not(target_os = "redox"))]
pub use libc::{LOG_AUTHPRIV, LOG_CRON, LOG_PERROR};

#[derive(Debug)]
enum GlobalIdent {
    Explicit(Box<CStr>),
    Implicit,
}

impl GlobalIdent {
    fn as_ptr(&self) -> *const c_char {
        match self {
            Self::Explicit(cstr) => cstr.as_ptr(),
            Self::Implicit => core::ptr::null(),
        }
    }
}

fn global_ident() -> &'static RwLock<Option<GlobalIdent>> {
    static IDENT: OnceLock<RwLock<Option<GlobalIdent>>> = OnceLock::new();
    IDENT.get_or_init(|| RwLock::new(None))
}

#[must_use]
pub fn is_open() -> bool {
    global_ident().read().is_some()
}

pub fn openlog(ident: Option<Box<CStr>>, logoption: i32, facility: i32) {
    let ident = match ident {
        Some(ident) => GlobalIdent::Explicit(ident),
        None => GlobalIdent::Implicit,
    };
    let mut locked_ident = global_ident().write();
    unsafe { libc::openlog(ident.as_ptr(), logoption, facility) };
    *locked_ident = Some(ident);
}

pub fn syslog(priority: i32, msg: &CStr) {
    let _locked_ident = global_ident().read();
    let cformat = c"%s";
    unsafe { libc::syslog(priority, cformat.as_ptr(), msg.as_ptr()) };
}

pub fn closelog() {
    let mut locked_ident = global_ident().write();
    if locked_ident.is_some() {
        unsafe { libc::closelog() };
        *locked_ident = None;
    }
}

#[must_use]
pub fn setlogmask(maskpri: i32) -> i32 {
    unsafe { libc::setlogmask(maskpri) }
}

#[must_use]
pub const fn log_mask(pri: i32) -> i32 {
    1 << pri
}

#[must_use]
pub const fn log_upto(pri: i32) -> i32 {
    (1 << (pri + 1)) - 1
}

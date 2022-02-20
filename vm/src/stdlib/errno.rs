use crate::PyObjectRef;
use crate::VirtualMachine;

#[pymodule]
mod errno {}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = errno::make_module(vm);
    let errorcode = vm.ctx.new_dict();
    extend_module!(vm, module, {
        "errorcode" => errorcode.clone(),
    });
    for (name, code) in ERROR_CODES {
        let name = vm.ctx.new_str(*name);
        let code = vm.new_pyobj(*code);
        errorcode
            .set_item(code.clone(), name.clone().into(), vm)
            .unwrap();
        module.set_attr(name, code, vm).unwrap();
    }
    module
}

#[cfg(any(unix, windows, target_os = "wasi"))]
pub mod errors {
    pub use libc::*;
    #[cfg(windows)]
    pub use winapi::shared::winerror::*;
    #[cfg(windows)]
    macro_rules! reexport_wsa {
        ($($errname:ident),*$(,)?) => {
            paste::paste! {
                $(pub const $errname: i32 = [<WSA $errname>] as i32;)*
            }
        }
    }
    #[cfg(windows)]
    reexport_wsa! {
        EADDRINUSE, EADDRNOTAVAIL, EAFNOSUPPORT, EALREADY, ECONNABORTED, ECONNREFUSED, ECONNRESET,
        EDESTADDRREQ, EDQUOT, EHOSTDOWN, EHOSTUNREACH, EINPROGRESS, EISCONN, ELOOP, EMSGSIZE,
        ENETDOWN, ENETRESET, ENETUNREACH, ENOBUFS, ENOPROTOOPT, ENOTCONN, ENOTSOCK, EOPNOTSUPP,
        EPFNOSUPPORT, EPROTONOSUPPORT, EPROTOTYPE, EREMOTE, ESHUTDOWN, ESOCKTNOSUPPORT, ESTALE,
        ETIMEDOUT, ETOOMANYREFS, EUSERS, EWOULDBLOCK,
        // TODO: EBADF should be here once winerrs are translated to errnos but it messes up some things atm
    }
    #[cfg(windows)]
    pub const WSAHOS: i32 = WSAHOST_NOT_FOUND as i32;
}

#[cfg(any(unix, windows, target_os = "wasi"))]
macro_rules! e {
    ($name:ident) => {
        (stringify!($name), errors::$name as _)
    };
    (cfg($($cfg:tt)*), $name:ident) => {
        #[cfg($($cfg)*)]
        (stringify!($name), errors::$name as _)
    };
}

#[cfg(any(unix, windows, target_os = "wasi"))]
const ERROR_CODES: &[(&str, i32)] = &[
    e!(ENODEV),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOCSI
    ),
    e!(EHOSTUNREACH),
    e!(cfg(not(windows)), ENOMSG),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EUCLEAN
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EL2NSYNC
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EL2HLT
    ),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "wasi",
            windows
        ))),
        ENODATA
    ),
    e!(cfg(not(any(windows, target_os = "wasi"))), ENOTBLK),
    e!(ENOSYS),
    e!(EPIPE),
    e!(EINVAL),
    e!(cfg(not(windows)), EOVERFLOW),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EADV
    ),
    e!(EINTR),
    e!(cfg(not(target_os = "wasi")), EUSERS),
    e!(ENOTEMPTY),
    e!(ENOBUFS),
    e!(cfg(not(windows)), EPROTO),
    e!(cfg(not(target_os = "wasi")), EREMOTE),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENAVAIL
    ),
    e!(ECHILD),
    e!(ELOOP),
    e!(EXDEV),
    e!(E2BIG),
    e!(ESRCH),
    e!(EMSGSIZE),
    e!(EAFNOSUPPORT),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBADR
    ),
    e!(cfg(not(target_os = "wasi")), EHOSTDOWN),
    e!(cfg(not(target_os = "wasi")), EPFNOSUPPORT),
    e!(ENOPROTOOPT),
    e!(EBUSY),
    e!(EWOULDBLOCK),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBADFD
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EDOTDOT
    ),
    e!(EISCONN),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOANO
    ),
    e!(cfg(not(target_os = "wasi")), ESHUTDOWN),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ECHRNG
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELIBBAD
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENONET
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBADE
    ),
    e!(EBADF),
    e!(cfg(not(any(target_os = "openbsd", windows))), EMULTIHOP),
    e!(EIO),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EUNATCH
    ),
    e!(EPROTOTYPE),
    e!(ENOSPC),
    e!(ENOEXEC),
    e!(EALREADY),
    e!(ENETDOWN),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOTNAM
    ),
    e!(EACCES),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELNRNG
    ),
    e!(EILSEQ),
    e!(ENOTDIR),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOTUNIQ
    ),
    e!(EPERM),
    e!(EDOM),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EXFULL
    ),
    e!(ECONNREFUSED),
    e!(EISDIR),
    e!(EPROTONOSUPPORT),
    e!(EROFS),
    e!(EADDRNOTAVAIL),
    e!(cfg(not(windows)), EIDRM),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ECOMM
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ESRMNT
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EREMOTEIO
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EL3RST
    ),
    e!(cfg(not(windows)), EBADMSG),
    e!(ENFILE),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELIBMAX
    ),
    e!(ESPIPE),
    e!(cfg(not(any(target_os = "openbsd", windows))), ENOLINK),
    e!(ENETRESET),
    e!(ETIMEDOUT),
    e!(ENOENT),
    e!(EEXIST),
    e!(EDQUOT),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "wasi",
            windows
        ))),
        ENOSTR
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBADSLT
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBADRQC
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELIBACC
    ),
    e!(EFAULT),
    e!(EFBIG),
    e!(EDEADLK),
    e!(ENOTCONN),
    e!(EDESTADDRREQ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELIBSCN
    ),
    e!(ENOLCK),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EISNAM
    ),
    e!(ECONNABORTED),
    e!(ENETUNREACH),
    e!(ESTALE),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "wasi",
            windows
        ))),
        ENOSR
    ),
    e!(ENOMEM),
    e!(ENOTSOCK),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ESTRPIPE
    ),
    e!(EMLINK),
    e!(ERANGE),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ELIBEXEC
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EL3HLT
    ),
    e!(ECONNRESET),
    e!(EADDRINUSE),
    e!(EOPNOTSUPP),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EREMCHG
    ),
    e!(EAGAIN),
    e!(ENAMETOOLONG),
    e!(ENOTTY),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ERESTART
    ),
    e!(cfg(not(target_os = "wasi")), ESOCKTNOSUPPORT),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "wasi",
            windows
        ))),
        ETIME
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EBFONT
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EDEADLOCK
    ),
    e!(cfg(not(target_os = "wasi")), ETOOMANYREFS),
    e!(EMFILE),
    e!(cfg(not(windows)), ETXTBSY),
    e!(EINPROGRESS),
    e!(ENXIO),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOPKG
    ),
    // TODO: e!(cfg(windows), WSASY),
    e!(cfg(windows), WSAEHOSTDOWN),
    e!(cfg(windows), WSAENETDOWN),
    e!(cfg(windows), WSAENOTSOCK),
    e!(cfg(windows), WSAEHOSTUNREACH),
    e!(cfg(windows), WSAELOOP),
    e!(cfg(windows), WSAEMFILE),
    e!(cfg(windows), WSAESTALE),
    e!(cfg(windows), WSAVERNOTSUPPORTED),
    e!(cfg(windows), WSAENETUNREACH),
    e!(cfg(windows), WSAEPROCLIM),
    e!(cfg(windows), WSAEFAULT),
    e!(cfg(windows), WSANOTINITIALISED),
    e!(cfg(windows), WSAEUSERS),
    // TODO: e!(cfg(windows), WSAMAKEASYNCREPL),
    e!(cfg(windows), WSAENOPROTOOPT),
    e!(cfg(windows), WSAECONNABORTED),
    e!(cfg(windows), WSAENAMETOOLONG),
    e!(cfg(windows), WSAENOTEMPTY),
    e!(cfg(windows), WSAESHUTDOWN),
    e!(cfg(windows), WSAEAFNOSUPPORT),
    e!(cfg(windows), WSAETOOMANYREFS),
    e!(cfg(windows), WSAEACCES),
    // TODO: e!(cfg(windows), WSATR),
    e!(cfg(windows), WSABASEERR),
    // TODO: e!(cfg(windows), WSADESCRIPTIO),
    e!(cfg(windows), WSAEMSGSIZE),
    e!(cfg(windows), WSAEBADF),
    e!(cfg(windows), WSAECONNRESET),
    // TODO: e!(cfg(windows), WSAGETSELECTERRO),
    e!(cfg(windows), WSAETIMEDOUT),
    e!(cfg(windows), WSAENOBUFS),
    e!(cfg(windows), WSAEDISCON),
    e!(cfg(windows), WSAEINTR),
    e!(cfg(windows), WSAEPROTOTYPE),
    e!(cfg(windows), WSAHOS),
    e!(cfg(windows), WSAEADDRINUSE),
    e!(cfg(windows), WSAEADDRNOTAVAIL),
    e!(cfg(windows), WSAEALREADY),
    e!(cfg(windows), WSAEPROTONOSUPPORT),
    e!(cfg(windows), WSASYSNOTREADY),
    e!(cfg(windows), WSAEWOULDBLOCK),
    e!(cfg(windows), WSAEPFNOSUPPORT),
    e!(cfg(windows), WSAEOPNOTSUPP),
    e!(cfg(windows), WSAEISCONN),
    e!(cfg(windows), WSAEDQUOT),
    e!(cfg(windows), WSAENOTCONN),
    e!(cfg(windows), WSAEREMOTE),
    e!(cfg(windows), WSAEINVAL),
    e!(cfg(windows), WSAEINPROGRESS),
    // TODO: e!(cfg(windows), WSAGETSELECTEVEN),
    e!(cfg(windows), WSAESOCKTNOSUPPORT),
    // TODO: e!(cfg(windows), WSAGETASYNCERRO),
    // TODO: e!(cfg(windows), WSAMAKESELECTREPL),
    // TODO: e!(cfg(windows), WSAGETASYNCBUFLE),
    e!(cfg(windows), WSAEDESTADDRREQ),
    e!(cfg(windows), WSAECONNREFUSED),
    e!(cfg(windows), WSAENETRESET),
    // TODO: e!(cfg(windows), WSAN),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        )),
        ENOMEDIUM
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        )),
        EMEDIUMTYPE
    ),
    e!(ECANCELED),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        ENOKEY
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EKEYEXPIRED
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EKEYREVOKED
    ),
    e!(
        cfg(any(
            target_os = "android",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "redox"
        )),
        EKEYREJECTED
    ),
    e!(cfg(not(any(windows, target_os = "netbsd"))), EOWNERDEAD),
    e!(
        cfg(not(any(windows, target_os = "netbsd"))),
        ENOTRECOVERABLE
    ),
    e!(
        cfg(any(target_os = "fuchsia", target_os = "linux")),
        ERFKILL
    ),
    // Solaris-specific errnos
    e!(cfg(not(target_os = "redox")), ENOTSUP),
    e!(
        cfg(any(target_os = "illumos", target_os = "solaris")),
        ELOCKUNMAPPED
    ),
    e!(
        cfg(any(target_os = "illumos", target_os = "solaris")),
        ENOTACTIVE
    ),
    // MacOSX specific errnos
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EAUTH
    ),
    e!(cfg(target_vendor = "apple"), EBADARCH),
    e!(cfg(target_vendor = "apple"), EBADEXEC),
    e!(cfg(target_vendor = "apple"), EBADMACHO),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EBADRPC
    ),
    e!(cfg(target_vendor = "apple"), EDEVERR),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EFTYPE
    ),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        ENEEDAUTH
    ),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        ENOATTR
    ),
    e!(cfg(target_vendor = "apple"), ENOPOLICY),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EPROCLIM
    ),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EPROCUNAVAIL
    ),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EPROGMISMATCH
    ),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        EPROGUNAVAIL
    ),
    e!(cfg(target_vendor = "apple"), EPWROFF),
    e!(
        cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_vendor = "apple"
        )),
        ERPCMISMATCH
    ),
    e!(cfg(target_vendor = "apple"), ESHLIBVERS),
];

#[cfg(not(any(unix, windows, target_os = "wasi")))]
const ERROR_CODES: &[(&str, i32)] = &[];

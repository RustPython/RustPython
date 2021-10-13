use crate::VirtualMachine;
use crate::{ItemProtocol, PyObjectRef};

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let errorcode = vm.ctx.new_dict();
    let module = py_module!(vm, "errno", {
        "errorcode" => errorcode.clone(),
    });
    for (name, code) in ERROR_CODES {
        let name = vm.new_pyobj(*name);
        let code = vm.new_pyobj(*code);
        errorcode.set_item(code.clone(), name.clone(), vm).unwrap();
        vm.set_attr(&module, name, code).unwrap();
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
    e!(EHOSTUNREACH),
    e!(cfg(not(windows)), ENOMSG),
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
    e!(EOPNOTSUPP),
    e!(ENOSYS),
    e!(EPIPE),
    e!(EINVAL),
    e!(cfg(not(windows)), EOVERFLOW),
    e!(EINTR),
    e!(cfg(not(target_os = "wasi")), EUSERS),
    e!(ENOTEMPTY),
    e!(ENOBUFS),
    e!(cfg(not(windows)), EPROTO),
    e!(cfg(not(target_os = "wasi")), EREMOTE),
    e!(ECHILD),
    e!(ELOOP),
    e!(EXDEV),
    e!(E2BIG),
    e!(ESRCH),
    e!(EMSGSIZE),
    e!(EAFNOSUPPORT),
    e!(cfg(not(target_os = "wasi")), EHOSTDOWN),
    e!(cfg(not(target_os = "wasi")), EPFNOSUPPORT),
    e!(ENOPROTOOPT),
    e!(EBUSY),
    e!(EAGAIN),
    e!(EISCONN),
    e!(cfg(not(target_os = "wasi")), ESHUTDOWN),
    e!(EBADF),
    e!(cfg(not(any(target_os = "openbsd", windows))), EMULTIHOP),
    e!(EIO),
    e!(EPROTOTYPE),
    e!(ENOSPC),
    e!(ENOEXEC),
    e!(EALREADY),
    e!(ENETDOWN),
    e!(EACCES),
    e!(EILSEQ),
    e!(ENOTDIR),
    e!(EPERM),
    e!(EDOM),
    e!(ECONNREFUSED),
    e!(EISDIR),
    e!(EPROTONOSUPPORT),
    e!(EROFS),
    e!(EADDRNOTAVAIL),
    e!(cfg(not(windows)), EIDRM),
    e!(cfg(not(windows)), EBADMSG),
    e!(ENFILE),
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
    e!(EFAULT),
    e!(EFBIG),
    e!(ENOTCONN),
    e!(EDESTADDRREQ),
    e!(ENOLCK),
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
    e!(EMLINK),
    e!(ERANGE),
    e!(ECONNRESET),
    e!(EADDRINUSE),
    e!(cfg(not(any(target_os = "redox", windows))), ENOTSUP),
    e!(ENAMETOOLONG),
    e!(ENOTTY),
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
    e!(cfg(not(target_os = "wasi")), ETOOMANYREFS),
    e!(EMFILE),
    e!(cfg(not(windows)), ETXTBSY),
    e!(EINPROGRESS),
    e!(ENXIO),
    e!(ECANCELED),
    e!(EWOULDBLOCK),
    e!(cfg(not(windows)), EOWNERDEAD),
    e!(cfg(not(windows)), ENOTRECOVERABLE),
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
    e!(cfg(windows), WSAENOPROTOOPT),
    e!(cfg(windows), WSAECONNABORTED),
    e!(cfg(windows), WSAENAMETOOLONG),
    e!(cfg(windows), WSAENOTEMPTY),
    e!(cfg(windows), WSAESHUTDOWN),
    e!(cfg(windows), WSAEAFNOSUPPORT),
    e!(cfg(windows), WSAETOOMANYREFS),
    e!(cfg(windows), WSAEACCES),
    e!(cfg(windows), WSABASEERR),
    e!(cfg(windows), WSAEMSGSIZE),
    e!(cfg(windows), WSAEBADF),
    e!(cfg(windows), WSAECONNRESET),
    e!(cfg(windows), WSAETIMEDOUT),
    e!(cfg(windows), WSAENOBUFS),
    e!(cfg(windows), WSAEDISCON),
    e!(cfg(windows), WSAEINTR),
    e!(cfg(windows), WSAEPROTOTYPE),
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
    e!(cfg(windows), WSAESOCKTNOSUPPORT),
    e!(cfg(windows), WSAEDESTADDRREQ),
    e!(cfg(windows), WSAECONNREFUSED),
    e!(cfg(windows), WSAENETRESET),
];

#[cfg(not(any(unix, windows, target_os = "wasi")))]
const ERROR_CODES: &[(&str, i32)] = &[];

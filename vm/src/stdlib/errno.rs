use crate::pyobject::{ItemProtocol, PyObjectRef};
use crate::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let errorcode = vm.ctx.new_dict();
    let module = py_module!(vm, "errno", {
        "errorcode" => errorcode.clone(),
    });
    for (name, code) in ERROR_CODES {
        let name = vm.ctx.new_str((*name).to_owned());
        let code = vm.ctx.new_int(*code);
        errorcode.set_item(code.clone(), name.clone(), vm).unwrap();
        vm.set_attr(&module, name, code).unwrap();
    }
    module
}

#[rustfmt::skip]
#[allow(unused)]
pub mod errors {
    pub use libc::*;
    #[cfg(windows)]
    pub use winapi::shared::winerror::*;
    #[cfg(windows)] pub const EACCES: i32 = WSAEACCES as _;
    #[cfg(windows)] pub const EADDRINUSE: i32 = WSAEADDRINUSE as _;
    #[cfg(windows)] pub const EADDRNOTAVAIL: i32 = WSAEADDRNOTAVAIL as _;
    #[cfg(windows)] pub const EAFNOSUPPORT: i32 = WSAEAFNOSUPPORT as _;
    #[cfg(windows)] pub const EALREADY: i32 = WSAEALREADY as _;
    #[cfg(windows)] pub const EBADF: i32 = WSAEBADF as _;
    #[cfg(windows)] pub const ECANCELED: i32 = WSAECANCELLED as _;
    #[cfg(windows)] pub const ECONNABORTED: i32 = WSAECONNABORTED as _;
    #[cfg(windows)] pub const ECONNREFUSED: i32 = WSAECONNREFUSED as _;
    #[cfg(windows)] pub const ECONNRESET: i32 = WSAECONNRESET as _;
    #[cfg(windows)] pub const EDESTADDRREQ: i32 = WSAEDESTADDRREQ as _;
    #[cfg(windows)] pub const EDISCON: i32 = WSAEDISCON as _;
    #[cfg(windows)] pub const EDQUOT: i32 = WSAEDQUOT as _;
    #[cfg(windows)] pub const EFAULT: i32 = WSAEFAULT as _;
    #[cfg(windows)] pub const EHOSTDOWN: i32 = WSAEHOSTDOWN as _;
    #[cfg(windows)] pub const EHOSTUNREACH: i32 = WSAEHOSTUNREACH as _;
    #[cfg(windows)] pub const EINPROGRESS: i32 = WSAEINPROGRESS as _;
    #[cfg(windows)] pub const EINTR: i32 = WSAEINTR as _;
    #[cfg(windows)] pub const EINVAL: i32 = WSAEINVAL as _;
    #[cfg(windows)] pub const EINVALIDPROCTABLE: i32 = WSAEINVALIDPROCTABLE as _;
    #[cfg(windows)] pub const EINVALIDPROVIDER: i32 = WSAEINVALIDPROVIDER as _;
    #[cfg(windows)] pub const EISCONN: i32 = WSAEISCONN as _;
    #[cfg(windows)] pub const ELOOP: i32 = WSAELOOP as _;
    #[cfg(windows)] pub const EMFILE: i32 = WSAEMFILE as _;
    #[cfg(windows)] pub const EMSGSIZE: i32 = WSAEMSGSIZE as _;
    #[cfg(windows)] pub const ENAMETOOLONG: i32 = WSAENAMETOOLONG as _;
    #[cfg(windows)] pub const ENETDOWN: i32 = WSAENETDOWN as _;
    #[cfg(windows)] pub const ENETRESET: i32 = WSAENETRESET as _;
    #[cfg(windows)] pub const ENETUNREACH: i32 = WSAENETUNREACH as _;
    #[cfg(windows)] pub const ENOBUFS: i32 = WSAENOBUFS as _;
    #[cfg(windows)] pub const ENOMORE: i32 = WSAENOMORE as _;
    #[cfg(windows)] pub const ENOPROTOOPT: i32 = WSAENOPROTOOPT as _;
    #[cfg(windows)] pub const ENOTCONN: i32 = WSAENOTCONN as _;
    #[cfg(windows)] pub const ENOTEMPTY: i32 = WSAENOTEMPTY as _;
    #[cfg(windows)] pub const ENOTSOCK: i32 = WSAENOTSOCK as _;
    #[cfg(windows)] pub const EOPNOTSUPP: i32 = WSAEOPNOTSUPP as _;
    #[cfg(windows)] pub const EPFNOSUPPORT: i32 = WSAEPFNOSUPPORT as _;
    #[cfg(windows)] pub const EPROCLIM: i32 = WSAEPROCLIM as _;
    #[cfg(windows)] pub const EPROTONOSUPPORT: i32 = WSAEPROTONOSUPPORT as _;
    #[cfg(windows)] pub const EPROTOTYPE: i32 = WSAEPROTOTYPE as _;
    #[cfg(windows)] pub const EPROVIDERFAILEDINIT: i32 = WSAEPROVIDERFAILEDINIT as _;
    #[cfg(windows)] pub const EREFUSED: i32 = WSAEREFUSED as _;
    #[cfg(windows)] pub const EREMOTE: i32 = WSAEREMOTE as _;
    #[cfg(windows)] pub const ESHUTDOWN: i32 = WSAESHUTDOWN as _;
    #[cfg(windows)] pub const ESOCKTNOSUPPORT: i32 = WSAESOCKTNOSUPPORT as _;
    #[cfg(windows)] pub const ESTALE: i32 = WSAESTALE as _;
    #[cfg(windows)] pub const ETIMEDOUT: i32 = WSAETIMEDOUT as _;
    #[cfg(windows)] pub const ETOOMANYREFS: i32 = WSAETOOMANYREFS as _;
    #[cfg(windows)] pub const EUSERS: i32 = WSAEUSERS as _;
    #[cfg(windows)] pub const EWOULDBLOCK: i32 = WSAEWOULDBLOCK as _;
}

macro_rules! e {
    ($name:ident) => {
        (stringify!($name), errors::$name as _)
    };
    (cfg($($cfg:tt)*), $name:ident) => {
        #[cfg($($cfg)*)]
        (stringify!($name), errors::$name as _)
    };
}

#[cfg(any(unix, windows))]
const ERROR_CODES: &[(&str, i32)] = &[
    e!(ENODEV),
    e!(EHOSTUNREACH),
    e!(cfg(not(windows)), ENOMSG),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            windows
        ))),
        ENODATA
    ),
    e!(cfg(not(windows)), ENOTBLK),
    e!(ENOSYS),
    e!(EPIPE),
    e!(EINVAL),
    e!(cfg(not(windows)), EOVERFLOW),
    e!(EINTR),
    e!(EUSERS),
    e!(ENOTEMPTY),
    e!(ENOBUFS),
    e!(cfg(not(windows)), EPROTO),
    e!(EREMOTE),
    e!(ECHILD),
    e!(ELOOP),
    e!(EXDEV),
    e!(E2BIG),
    e!(ESRCH),
    e!(EMSGSIZE),
    e!(EAFNOSUPPORT),
    e!(EHOSTDOWN),
    e!(EPFNOSUPPORT),
    e!(ENOPROTOOPT),
    e!(EBUSY),
    e!(EAGAIN),
    e!(EISCONN),
    e!(ESHUTDOWN),
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
    e!(ESOCKTNOSUPPORT),
    e!(
        cfg(not(any(
            target_os = "openbsd",
            target_os = "freebsd",
            windows
        ))),
        ETIME
    ),
    e!(ETOOMANYREFS),
    e!(EMFILE),
    e!(cfg(not(windows)), ETXTBSY),
    e!(EINPROGRESS),
    e!(ENXIO),
    e!(ECANCELED),
    e!(EWOULDBLOCK),
    e!(cfg(not(windows)), EOWNERDEAD),
    e!(cfg(not(windows)), ENOTRECOVERABLE),
    e!(cfg(windows), WSAEAFNOSUPPORT),
    e!(cfg(windows), WSAEHOSTDOWN),
    e!(cfg(windows), WSAEPFNOSUPPORT),
    e!(cfg(windows), WSAENOPROTOOPT),
    e!(cfg(windows), WSAEISCONN),
    e!(cfg(windows), WSAESHUTDOWN),
    e!(cfg(windows), WSAEINVAL),
    e!(cfg(windows), WSAEBADF),
    e!(cfg(windows), WSAENAMETOOLONG),
    e!(cfg(windows), WSAEPROCLIM),
    e!(cfg(windows), WSAEMFILE),
    e!(cfg(windows), WSAEINPROGRESS),
    e!(cfg(windows), WSAETOOMANYREFS),
    e!(cfg(windows), WSAESOCKTNOSUPPORT),
    e!(cfg(windows), WSAECONNRESET),
    e!(cfg(windows), WSAENOTSOCK),
    e!(cfg(windows), WSAECONNABORTED),
    e!(cfg(windows), WSAENOTCONN),
    e!(cfg(windows), WSAEDQUOT),
    e!(cfg(windows), WSAENETRESET),
    e!(cfg(windows), WSAEADDRNOTAVAIL),
    e!(cfg(windows), WSAEPROTONOSUPPORT),
    e!(cfg(windows), WSAECONNREFUSED),
    e!(cfg(windows), WSAEALREADY),
    e!(cfg(windows), WSAEPROTOTYPE),
    e!(cfg(windows), WSAEWOULDBLOCK),
    e!(cfg(windows), WSAEMSGSIZE),
    e!(cfg(windows), WSAELOOP),
    e!(cfg(windows), WSAEREMOTE),
    e!(cfg(windows), WSAENOBUFS),
    e!(cfg(windows), WSAEUSERS),
    e!(cfg(windows), WSAEHOSTUNREACH),
    e!(cfg(windows), WSAENETDOWN),
    e!(cfg(windows), WSAETIMEDOUT),
    e!(cfg(windows), WSAEDESTADDRREQ),
    e!(cfg(windows), WSAENETUNREACH),
    e!(cfg(windows), WSAESTALE),
    e!(cfg(windows), WSAEADDRINUSE),
    e!(cfg(windows), WSAEOPNOTSUPP),
    e!(cfg(windows), WSAEFAULT),
    e!(cfg(windows), WSAENOTEMPTY),
    e!(cfg(windows), WSAEACCES),
    e!(cfg(windows), WSAEDISCON),
    e!(cfg(windows), WSAEINTR),
];

#[cfg(not(any(unix, windows)))]
const ERROR_CODES: &[(&str, i32)] = &[];

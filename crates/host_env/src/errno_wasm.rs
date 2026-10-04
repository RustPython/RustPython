//! Errno numbers for `wasm32-unknown-unknown`.
//!
//! There is no libc errno table. The values are the Darwin/BSD numbers the
//! guest `O_*` flags and `OSError` subclasses match against.

#[must_use]
pub fn strerror_string(errno: i32) -> Option<String> {
    errors::strerror(errno).map(str::to_owned)
}

pub mod errors {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const ESRCH: i32 = 3;
    pub const EINTR: i32 = 4;
    pub const EIO: i32 = 5;
    pub const EBADF: i32 = 9;
    pub const ECHILD: i32 = 10;
    pub const EAGAIN: i32 = 35;
    pub const EWOULDBLOCK: i32 = 35;
    pub const EINPROGRESS: i32 = 36;
    pub const EALREADY: i32 = 37;
    pub const EPIPE: i32 = 32;
    pub const ENOTDIR: i32 = 20;
    pub const EISDIR: i32 = 21;
    pub const EINVAL: i32 = 22;
    pub const EMFILE: i32 = 24;
    pub const ESPIPE: i32 = 29;
    pub const EROFS: i32 = 30;
    pub const EACCES: i32 = 13;
    pub const EEXIST: i32 = 17;
    pub const ENOTSUP: i32 = 45;
    pub const EAFNOSUPPORT: i32 = 47;
    pub const ECONNABORTED: i32 = 53;
    pub const ECONNRESET: i32 = 54;
    pub const ESHUTDOWN: i32 = 58;
    pub const ETIMEDOUT: i32 = 60;
    pub const ECONNREFUSED: i32 = 61;

    pub fn strerror(errno: i32) -> Option<&'static str> {
        Some(match errno {
            EPERM => "Operation not permitted",
            ENOENT => "No such file or directory",
            ESRCH => "No such process",
            EIO => "Input/output error",
            EINTR => "Interrupted system call",
            ECHILD => "No child processes",
            EACCES => "Permission denied",
            EEXIST => "File exists",
            ENOTDIR => "Not a directory",
            EISDIR => "Is a directory",
            EPIPE => "Broken pipe",
            EAGAIN => "Resource temporarily unavailable",
            EINPROGRESS => "Operation now in progress",
            EALREADY => "Operation already in progress",
            ENOTSUP => "Operation not supported",
            EAFNOSUPPORT => "Address family not supported by protocol",
            ECONNABORTED => "Software caused connection abort",
            ECONNRESET => "Connection reset by peer",
            ETIMEDOUT => "Operation timed out",
            ECONNREFUSED => "Connection refused",
            EBADF => "Bad file descriptor",
            EINVAL => "Invalid argument",
            EMFILE => "Too many open files",
            ESPIPE => "Illegal seek",
            EROFS => "Read-only file system",
            ESHUTDOWN => "Can't send after socket shutdown",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::errors;

    #[test]
    fn eagain_is_darwin() {
        assert_eq!(errors::EAGAIN, 35);
        assert_eq!(errors::EWOULDBLOCK, 35);
        assert_eq!(
            errors::strerror(errors::EAGAIN),
            Some("Resource temporarily unavailable")
        );
    }
}

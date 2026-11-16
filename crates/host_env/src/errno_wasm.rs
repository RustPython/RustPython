//! Errno numbers for `wasm32-unknown-unknown`.
//!
//! Use WASI preview1 errno values, matching `wasm32-wasip1`. Native host
//! builds continue to use their platform libc errno table.

#[must_use]
pub fn strerror_string(errno: i32) -> Option<String> {
    errors::strerror(errno).map(str::to_owned)
}

pub mod errors {
    pub const EPERM: i32 = 63;
    pub const ENOENT: i32 = 44;
    pub const ESRCH: i32 = 71;
    pub const EINTR: i32 = 27;
    pub const EIO: i32 = 29;
    pub const EBADF: i32 = 8;
    pub const ECHILD: i32 = 12;
    pub const EACCES: i32 = 2;
    pub const EEXIST: i32 = 20;
    pub const ENOTDIR: i32 = 54;
    pub const EISDIR: i32 = 31;
    pub const EINVAL: i32 = 28;
    pub const EMFILE: i32 = 33;
    pub const EFBIG: i32 = 22;
    pub const ENOSPC: i32 = 51;
    pub const ESPIPE: i32 = 70;
    pub const EROFS: i32 = 69;
    pub const EPIPE: i32 = 64;
    pub const EAGAIN: i32 = 6;
    pub const EWOULDBLOCK: i32 = 6;
    pub const EINPROGRESS: i32 = 26;
    pub const EALREADY: i32 = 7;
    pub const ENOTSUP: i32 = 58;
    pub const EAFNOSUPPORT: i32 = 5;
    pub const ECONNABORTED: i32 = 13;
    pub const ECONNRESET: i32 = 15;
    pub const ETIMEDOUT: i32 = 73;
    pub const ECONNREFUSED: i32 = 14;
    pub const ENAMETOOLONG: i32 = 37;
    pub const EOVERFLOW: i32 = 61;
    pub const ENOTCAPABLE: i32 = 76;

    pub fn strerror(errno: i32) -> Option<&'static str> {
        Some(match errno {
            EPERM => "Operation not permitted",
            ENOENT => "No such file or directory",
            ESRCH => "No such process",
            EINTR => "Interrupted system call",
            EIO => "Input/output error",
            EBADF => "Bad file descriptor",
            ECHILD => "No child processes",
            EACCES => "Permission denied",
            EEXIST => "File exists",
            ENOTDIR => "Not a directory",
            EISDIR => "Is a directory",
            EINVAL => "Invalid argument",
            EMFILE => "Too many open files",
            EFBIG => "File too large",
            ENOSPC => "No space left on device",
            ESPIPE => "Illegal seek",
            EROFS => "Read-only file system",
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
            ENAMETOOLONG => "File name too long",
            EOVERFLOW => "Value too large to be stored in data type",
            ENOTCAPABLE => "Capabilities insufficient",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::errors;

    #[test]
    fn eagain_is_wasi() {
        assert_eq!(errors::EAGAIN, 6);
        assert_eq!(errors::EWOULDBLOCK, 6);
        assert_eq!(
            errors::strerror(errors::EAGAIN),
            Some("Resource temporarily unavailable")
        );
    }
}

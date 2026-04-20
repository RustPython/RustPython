use std::io;

#[cfg(unix)]
use std::os::fd::BorrowedFd;

pub fn fcntl_int(fd: i32, cmd: i32, arg: i32) -> io::Result<i32> {
    let ret = unsafe { libc::fcntl(fd, cmd, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn validate_fd(fd: i32) -> io::Result<()> {
    fcntl_int(fd, libc::F_GETFD, 0).map(|_| ())
}

#[cfg(unix)]
pub fn get_inheritable(fd: BorrowedFd<'_>) -> io::Result<bool> {
    use nix::fcntl as nix_fcntl;

    let flags = nix_fcntl::FdFlag::from_bits_truncate(
        nix_fcntl::fcntl(fd, nix_fcntl::FcntlArg::F_GETFD).map_err(io::Error::from)?,
    );
    Ok(!flags.contains(nix_fcntl::FdFlag::FD_CLOEXEC))
}

#[cfg(unix)]
pub fn get_blocking(fd: BorrowedFd<'_>) -> io::Result<bool> {
    use nix::fcntl as nix_fcntl;

    let flags = nix_fcntl::OFlag::from_bits_truncate(
        nix_fcntl::fcntl(fd, nix_fcntl::FcntlArg::F_GETFL).map_err(io::Error::from)?,
    );
    Ok(!flags.contains(nix_fcntl::OFlag::O_NONBLOCK))
}

#[cfg(unix)]
pub fn set_blocking(fd: BorrowedFd<'_>, blocking: bool) -> io::Result<()> {
    use nix::fcntl as nix_fcntl;

    let flags = nix_fcntl::OFlag::from_bits_truncate(
        nix_fcntl::fcntl(fd, nix_fcntl::FcntlArg::F_GETFL).map_err(io::Error::from)?,
    );
    let mut new_flags = flags;
    new_flags.set(nix_fcntl::OFlag::O_NONBLOCK, !blocking);
    if flags != new_flags {
        nix_fcntl::fcntl(fd, nix_fcntl::FcntlArg::F_SETFL(new_flags)).map_err(io::Error::from)?;
    }
    Ok(())
}

pub fn fcntl_with_bytes(fd: i32, cmd: i32, arg: &mut [u8]) -> io::Result<i32> {
    let ret = unsafe { libc::fcntl(fd, cmd, arg.as_mut_ptr()) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// # Safety
///
/// `arg` must be a valid pointer for the `request` passed to `ioctl` and must
/// satisfy the platform ABI requirements for that request.
pub unsafe fn ioctl_ptr(
    fd: i32,
    request: libc::c_ulong,
    arg: *mut libc::c_void,
) -> io::Result<i32> {
    let ret = unsafe { libc::ioctl(fd, request as _, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub fn ioctl_int(fd: i32, request: libc::c_ulong, arg: i32) -> io::Result<i32> {
    let ret = unsafe { libc::ioctl(fd, request as _, arg) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[cfg(not(any(target_os = "wasi", target_os = "redox")))]
pub fn flock(fd: i32, operation: i32) -> io::Result<i32> {
    let ret = unsafe { libc::flock(fd, operation) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[cfg(not(any(target_os = "wasi", target_os = "redox")))]
pub fn lockf(fd: i32, cmd: i32, lock: &libc::flock) -> io::Result<i32> {
    let ret = unsafe {
        libc::fcntl(
            fd,
            if (cmd & libc::LOCK_NB) != 0 {
                libc::F_SETLK
            } else {
                libc::F_SETLKW
            },
            lock,
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

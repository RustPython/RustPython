use std::os::fd::BorrowedFd;

pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> nix::Result<()> {
    use nix::fcntl;

    let flags = fcntl::FdFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD)?);
    let mut new_flags = flags;
    new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
    if flags != new_flags {
        fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags))?;
    }
    Ok(())
}

use core::ffi::CStr;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::Path;

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

#[cfg(target_os = "macos")]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    type MachPortT = libc::c_uint;
    type KernReturnT = libc::c_int;
    type MachMsgTypeNumberT = libc::c_uint;
    type ThreadActArrayT = *mut MachPortT;
    const KERN_SUCCESS: KernReturnT = 0;
    unsafe extern "C" {
        fn mach_task_self() -> MachPortT;
        fn task_for_pid(
            task: MachPortT,
            pid: libc::c_int,
            target_task: *mut MachPortT,
        ) -> KernReturnT;
        fn task_threads(
            target_task: MachPortT,
            act_list: *mut ThreadActArrayT,
            act_list_cnt: *mut MachMsgTypeNumberT,
        ) -> KernReturnT;
        fn vm_deallocate(
            target_task: MachPortT,
            address: libc::uintptr_t,
            size: libc::uintptr_t,
        ) -> KernReturnT;
    }

    let self_task = unsafe { mach_task_self() };
    let mut proc_task: MachPortT = 0;
    if unsafe { task_for_pid(self_task, libc::getpid(), &mut proc_task) } == KERN_SUCCESS {
        let mut threads: ThreadActArrayT = core::ptr::null_mut();
        let mut n_threads: MachMsgTypeNumberT = 0;
        if unsafe { task_threads(proc_task, &mut threads, &mut n_threads) } == KERN_SUCCESS {
            if !threads.is_null() {
                let _ = unsafe {
                    vm_deallocate(
                        self_task,
                        threads as libc::uintptr_t,
                        (n_threads as usize * core::mem::size_of::<MachPortT>()) as libc::uintptr_t,
                    )
                };
            }
            return n_threads as isize;
        }
    }
    0
}

#[cfg(target_os = "linux")]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    use std::io::Read as _;

    let mut file = match crate::fs::open("/proc/self/stat") {
        Ok(f) => f,
        Err(_) => return 0,
    };
    let mut buf = [0u8; 160];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return 0,
    };
    let line = match core::str::from_utf8(&buf[..n]) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    if let Some(field) = line.split_whitespace().nth(19) {
        return field.parse::<isize>().unwrap_or(0);
    }
    0
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[must_use]
pub fn get_number_of_os_threads() -> isize {
    0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Permissions {
    pub is_readable: bool,
    pub is_writable: bool,
    pub is_executable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessError {
    InvalidMode,
    Os(nix::Error),
}

impl From<nix::Error> for AccessError {
    fn from(value: nix::Error) -> Self {
        Self::Os(value)
    }
}

const F_OK: u8 = 0;
const R_OK: u8 = 4;
const W_OK: u8 = 2;
const X_OK: u8 = 1;

fn get_permissions(mode: u32) -> Permissions {
    Permissions {
        is_readable: mode & 4 != 0,
        is_writable: mode & 2 != 0,
        is_executable: mode & 1 != 0,
    }
}

pub fn get_right_permission(
    mode: u32,
    file_owner: nix::unistd::Uid,
    file_group: nix::unistd::Gid,
) -> nix::Result<Permissions> {
    let owner_mode = (mode & 0o700) >> 6;
    let owner_permissions = get_permissions(owner_mode);

    let group_mode = (mode & 0o070) >> 3;
    let group_permissions = get_permissions(group_mode);

    let others_mode = mode & 0o007;
    let others_permissions = get_permissions(others_mode);

    let user_id = nix::unistd::getuid();
    let groups_ids = getgroups()?;

    if file_owner == user_id {
        Ok(owner_permissions)
    } else if groups_ids.contains(&file_group) {
        Ok(group_permissions)
    } else {
        Ok(others_permissions)
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn getgroups() -> nix::Result<Vec<nix::unistd::Gid>> {
    use core::ptr;
    use libc::{c_int, gid_t};
    use nix::errno::Errno;

    let ret = unsafe { libc::getgroups(0, ptr::null_mut()) };
    let mut groups = Vec::<nix::unistd::Gid>::with_capacity(Errno::result(ret)? as usize);
    let ret = unsafe {
        libc::getgroups(
            groups.capacity() as c_int,
            groups.as_mut_ptr() as *mut gid_t,
        )
    };

    Errno::result(ret).map(|s| {
        unsafe { groups.set_len(s as usize) };
        groups
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "redox")))]
pub use nix::unistd::getgroups;

#[cfg(target_os = "redox")]
pub fn getgroups() -> nix::Result<Vec<nix::unistd::Gid>> {
    Err(nix::Error::EOPNOTSUPP)
}

pub fn check_access(path: &Path, mode: u8) -> Result<bool, AccessError> {
    use std::os::unix::fs::MetadataExt;

    if mode & !(R_OK | W_OK | X_OK) != 0 {
        return Err(AccessError::InvalidMode);
    }

    let metadata = match crate::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(false),
    };

    if mode == F_OK {
        return Ok(true);
    }

    let perm = get_right_permission(
        metadata.mode(),
        nix::unistd::Uid::from_raw(metadata.uid()),
        nix::unistd::Gid::from_raw(metadata.gid()),
    )?;

    let r_ok = (mode & R_OK == 0) || perm.is_readable;
    let w_ok = (mode & W_OK == 0) || perm.is_writable;
    let x_ok = (mode & X_OK == 0) || perm.is_executable;

    Ok(r_ok && w_ok && x_ok)
}

pub fn close_fds(above: i32, keep: &[BorrowedFd<'_>]) {
    #[cfg(not(target_os = "redox"))]
    if close_dir_fds(above, keep).is_ok() {
        return;
    }
    #[cfg(target_os = "redox")]
    if close_filetable_fds(above, keep).is_ok() {
        return;
    }
    close_fds_brute_force(above, keep)
}

#[allow(clippy::too_many_arguments)]
pub fn setup_child_fds(
    fds_to_keep: &[BorrowedFd<'_>],
    errpipe_write: BorrowedFd<'_>,
    p2cread: i32,
    p2cwrite: i32,
    c2pread: i32,
    c2pwrite: i32,
    errread: i32,
    errwrite: i32,
    errpipe_read: i32,
) -> nix::Result<()> {
    for &fd in fds_to_keep {
        if fd.as_raw_fd() != errpipe_write.as_raw_fd() {
            set_inheritable(fd, true)?;
        }
    }

    for fd in [p2cwrite, c2pread, errread] {
        if fd >= 0 {
            nix::unistd::close(fd)?;
        }
    }
    nix::unistd::close(errpipe_read)?;

    let c2pwrite = if c2pwrite == 0 {
        let fd = unsafe { BorrowedFd::borrow_raw(c2pwrite) };
        let dup = nix::unistd::dup(fd)?;
        set_inheritable(dup.as_fd(), true)?;
        dup.as_raw_fd()
    } else {
        c2pwrite
    };

    let mut errwrite = errwrite;
    while errwrite == 0 || errwrite == 1 {
        let fd = unsafe { BorrowedFd::borrow_raw(errwrite) };
        let dup = nix::unistd::dup(fd)?;
        set_inheritable(dup.as_fd(), true)?;
        errwrite = dup.as_raw_fd();
    }

    dup_into_stdio(p2cread, 0)?;
    dup_into_stdio(c2pwrite, 1)?;
    dup_into_stdio(errwrite, 2)?;
    Ok(())
}

fn dup_into_stdio(fd: i32, io_fd: i32) -> nix::Result<()> {
    if fd < 0 {
        return Ok(());
    }
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    if fd.as_raw_fd() == io_fd {
        set_inheritable(fd, true)
    } else {
        match io_fd {
            0 => nix::unistd::dup2_stdin(fd),
            1 => nix::unistd::dup2_stdout(fd),
            2 => nix::unistd::dup2_stderr(fd),
            _ => unreachable!(),
        }
    }
}

pub fn chdir(cwd: &CStr) -> nix::Result<()> {
    nix::unistd::chdir(cwd)
}

pub fn set_umask(child_umask: i32) {
    if child_umask >= 0 {
        unsafe { libc::umask(child_umask as libc::mode_t) };
    }
}

pub fn restore_signals() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        libc::signal(libc::SIGXFSZ, libc::SIG_DFL);
    }
}

pub fn setsid_if_needed(call_setsid: bool) -> nix::Result<()> {
    if call_setsid {
        nix::unistd::setsid()?;
    }
    Ok(())
}

pub fn setpgid_if_needed(pgid_to_set: libc::pid_t) -> nix::Result<()> {
    if pgid_to_set > -1 {
        nix::unistd::setpgid(
            nix::unistd::Pid::from_raw(0),
            nix::unistd::Pid::from_raw(pgid_to_set),
        )?;
    }
    Ok(())
}

pub fn setgroups_if_needed(_groups: Option<&[nix::unistd::Gid]>) -> nix::Result<()> {
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
    if let Some(groups) = _groups {
        nix::unistd::setgroups(groups)?;
    }
    Ok(())
}

pub fn setregid_if_needed(gid: Option<nix::unistd::Gid>) -> nix::Result<()> {
    if let Some(gid) = gid.filter(|x| x.as_raw() != u32::MAX) {
        let ret = unsafe { libc::setregid(gid.as_raw(), gid.as_raw()) };
        nix::Error::result(ret)?;
    }
    Ok(())
}

pub fn setreuid_if_needed(uid: Option<nix::unistd::Uid>) -> nix::Result<()> {
    if let Some(uid) = uid.filter(|x| x.as_raw() != u32::MAX) {
        let ret = unsafe { libc::setreuid(uid.as_raw(), uid.as_raw()) };
        nix::Error::result(ret)?;
    }
    Ok(())
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn exec_replace<T: AsRef<CStr>>(
    exec_list: &[T],
    argv: *const *const libc::c_char,
    envp: Option<*const *const libc::c_char>,
) -> nix::errno::Errno {
    let mut first_err = None;
    for exec in exec_list {
        if let Some(envp) = envp {
            unsafe { libc::execve(exec.as_ref().as_ptr(), argv, envp) };
        } else {
            unsafe { libc::execv(exec.as_ref().as_ptr(), argv) };
        }
        let e = nix::errno::Errno::last();
        if e != nix::errno::Errno::ENOENT && e != nix::errno::Errno::ENOTDIR && first_err.is_none()
        {
            first_err = Some(e);
        }
    }
    first_err.unwrap_or_else(nix::errno::Errno::last)
}

fn should_keep(above: i32, keep: &[BorrowedFd<'_>], fd: i32) -> bool {
    fd > above
        && keep
            .binary_search_by_key(&fd, BorrowedFd::as_raw_fd)
            .is_err()
}

#[cfg(not(target_os = "redox"))]
fn close_dir_fds(above: i32, keep: &[BorrowedFd<'_>]) -> nix::Result<()> {
    use nix::{dir::Dir, fcntl::OFlag};
    use std::os::fd::AsRawFd;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple",
    ))]
    let fd_dir_name = c"/dev/fd";

    #[cfg(any(target_os = "linux", target_os = "android"))]
    let fd_dir_name = c"/proc/self/fd";

    let mut dir = Dir::open(
        fd_dir_name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY,
        nix::sys::stat::Mode::empty(),
    )?;
    let dirfd = dir.as_raw_fd();
    'outer: for e in dir.iter() {
        let e = e?;
        let mut parser = IntParser::default();
        for &c in e.file_name().to_bytes() {
            if parser.feed(c).is_err() {
                continue 'outer;
            }
        }
        let fd = parser.num;
        if fd != dirfd && should_keep(above, keep, fd) {
            let _ = nix::unistd::close(fd);
        }
    }
    Ok(())
}

#[cfg(target_os = "redox")]
fn close_filetable_fds(above: i32, keep: &[BorrowedFd<'_>]) -> nix::Result<()> {
    use nix::fcntl;
    use std::os::fd::AsRawFd;

    let filetable = fcntl::open(
        c"/scheme/thisproc/current/filetable",
        fcntl::OFlag::O_RDONLY,
        nix::sys::stat::Mode::empty(),
    )?;
    let read_one = || -> nix::Result<_> {
        let mut byte = 0;
        let n = nix::unistd::read(&filetable, std::slice::from_mut(&mut byte))?;
        Ok((n > 0).then_some(byte))
    };
    while let Some(c) = read_one()? {
        let mut parser = IntParser::default();
        if parser.feed(c).is_err() {
            continue;
        }
        let done = loop {
            let Some(c) = read_one()? else { break true };
            if parser.feed(c).is_err() {
                break false;
            }
        };

        let fd = parser.num;
        if fd != filetable.as_raw_fd() && should_keep(above, keep, fd) {
            let _ = nix::unistd::close(fd);
        }
        if done {
            break;
        }
    }
    Ok(())
}

fn close_fds_brute_force(above: i32, keep: &[BorrowedFd<'_>]) {
    let max_fd = nix::unistd::sysconf(nix::unistd::SysconfVar::OPEN_MAX)
        .ok()
        .flatten()
        .unwrap_or(256) as i32;

    let mut prev = above;
    for fd in keep
        .iter()
        .map(BorrowedFd::as_raw_fd)
        .chain(core::iter::once(max_fd))
    {
        for candidate in prev + 1..fd {
            unsafe { libc::close(candidate) };
        }
        prev = fd;
    }
}

#[derive(Default)]
struct IntParser {
    num: i32,
}

struct NonDigit;

impl IntParser {
    fn feed(&mut self, c: u8) -> Result<(), NonDigit> {
        let digit = (c as char).to_digit(10).ok_or(NonDigit)?;
        self.num *= 10;
        self.num += digit as i32;
        Ok(())
    }
}

use alloc::ffi::CString;
use core::ffi::CStr;
use std::ffi::{OsStr, OsString};
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd};
use std::path::Path;

pub struct UnameInfo {
    pub sysname: String,
    pub nodename: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub type PriorityWhichType = libc::__priority_which_t;
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub type PriorityWhichType = libc::c_int;

#[cfg(target_os = "freebsd")]
pub type PriorityWhoType = i32;
#[cfg(not(target_os = "freebsd"))]
pub type PriorityWhoType = u32;

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
#[derive(Clone, Debug)]
pub enum PosixSpawnFileAction {
    Open {
        fd: i32,
        path: CString,
        oflag: i32,
        mode: u32,
    },
    Close {
        fd: i32,
    },
    Dup2 {
        fd: i32,
        newfd: i32,
    },
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
pub struct PosixSpawnConfig<'a> {
    pub path: &'a CStr,
    pub args: &'a [CString],
    pub env: &'a [CString],
    pub file_actions: &'a [PosixSpawnFileAction],
    pub setsigdef: Option<&'a [i32]>,
    pub setpgroup: Option<libc::pid_t>,
    pub resetids: bool,
    pub setsid: bool,
    pub setsigmask: Option<&'a [i32]>,
    pub spawnp: bool,
}

pub fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool) -> std::io::Result<()> {
    use nix::fcntl;

    let flags = fcntl::FdFlag::from_bits_truncate(
        fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD).map_err(std::io::Error::from)?,
    );
    let mut new_flags = flags;
    new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
    if flags != new_flags {
        fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags)).map_err(std::io::Error::from)?;
    }
    Ok(())
}

pub fn is_session_leader() -> bool {
    unsafe { libc::getsid(0) == libc::getpid() }
}

pub fn getpid() -> libc::pid_t {
    unsafe { libc::getpid() }
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn dup_fd(fd: BorrowedFd<'_>) -> std::io::Result<std::os::fd::OwnedFd> {
    nix::unistd::dup(fd).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn symlinkat(src: &CStr, dir_fd: BorrowedFd<'_>, dst: &CStr) -> std::io::Result<()> {
    nix::unistd::symlinkat(src, dir_fd, dst).map_err(std::io::Error::from)
}

#[cfg(target_os = "redox")]
pub fn symlink(src: &CStr, dst: &CStr) -> std::io::Result<()> {
    let ret = unsafe { libc::symlink(src.as_ptr(), dst.as_ptr()) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "redox"))]
pub fn chroot(path: &Path) -> std::io::Result<()> {
    nix::unistd::chroot(path).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn unlinkat(dir_fd: i32, path: &CStr) -> std::io::Result<()> {
    let ret = unsafe { libc::unlinkat(dir_fd, path.as_ptr(), 0) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "redox"))]
pub fn mknod(path: &CStr, mode: libc::mode_t, device: libc::dev_t) -> std::io::Result<()> {
    let ret = unsafe { libc::mknod(path.as_ptr(), mode, device) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(not(target_os = "redox"), not(target_vendor = "apple")))]
pub fn mknodat(
    dir_fd: i32,
    path: &CStr,
    mode: libc::mode_t,
    device: libc::dev_t,
) -> std::io::Result<()> {
    let ret = unsafe { libc::mknodat(dir_fd, path.as_ptr(), mode, device) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn uid_from_raw(uid: u32) -> nix::unistd::Uid {
    nix::unistd::Uid::from_raw(uid)
}

fn gid_from_raw(gid: u32) -> nix::unistd::Gid {
    nix::unistd::Gid::from_raw(gid)
}

pub fn fchown(fd: BorrowedFd<'_>, uid: Option<u32>, gid: Option<u32>) -> std::io::Result<()> {
    nix::unistd::fchown(fd, uid.map(uid_from_raw), gid.map(gid_from_raw))
        .map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn fchdir(fd: i32) -> std::io::Result<()> {
    let ret = unsafe { libc::fchdir(fd) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn fork() -> std::io::Result<libc::pid_t> {
    let pid = unsafe { libc::fork() };
    if pid == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(pid)
    }
}

pub fn write_fd(fd: BorrowedFd<'_>, buf: &[u8]) -> std::io::Result<usize> {
    nix::unistd::write(fd, buf).map_err(std::io::Error::from)
}

pub fn fchownat(
    dir_fd: BorrowedFd<'_>,
    path: &OsStr,
    uid: Option<u32>,
    gid: Option<u32>,
    follow_symlinks: bool,
) -> std::io::Result<()> {
    let flag = if follow_symlinks {
        nix::fcntl::AtFlags::empty()
    } else {
        nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW
    };
    nix::unistd::fchownat(
        dir_fd,
        path,
        uid.map(uid_from_raw),
        gid.map(gid_from_raw),
        flag,
    )
    .map_err(std::io::Error::from)
}

pub fn uname_info() -> std::io::Result<UnameInfo> {
    let info = uname::uname()?;
    Ok(UnameInfo {
        sysname: info.sysname,
        nodename: info.nodename,
        release: info.release,
        version: info.version,
        machine: info.machine,
    })
}

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub fn pipe2(flags: libc::c_int) -> nix::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    nix::unistd::pipe2(nix::fcntl::OFlag::from_bits_truncate(flags))
}

#[cfg(not(target_os = "redox"))]
pub fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let (rfd, wfd) = nix::unistd::pipe().map_err(std::io::Error::from)?;
    set_inheritable(rfd.as_fd(), false)?;
    set_inheritable(wfd.as_fd(), false)?;
    Ok((rfd, wfd))
}

pub fn sched_yield() -> std::io::Result<()> {
    nix::sched::sched_yield().map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn nice(increment: i32) -> std::io::Result<i32> {
    crate::os::clear_errno();
    let res = unsafe { libc::nice(increment) };
    if res == -1 && crate::os::get_errno() != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(res)
    }
}

#[cfg(not(target_os = "redox"))]
pub fn sched_get_priority_max(policy: i32) -> std::io::Result<i32> {
    let max = unsafe { libc::sched_get_priority_max(policy) };
    if max == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(max)
    }
}

#[cfg(not(target_os = "redox"))]
pub fn sched_get_priority_min(policy: i32) -> std::io::Result<i32> {
    let min = unsafe { libc::sched_get_priority_min(policy) };
    if min == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(min)
    }
}

#[cfg(not(target_os = "redox"))]
pub fn fchmod(fd: BorrowedFd<'_>, mode: u32) -> std::io::Result<()> {
    nix::sys::stat::fchmod(
        fd,
        nix::sys::stat::Mode::from_bits_truncate(mode as libc::mode_t),
    )
    .map_err(std::io::Error::from)
}

#[cfg(target_os = "redox")]
pub fn utimes(
    path: &Path,
    acc: core::time::Duration,
    modif: core::time::Duration,
) -> std::io::Result<()> {
    let tv = |d: core::time::Duration| libc::timeval {
        tv_sec: d.as_secs() as _,
        tv_usec: d.as_micros() as _,
    };
    nix::sys::stat::utimes(path, &tv(acc).into(), &tv(modif).into()).map_err(std::io::Error::from)
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
    Os(i32),
}

impl From<std::io::Error> for AccessError {
    fn from(value: std::io::Error) -> Self {
        Self::Os(value.raw_os_error().unwrap_or(0))
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
    file_owner: u32,
    file_group: u32,
) -> std::io::Result<Permissions> {
    let owner_mode = (mode & 0o700) >> 6;
    let owner_permissions = get_permissions(owner_mode);

    let group_mode = (mode & 0o070) >> 3;
    let group_permissions = get_permissions(group_mode);

    let others_mode = mode & 0o007;
    let others_permissions = get_permissions(others_mode);

    let user_id = nix::unistd::getuid().as_raw();
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
pub fn getgroups() -> std::io::Result<Vec<u32>> {
    use core::ptr;
    use libc::{c_int, gid_t};
    use nix::errno::Errno;

    let ret = unsafe { libc::getgroups(0, ptr::null_mut()) };
    let mut groups =
        Vec::<gid_t>::with_capacity(Errno::result(ret).map_err(std::io::Error::from)? as usize);
    let ret = unsafe { libc::getgroups(groups.capacity() as c_int, groups.as_mut_ptr()) };

    Errno::result(ret).map_err(std::io::Error::from).map(|s| {
        unsafe { groups.set_len(s as usize) };
        groups.into_iter().collect()
    })
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "redox")))]
pub fn getgroups() -> std::io::Result<Vec<u32>> {
    nix::unistd::getgroups()
        .map(|groups| groups.into_iter().map(|gid| gid.as_raw()).collect())
        .map_err(std::io::Error::from)
}

#[cfg(target_os = "redox")]
pub fn getgroups() -> std::io::Result<Vec<u32>> {
    Err(std::io::Error::from_raw_os_error(libc::EOPNOTSUPP))
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

    let perm = get_right_permission(metadata.mode(), metadata.uid(), metadata.gid())?;

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
) -> std::io::Result<()> {
    for &fd in fds_to_keep {
        if fd.as_raw_fd() != errpipe_write.as_raw_fd() {
            set_inheritable(fd, true)?;
        }
    }

    for fd in [p2cwrite, c2pread, errread] {
        if fd >= 0 {
            nix::unistd::close(fd).map_err(std::io::Error::from)?;
        }
    }
    nix::unistd::close(errpipe_read).map_err(std::io::Error::from)?;

    let c2pwrite = if c2pwrite == 0 {
        let fd = unsafe { BorrowedFd::borrow_raw(c2pwrite) };
        let dup = nix::unistd::dup(fd).map_err(std::io::Error::from)?;
        set_inheritable(dup.as_fd(), true)?;
        dup.into_raw_fd()
    } else {
        c2pwrite
    };

    let mut errwrite = errwrite;
    while errwrite == 0 || errwrite == 1 {
        let fd = unsafe { BorrowedFd::borrow_raw(errwrite) };
        let dup = nix::unistd::dup(fd).map_err(std::io::Error::from)?;
        set_inheritable(dup.as_fd(), true)?;
        errwrite = dup.into_raw_fd();
    }

    dup_into_stdio(p2cread, 0)?;
    dup_into_stdio(c2pwrite, 1)?;
    dup_into_stdio(errwrite, 2)?;
    Ok(())
}

fn dup_into_stdio(fd: i32, io_fd: i32) -> std::io::Result<()> {
    if fd < 0 {
        return Ok(());
    }
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    if fd.as_raw_fd() == io_fd {
        set_inheritable(fd, true)
    } else {
        match io_fd {
            0 => nix::unistd::dup2_stdin(fd).map_err(std::io::Error::from),
            1 => nix::unistd::dup2_stdout(fd).map_err(std::io::Error::from),
            2 => nix::unistd::dup2_stderr(fd).map_err(std::io::Error::from),
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

pub fn umask(mask: libc::mode_t) -> libc::mode_t {
    unsafe { libc::umask(mask) }
}

#[cfg(not(any(target_os = "redox", target_os = "android")))]
pub fn sync() {
    unsafe { libc::sync() };
}

pub fn getlogin() -> Option<CString> {
    let ptr = unsafe { libc::getlogin() };
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(ptr) }.to_owned())
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

pub fn setgroups_if_needed(_groups: Option<&[u32]>) -> nix::Result<()> {
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
    if let Some(groups) = _groups {
        let groups = groups.iter().copied().map(gid_from_raw).collect::<Vec<_>>();
        nix::unistd::setgroups(&groups)?;
    }
    Ok(())
}

pub fn setregid_if_needed(gid: Option<u32>) -> nix::Result<()> {
    if let Some(gid) = gid.filter(|&x| x != u32::MAX) {
        let ret = unsafe { libc::setregid(gid as libc::gid_t, gid as libc::gid_t) };
        nix::Error::result(ret)?;
    }
    Ok(())
}

pub fn setreuid_if_needed(uid: Option<u32>) -> nix::Result<()> {
    if let Some(uid) = uid.filter(|&x| x != u32::MAX) {
        let ret = unsafe { libc::setreuid(uid as libc::uid_t, uid as libc::uid_t) };
        nix::Error::result(ret)?;
    }
    Ok(())
}

pub fn getppid() -> libc::pid_t {
    nix::unistd::getppid().as_raw()
}

pub fn getgid() -> u32 {
    nix::unistd::getgid().as_raw()
}

pub fn getegid() -> u32 {
    nix::unistd::getegid().as_raw()
}

pub fn getpgid(pid: u32) -> std::io::Result<libc::pid_t> {
    nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid as i32)))
        .map(nix::unistd::Pid::as_raw)
        .map_err(std::io::Error::from)
}

pub fn getpgrp() -> libc::pid_t {
    nix::unistd::getpgrp().as_raw()
}

#[cfg(not(target_os = "redox"))]
pub fn getsid(pid: u32) -> std::io::Result<libc::pid_t> {
    nix::unistd::getsid(Some(nix::unistd::Pid::from_raw(pid as i32)))
        .map(nix::unistd::Pid::as_raw)
        .map_err(std::io::Error::from)
}

pub fn getuid() -> u32 {
    nix::unistd::getuid().as_raw()
}

pub fn geteuid() -> u32 {
    nix::unistd::geteuid().as_raw()
}

#[cfg(not(any(target_os = "wasi", target_os = "android")))]
pub fn setgid(gid: u32) -> std::io::Result<()> {
    nix::unistd::setgid(gid_from_raw(gid)).map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
pub fn setegid(egid: u32) -> std::io::Result<()> {
    nix::unistd::setegid(gid_from_raw(egid)).map_err(std::io::Error::from)
}

pub fn setpgid(pid: u32, pgid: u32) -> std::io::Result<()> {
    nix::unistd::setpgid(
        nix::unistd::Pid::from_raw(pid as i32),
        nix::unistd::Pid::from_raw(pgid as i32),
    )
    .map_err(std::io::Error::from)
}

pub fn setpgrp() -> std::io::Result<()> {
    nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
        .map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "redox")))]
pub fn setsid() -> std::io::Result<()> {
    nix::unistd::setsid()
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "redox")))]
pub fn tcgetpgrp(fd: BorrowedFd<'_>) -> std::io::Result<libc::pid_t> {
    nix::unistd::tcgetpgrp(fd)
        .map(nix::unistd::Pid::as_raw)
        .map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "redox")))]
pub fn tcsetpgrp(fd: BorrowedFd<'_>, pgid: libc::pid_t) -> std::io::Result<()> {
    nix::unistd::tcsetpgrp(fd, nix::unistd::Pid::from_raw(pgid)).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn getpriority(which: PriorityWhichType, who: PriorityWhoType) -> std::io::Result<i32> {
    crate::os::clear_errno();
    let retval = unsafe { libc::getpriority(which, who) };
    if crate::os::get_errno() != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(retval)
    }
}

#[cfg(not(target_os = "redox"))]
pub fn setpriority(
    which: PriorityWhichType,
    who: PriorityWhoType,
    priority: i32,
) -> std::io::Result<()> {
    let retval = unsafe { libc::setpriority(which, who, priority) };
    if retval == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn waitpid(pid: libc::pid_t, status: &mut i32, opt: i32) -> std::io::Result<libc::pid_t> {
    let res = unsafe { libc::waitpid(pid, status, opt) };
    if res == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(res)
    }
}

pub fn kill(pid: i32, sig: i32) -> std::io::Result<()> {
    let ret = unsafe { libc::kill(pid, sig) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "wasi", target_os = "android")))]
pub fn setuid(uid: u32) -> std::io::Result<()> {
    nix::unistd::setuid(uid_from_raw(uid)).map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
pub fn seteuid(euid: u32) -> std::io::Result<()> {
    nix::unistd::seteuid(uid_from_raw(euid)).map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
pub fn setreuid(ruid: u32, euid: u32) -> std::io::Result<()> {
    let ret = unsafe { libc::setreuid(ruid as libc::uid_t, euid as libc::uid_t) };
    nix::Error::result(ret)
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
pub fn setresuid(ruid: u32, euid: u32, suid: u32) -> std::io::Result<()> {
    let ret = unsafe {
        libc::setresuid(
            ruid as libc::uid_t,
            euid as libc::uid_t,
            suid as libc::uid_t,
        )
    };
    nix::Error::result(ret)
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(not(target_os = "redox"))]
pub fn openpty() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let pty = nix::pty::openpty(None, None).map_err(std::io::Error::from)?;
    set_inheritable(pty.master.as_fd(), false)?;
    set_inheritable(pty.slave.as_fd(), false)?;
    Ok((pty.master, pty.slave))
}

pub fn ttyname(fd: BorrowedFd<'_>) -> std::io::Result<OsString> {
    nix::unistd::ttyname(fd)
        .map(std::path::PathBuf::into_os_string)
        .map_err(std::io::Error::from)
}

pub fn execv(path: &CStr, argv: &[&CStr]) -> std::io::Result<()> {
    match nix::unistd::execv(path, argv) {
        Ok(never) => match never {},
        Err(err) => Err(err.into()),
    }
}

pub fn execve(path: &CStr, argv: &[&CStr], env: &[&CStr]) -> std::io::Result<()> {
    match nix::unistd::execve(path, argv, env) {
        Ok(never) => match never {},
        Err(err) => Err(err.into()),
    }
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
pub fn getresuid() -> std::io::Result<(u32, u32, u32)> {
    let ret = nix::unistd::getresuid().map_err(std::io::Error::from)?;
    Ok((
        ret.real.as_raw(),
        ret.effective.as_raw(),
        ret.saved.as_raw(),
    ))
}

#[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
pub fn getresgid() -> std::io::Result<(u32, u32, u32)> {
    let ret = nix::unistd::getresgid().map_err(std::io::Error::from)?;
    Ok((
        ret.real.as_raw(),
        ret.effective.as_raw(),
        ret.saved.as_raw(),
    ))
}

#[cfg(any(target_os = "freebsd", target_os = "linux", target_os = "openbsd"))]
pub fn setresgid(rgid: u32, egid: u32, sgid: u32) -> std::io::Result<()> {
    let ret = unsafe {
        libc::setresgid(
            rgid as libc::gid_t,
            egid as libc::gid_t,
            sgid as libc::gid_t,
        )
    };
    nix::Error::result(ret)
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
pub fn setregid(rgid: u32, egid: u32) -> std::io::Result<()> {
    let ret = unsafe { libc::setregid(rgid as libc::gid_t, egid as libc::gid_t) };
    nix::Error::result(ret)
        .map(drop)
        .map_err(std::io::Error::from)
}

#[cfg(any(target_os = "freebsd", target_os = "linux", target_os = "openbsd"))]
pub fn initgroups(user: &CStr, gid: u32) -> std::io::Result<()> {
    nix::unistd::initgroups(user, gid_from_raw(gid)).map_err(std::io::Error::from)
}

#[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
pub fn setgroups_raw(groups: &[u32]) -> std::io::Result<()> {
    let gids = groups.iter().copied().map(gid_from_raw).collect::<Vec<_>>();
    nix::unistd::setgroups(&gids).map_err(std::io::Error::from)
}

pub fn dup_noninheritable(fd: BorrowedFd<'_>) -> std::io::Result<OwnedFd> {
    let fd = nix::unistd::dup(fd).map_err(std::io::Error::from)?;
    set_inheritable(fd.as_fd(), false)?;
    Ok(fd)
}

pub fn dup2(fd: BorrowedFd<'_>, fd2: OwnedFd, inheritable: bool) -> std::io::Result<OwnedFd> {
    let mut fd2 = core::mem::ManuallyDrop::new(fd2);
    nix::unistd::dup2(fd, &mut fd2).map_err(std::io::Error::from)?;
    let fd2 = core::mem::ManuallyDrop::into_inner(fd2);
    if !inheritable {
        set_inheritable(fd2.as_fd(), false)?;
    }
    Ok(fd2)
}

pub fn get_terminal_size(fd: libc::c_int) -> std::io::Result<(u16, u16)> {
    let mut w = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut w) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok((w.ws_col, w.ws_row))
    }
}

pub fn pathconf(path: &CStr, name: i32) -> std::io::Result<Option<libc::c_long>> {
    crate::os::clear_errno();
    debug_assert_eq!(crate::os::get_errno(), 0);
    let raw = unsafe { libc::pathconf(path.as_ptr(), name) };
    if raw == -1 {
        if crate::os::get_errno() == 0 {
            Ok(None)
        } else {
            Err(std::io::Error::last_os_error())
        }
    } else {
        Ok(Some(raw))
    }
}

pub fn fpathconf(fd: i32, name: i32) -> std::io::Result<Option<libc::c_long>> {
    crate::os::clear_errno();
    debug_assert_eq!(crate::os::get_errno(), 0);
    let raw = unsafe { libc::fpathconf(fd, name) };
    if raw == -1 {
        if crate::os::get_errno() == 0 {
            Ok(None)
        } else {
            Err(std::io::Error::last_os_error())
        }
    } else {
        Ok(Some(raw))
    }
}

pub fn sysconf(name: i32) -> std::io::Result<libc::c_long> {
    crate::os::set_errno(0);
    let raw = unsafe { libc::sysconf(name) };
    if raw == -1 && crate::os::get_errno() != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(raw)
    }
}

#[cfg(target_os = "linux")]
pub unsafe fn getrandom(
    buf: *mut libc::c_void,
    buflen: usize,
    flags: u32,
) -> std::io::Result<usize> {
    let len = unsafe { libc::syscall(libc::SYS_getrandom, buf, buflen, flags as usize) as isize };
    if len < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(len as usize)
    }
}

pub fn wcoredump(status: i32) -> bool {
    libc::WCOREDUMP(status)
}

pub fn wifcontinued(status: i32) -> bool {
    libc::WIFCONTINUED(status)
}

pub fn wifstopped(status: i32) -> bool {
    libc::WIFSTOPPED(status)
}

pub fn wifsignaled(status: i32) -> bool {
    libc::WIFSIGNALED(status)
}

pub fn wifexited(status: i32) -> bool {
    libc::WIFEXITED(status)
}

pub fn wexitstatus(status: i32) -> i32 {
    libc::WEXITSTATUS(status)
}

pub fn wstopsig(status: i32) -> i32 {
    libc::WSTOPSIG(status)
}

pub fn wtermsig(status: i32) -> i32 {
    libc::WTERMSIG(status)
}

#[cfg(target_os = "linux")]
pub fn pidfd_open(pid: libc::pid_t, flags: u32) -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) as libc::c_long };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
    }
}

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "openbsd"
))]
pub fn getgrouplist(user: &CStr, gid: u32) -> std::io::Result<Vec<u32>> {
    nix::unistd::getgrouplist(user, gid_from_raw(gid))
        .map(|groups| groups.into_iter().map(|gid| gid.as_raw()).collect())
        .map_err(std::io::Error::from)
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
pub fn validate_posix_spawn_signal(sig: i32) -> bool {
    nix::sys::signal::Signal::try_from(sig).is_ok()
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
pub const fn supports_posix_spawn_setsid() -> bool {
    cfg!(any(
        target_os = "linux",
        target_os = "haiku",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "hurd",
    ))
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
fn build_posix_spawn_file_actions(
    actions: &[PosixSpawnFileAction],
) -> std::io::Result<nix::spawn::PosixSpawnFileActions> {
    let mut file_actions =
        nix::spawn::PosixSpawnFileActions::init().map_err(std::io::Error::from)?;
    for action in actions {
        match action {
            PosixSpawnFileAction::Open {
                fd,
                path,
                oflag,
                mode,
            } => file_actions
                .add_open(
                    *fd,
                    path.as_c_str(),
                    nix::fcntl::OFlag::from_bits_retain(*oflag),
                    nix::sys::stat::Mode::from_bits_retain(*mode as libc::mode_t),
                )
                .map_err(std::io::Error::from)?,
            PosixSpawnFileAction::Close { fd } => {
                file_actions.add_close(*fd).map_err(std::io::Error::from)?
            }
            PosixSpawnFileAction::Dup2 { fd, newfd } => file_actions
                .add_dup2(*fd, *newfd)
                .map_err(std::io::Error::from)?,
        }
    }
    Ok(file_actions)
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
fn build_sigset(signals: &[i32]) -> nix::sys::signal::SigSet {
    let mut set = nix::sys::signal::SigSet::empty();
    for &sig in signals {
        let sig = nix::sys::signal::Signal::try_from(sig).expect("validated signal");
        set.add(sig);
    }
    set
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
fn build_posix_spawn_attrs(
    config: &PosixSpawnConfig<'_>,
) -> std::io::Result<nix::spawn::PosixSpawnAttr> {
    let mut attrp = nix::spawn::PosixSpawnAttr::init().map_err(std::io::Error::from)?;
    let mut flags = nix::spawn::PosixSpawnFlags::empty();

    if let Some(sigs) = config.setsigdef {
        let set = build_sigset(sigs);
        attrp.set_sigdefault(&set).map_err(std::io::Error::from)?;
        flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF);
    }

    if let Some(pgid) = config.setpgroup {
        attrp
            .set_pgroup(nix::unistd::Pid::from_raw(pgid))
            .map_err(std::io::Error::from)?;
        flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETPGROUP);
    }

    if config.resetids {
        flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_RESETIDS);
    }

    if config.setsid {
        #[cfg(any(
            target_os = "linux",
            target_os = "haiku",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "hurd",
        ))]
        {
            flags.insert(nix::spawn::PosixSpawnFlags::from_bits_retain(
                libc::POSIX_SPAWN_SETSID,
            ));
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "haiku",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "hurd",
        )))]
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "setsid parameter is not supported on this platform",
            ));
        }
    }

    if let Some(sigs) = config.setsigmask {
        let set = build_sigset(sigs);
        attrp.set_sigmask(&set).map_err(std::io::Error::from)?;
        flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK);
    }

    if !flags.is_empty() {
        attrp.set_flags(flags).map_err(std::io::Error::from)?;
    }

    Ok(attrp)
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
pub fn posix_spawn(config: PosixSpawnConfig<'_>) -> std::io::Result<libc::pid_t> {
    let file_actions = build_posix_spawn_file_actions(config.file_actions)?;
    let attrp = build_posix_spawn_attrs(&config)?;
    let pid = if config.spawnp {
        nix::spawn::posix_spawnp(config.path, &file_actions, &attrp, config.args, config.env)
    } else {
        nix::spawn::posix_spawn(config.path, &file_actions, &attrp, config.args, config.env)
    }
    .map_err(std::io::Error::from)?;
    Ok(pid.into())
}

#[cfg(target_os = "linux")]
pub fn sendfile(
    out_fd: BorrowedFd<'_>,
    in_fd: BorrowedFd<'_>,
    offset: &mut crate::crt_fd::Offset,
    count: usize,
) -> std::io::Result<usize> {
    nix::sys::sendfile::sendfile(out_fd, in_fd, Some(offset), count).map_err(std::io::Error::from)
}

#[cfg(target_os = "macos")]
pub fn sendfile(
    in_fd: BorrowedFd<'_>,
    out_fd: BorrowedFd<'_>,
    offset: crate::crt_fd::Offset,
    count: i64,
    headers: Option<&[&[u8]]>,
    trailers: Option<&[&[u8]]>,
) -> (std::io::Result<()>, i64) {
    let (res, written) =
        nix::sys::sendfile::sendfile(in_fd, out_fd, offset, Some(count), headers, trailers);
    (res.map_err(std::io::Error::from), written)
}

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "netbsd"
))]
pub fn sched_getscheduler(pid: libc::pid_t) -> std::io::Result<i32> {
    let policy = unsafe { libc::sched_getscheduler(pid) };
    if policy == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(policy)
    }
}

#[cfg(all(
    not(target_env = "musl"),
    any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd"
    )
))]
pub fn sched_setscheduler(
    pid: i32,
    policy: i32,
    param: &libc::sched_param,
) -> std::io::Result<i32> {
    let ret = unsafe { libc::sched_setscheduler(pid, policy, param) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

#[cfg(any(
    target_os = "android",
    target_os = "freebsd",
    target_os = "linux",
    target_os = "netbsd"
))]
pub fn sched_getparam(pid: libc::pid_t) -> std::io::Result<libc::sched_param> {
    let mut param = core::mem::MaybeUninit::uninit();
    let ret = unsafe { libc::sched_getparam(pid, param.as_mut_ptr()) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { param.assume_init() })
    }
}

#[cfg(all(
    not(target_env = "musl"),
    any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd"
    )
))]
pub fn sched_setparam(pid: i32, param: &libc::sched_param) -> std::io::Result<i32> {
    let ret = unsafe { libc::sched_setparam(pid, param) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(ret)
    }
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

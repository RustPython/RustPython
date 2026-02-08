// spell-checker:disable

use std::os::fd::BorrowedFd;

pub(crate) use module::module_def;

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

#[pymodule(name = "posix", with(
    super::os::_os,
    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    posix_sched
))]
pub mod module {
    use crate::{
        AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyInt, PyListRef, PyStr, PyTupleRef},
        convert::{IntoPyException, ToPyObject, TryFromObject},
        exceptions::OSErrorBuilder,
        function::{Either, KwArgs, OptionalArg},
        ospath::{OsPath, OsPathOrFd},
        stdlib::os::{
            _os, DirFd, FollowSymlinks, SupportFunc, TargetIsDirectory, fs_metadata,
            warn_if_bool_fd,
        },
    };
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    use crate::{builtins::PyStrRef, utils::ToCString};
    use alloc::ffi::CString;
    use bitflags::bitflags;
    use core::ffi::CStr;
    use nix::{
        fcntl,
        unistd::{self, Gid, Pid, Uid},
    };
    use std::{
        env, fs, io,
        os::fd::{AsFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd},
    };
    use strum_macros::{EnumIter, EnumString};

    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use libc::{SCHED_DEADLINE, SCHED_NORMAL};

    #[cfg(target_os = "freebsd")]
    #[pyattr]
    use libc::{MFD_HUGE_MASK, SF_MNOWAIT, SF_NOCACHE, SF_NODISKIO, SF_SYNC};

    #[cfg(target_os = "linux")]
    #[pyattr]
    use libc::PIDFD_NONBLOCK;

    #[cfg(target_os = "macos")]
    #[pyattr]
    use libc::{
        COPYFILE_DATA as _COPYFILE_DATA, O_EVTONLY, O_NOFOLLOW_ANY, PRIO_DARWIN_BG,
        PRIO_DARWIN_NONUI, PRIO_DARWIN_PROCESS, PRIO_DARWIN_THREAD,
    };

    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use libc::{
        CLONE_FILES, CLONE_FS, CLONE_NEWCGROUP, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS,
        CLONE_NEWPID, CLONE_NEWUSER, CLONE_NEWUTS, CLONE_SIGHAND, CLONE_SYSVSEM, CLONE_THREAD,
        CLONE_VM, EFD_CLOEXEC, EFD_NONBLOCK, EFD_SEMAPHORE, O_NOATIME, O_TMPFILE, P_PIDFD,
        SCHED_BATCH, SCHED_IDLE, SCHED_RESET_ON_FORK, SPLICE_F_MORE, SPLICE_F_MOVE,
        SPLICE_F_NONBLOCK,
    };

    #[cfg(any(target_os = "android", unix))]
    #[pyattr]
    use libc::{
        F_OK, O_CLOEXEC, O_DIRECTORY, O_NOFOLLOW, O_NONBLOCK, PRIO_PGRP, PRIO_PROCESS, PRIO_USER,
        R_OK, RTLD_GLOBAL, RTLD_LAZY, RTLD_LOCAL, RTLD_NOW, W_OK, WCONTINUED, WNOHANG, WUNTRACED,
        X_OK,
    };

    #[cfg(any(target_os = "macos", target_os = "redox"))]
    #[pyattr]
    use libc::O_SYMLINK;

    #[cfg(any(target_os = "android", target_os = "freebsd", target_os = "linux"))]
    #[pyattr]
    use libc::{
        MFD_ALLOW_SEALING, MFD_CLOEXEC, MFD_HUGETLB, POSIX_FADV_DONTNEED, POSIX_FADV_NOREUSE,
        POSIX_FADV_NORMAL, POSIX_FADV_RANDOM, POSIX_FADV_SEQUENTIAL, POSIX_FADV_WILLNEED,
    };

    #[cfg(any(target_os = "android", target_os = "linux", target_os = "netbsd"))]
    #[pyattr]
    use libc::{TFD_CLOEXEC, TFD_NONBLOCK, TFD_TIMER_ABSTIME, TFD_TIMER_CANCEL_ON_SET};

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "netbsd"))]
    #[pyattr]
    use libc::{XATTR_CREATE, XATTR_REPLACE};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "linux",
        target_os = "netbsd"
    ))]
    #[pyattr]
    use libc::{GRND_NONBLOCK, GRND_RANDOM};

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "redox"
    ))]
    #[pyattr]
    use libc::O_PATH;

    #[cfg(any(
        target_os = "android",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use libc::O_RSYNC;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos"
    ))]
    #[pyattr]
    use libc::{RTLD_NODELETE, SEEK_DATA, SEEK_HOLE};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd"
    ))]
    #[pyattr]
    use libc::O_DIRECT;

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd"
    ))]
    #[pyattr]
    use libc::RTLD_NOLOAD;

    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use libc::O_DSYNC;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use libc::SCHED_OTHER;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    ))]
    #[pyattr]
    use libc::{O_EXLOCK, O_FSYNC, O_SHLOCK};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use libc::{
        CLD_CONTINUED, CLD_DUMPED, CLD_EXITED, CLD_KILLED, CLD_STOPPED, CLD_TRAPPED, F_LOCK,
        F_TEST, F_TLOCK, F_ULOCK, O_SYNC, P_ALL, P_PGID, P_PID, SCHED_FIFO, SCHED_RR,
    };

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    ))]
    #[pyattr]
    use libc::{O_ASYNC, O_NDELAY, O_NOCTTY, WEXITED, WNOWAIT, WSTOPPED};

    #[pyattr]
    const EX_OK: i8 = exitcode::OK as i8;

    #[pyattr]
    const EX_USAGE: i8 = exitcode::USAGE as i8;

    #[pyattr]
    const EX_DATAERR: i8 = exitcode::DATAERR as i8;

    #[pyattr]
    const EX_NOINPUT: i8 = exitcode::NOINPUT as i8;

    #[pyattr]
    const EX_NOUSER: i8 = exitcode::NOUSER as i8;

    #[pyattr]
    const EX_NOHOST: i8 = exitcode::NOHOST as i8;

    #[pyattr]
    const EX_UNAVAILABLE: i8 = exitcode::UNAVAILABLE as i8;

    #[pyattr]
    const EX_SOFTWARE: i8 = exitcode::SOFTWARE as i8;

    #[pyattr]
    const EX_OSERR: i8 = exitcode::OSERR as i8;

    #[pyattr]
    const EX_OSFILE: i8 = exitcode::OSFILE as i8;

    #[pyattr]
    const EX_CANTCREAT: i8 = exitcode::CANTCREAT as i8;

    #[pyattr]
    const EX_IOERR: i8 = exitcode::IOERR as i8;

    #[pyattr]
    const EX_TEMPFAIL: i8 = exitcode::TEMPFAIL as i8;

    #[pyattr]
    const EX_PROTOCOL: i8 = exitcode::PROTOCOL as i8;

    #[pyattr]
    const EX_NOPERM: i8 = exitcode::NOPERM as i8;

    #[pyattr]
    const EX_CONFIG: i8 = exitcode::CONFIG as i8;

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_OPEN: i32 = PosixSpawnFileActionIdentifier::Open as i32;

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_CLOSE: i32 = PosixSpawnFileActionIdentifier::Close as i32;

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_DUP2: i32 = PosixSpawnFileActionIdentifier::Dup2 as i32;

    impl TryFromObject for BorrowedFd<'_> {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            crate::stdlib::os::warn_if_bool_fd(&obj, vm)?;
            let fd = i32::try_from_object(vm, obj)?;
            if fd == -1 {
                return Err(io::Error::from_raw_os_error(libc::EBADF).into_pyexception(vm));
            }
            // SAFETY: none, really. but, python's os api of passing around file descriptors
            //         everywhere isn't really io-safe anyway, so, this is passed to the user.
            Ok(unsafe { BorrowedFd::borrow_raw(fd) })
        }
    }

    impl TryFromObject for OwnedFd {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let fd = i32::try_from_object(vm, obj)?;
            if fd == -1 {
                return Err(io::Error::from_raw_os_error(libc::EBADF).into_pyexception(vm));
            }
            // SAFETY: none, really. but, python's os api of passing around file descriptors
            //         everywhere isn't really io-safe anyway, so, this is passed to the user.
            Ok(unsafe { Self::from_raw_fd(fd) })
        }
    }

    impl ToPyObject for OwnedFd {
        fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
            self.into_raw_fd().to_pyobject(vm)
        }
    }

    // Flags for os_access
    bitflags! {
        #[derive(Copy, Clone, Debug, PartialEq)]
        pub struct AccessFlags: u8 {
            const F_OK = _os::F_OK;
            const R_OK = _os::R_OK;
            const W_OK = _os::W_OK;
            const X_OK = _os::X_OK;
        }
    }

    struct Permissions {
        is_readable: bool,
        is_writable: bool,
        is_executable: bool,
    }

    const fn get_permissions(mode: u32) -> Permissions {
        Permissions {
            is_readable: mode & 4 != 0,
            is_writable: mode & 2 != 0,
            is_executable: mode & 1 != 0,
        }
    }

    fn get_right_permission(
        mode: u32,
        file_owner: Uid,
        file_group: Gid,
    ) -> nix::Result<Permissions> {
        let owner_mode = (mode & 0o700) >> 6;
        let owner_permissions = get_permissions(owner_mode);

        let group_mode = (mode & 0o070) >> 3;
        let group_permissions = get_permissions(group_mode);

        let others_mode = mode & 0o007;
        let others_permissions = get_permissions(others_mode);

        let user_id = nix::unistd::getuid();
        let groups_ids = getgroups_impl()?;

        if file_owner == user_id {
            Ok(owner_permissions)
        } else if groups_ids.contains(&file_group) {
            Ok(group_permissions)
        } else {
            Ok(others_permissions)
        }
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    fn getgroups_impl() -> nix::Result<Vec<Gid>> {
        use core::ptr;
        use libc::{c_int, gid_t};
        use nix::errno::Errno;
        let ret = unsafe { libc::getgroups(0, ptr::null_mut()) };
        let mut groups = Vec::<Gid>::with_capacity(Errno::result(ret)? as usize);
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
    use nix::unistd::getgroups as getgroups_impl;

    #[cfg(target_os = "redox")]
    fn getgroups_impl() -> nix::Result<Vec<Gid>> {
        Err(nix::Error::EOPNOTSUPP)
    }

    #[pyfunction]
    fn getgroups(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let group_ids = getgroups_impl().map_err(|e| e.into_pyexception(vm))?;
        Ok(group_ids
            .into_iter()
            .map(|gid| vm.ctx.new_int(gid.as_raw()).into())
            .collect())
    }

    #[pyfunction]
    pub(super) fn access(path: OsPath, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        use std::os::unix::fs::MetadataExt;

        let flags = AccessFlags::from_bits(mode).ok_or_else(|| {
            vm.new_value_error(
            "One of the flags is wrong, there are only 4 possibilities F_OK, R_OK, W_OK and X_OK",
        )
        })?;

        let metadata = match fs::metadata(&path.path) {
            Ok(m) => m,
            // If the file doesn't exist, return False for any access check
            Err(_) => return Ok(false),
        };

        // if it's only checking for F_OK
        if flags == AccessFlags::F_OK {
            return Ok(true); // File exists
        }

        let user_id = metadata.uid();
        let group_id = metadata.gid();
        let mode = metadata.mode();

        let perm = get_right_permission(mode, Uid::from_raw(user_id), Gid::from_raw(group_id))
            .map_err(|err| err.into_pyexception(vm))?;

        let r_ok = !flags.contains(AccessFlags::R_OK) || perm.is_readable;
        let w_ok = !flags.contains(AccessFlags::W_OK) || perm.is_writable;
        let x_ok = !flags.contains(AccessFlags::X_OK) || perm.is_executable;

        Ok(r_ok && w_ok && x_ok)
    }

    #[pyattr]
    fn environ(vm: &VirtualMachine) -> PyDictRef {
        use rustpython_common::os::ffi::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars_os() {
            let key: PyObjectRef = vm.ctx.new_bytes(key.into_vec()).into();
            let value: PyObjectRef = vm.ctx.new_bytes(value.into_vec()).into();
            environ.set_item(&*key, value, vm).unwrap();
        }

        environ
    }

    #[pyfunction]
    fn _create_environ(vm: &VirtualMachine) -> PyDictRef {
        use rustpython_common::os::ffi::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars_os() {
            let key: PyObjectRef = vm.ctx.new_bytes(key.into_vec()).into();
            let value: PyObjectRef = vm.ctx.new_bytes(value.into_vec()).into();
            environ.set_item(&*key, value, vm).unwrap();
        }
        environ
    }

    #[derive(FromArgs)]
    pub(super) struct SymlinkArgs<'fd> {
        src: OsPath,
        dst: OsPath,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        dir_fd: DirFd<'fd, { _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(args: SymlinkArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let src = args.src.into_cstring(vm)?;
        let dst = args.dst.into_cstring(vm)?;
        #[cfg(not(target_os = "redox"))]
        {
            nix::unistd::symlinkat(&*src, args.dir_fd.get(), &*dst)
                .map_err(|err| err.into_pyexception(vm))
        }
        #[cfg(target_os = "redox")]
        {
            let [] = args.dir_fd.0;
            let res = unsafe { libc::symlink(src.as_ptr(), dst.as_ptr()) };
            if res < 0 {
                Err(vm.new_last_errno_error())
            } else {
                Ok(())
            }
        }
    }

    #[pyfunction]
    #[pyfunction(name = "unlink")]
    fn remove(path: OsPath, dir_fd: DirFd<'_, 0>, vm: &VirtualMachine) -> PyResult<()> {
        let [] = dir_fd.0;
        fs::remove_file(&path).map_err(|err| OSErrorBuilder::with_filename(&err, path, vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchdir(fd: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        warn_if_bool_fd(&fd, vm)?;
        let fd = i32::try_from_object(vm, fd)?;
        let ret = unsafe { libc::fchdir(fd) };
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error().into_pyexception(vm))
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chroot(path: OsPath, vm: &VirtualMachine) -> PyResult<()> {
        use crate::exceptions::OSErrorBuilder;

        nix::unistd::chroot(&*path.path).map_err(|err| {
            // Use `From<nix::Error> for io::Error` when it is available
            let io_err: io::Error = err.into();
            OSErrorBuilder::with_filename(&io_err, path, vm)
        })
    }

    // As of now, redox does not seems to support chown command (cf. https://gitlab.redox-os.org/redox-os/coreutils , last checked on 05/07/2020)
    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chown(
        path: OsPathOrFd<'_>,
        uid: isize,
        gid: isize,
        dir_fd: DirFd<'_, 1>,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let uid = if uid >= 0 {
            Some(nix::unistd::Uid::from_raw(uid as u32))
        } else if uid == -1 {
            None
        } else {
            return Err(vm.new_os_error("Specified uid is not valid."));
        };

        let gid = if gid >= 0 {
            Some(nix::unistd::Gid::from_raw(gid as u32))
        } else if gid == -1 {
            None
        } else {
            return Err(vm.new_os_error("Specified gid is not valid."));
        };

        let flag = if follow_symlinks.0 {
            nix::fcntl::AtFlags::empty()
        } else {
            nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW
        };

        match path {
            OsPathOrFd::Path(ref p) => {
                nix::unistd::fchownat(dir_fd.get(), p.path.as_os_str(), uid, gid, flag)
            }
            OsPathOrFd::Fd(fd) => nix::unistd::fchown(fd, uid, gid),
        }
        .map_err(|err| {
            // Use `From<nix::Error> for io::Error` when it is available
            let err = io::Error::from_raw_os_error(err as i32);
            OSErrorBuilder::with_filename(&err, path, vm)
        })
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn lchown(path: OsPath, uid: isize, gid: isize, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            OsPathOrFd::Path(path),
            uid,
            gid,
            DirFd::default(),
            FollowSymlinks(false),
            vm,
        )
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchown(fd: BorrowedFd<'_>, uid: isize, gid: isize, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            OsPathOrFd::Fd(fd.into()),
            uid,
            gid,
            DirFd::default(),
            FollowSymlinks(true),
            vm,
        )
    }

    #[derive(FromArgs)]
    struct RegisterAtForkArgs {
        #[pyarg(named, optional)]
        before: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        after_in_parent: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        after_in_child: OptionalArg<PyObjectRef>,
    }

    impl RegisterAtForkArgs {
        fn into_validated(
            self,
            vm: &VirtualMachine,
        ) -> PyResult<(
            Option<PyObjectRef>,
            Option<PyObjectRef>,
            Option<PyObjectRef>,
        )> {
            fn into_option(
                arg: OptionalArg<PyObjectRef>,
                vm: &VirtualMachine,
            ) -> PyResult<Option<PyObjectRef>> {
                match arg {
                    OptionalArg::Present(obj) => {
                        if !obj.is_callable() {
                            return Err(vm.new_type_error("Args must be callable"));
                        }
                        Ok(Some(obj))
                    }
                    OptionalArg::Missing => Ok(None),
                }
            }
            let before = into_option(self.before, vm)?;
            let after_in_parent = into_option(self.after_in_parent, vm)?;
            let after_in_child = into_option(self.after_in_child, vm)?;
            if before.is_none() && after_in_parent.is_none() && after_in_child.is_none() {
                return Err(vm.new_type_error("At least one arg must be present"));
            }
            Ok((before, after_in_parent, after_in_child))
        }
    }

    #[pyfunction]
    fn register_at_fork(
        args: RegisterAtForkArgs,
        _ignored: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let (before, after_in_parent, after_in_child) = args.into_validated(vm)?;

        if let Some(before) = before {
            vm.state.before_forkers.lock().push(before);
        }
        if let Some(after_in_parent) = after_in_parent {
            vm.state.after_forkers_parent.lock().push(after_in_parent);
        }
        if let Some(after_in_child) = after_in_child {
            vm.state.after_forkers_child.lock().push(after_in_child);
        }
        Ok(())
    }

    fn run_at_forkers(mut funcs: Vec<PyObjectRef>, reversed: bool, vm: &VirtualMachine) {
        if !funcs.is_empty() {
            if reversed {
                funcs.reverse();
            }
            for func in funcs {
                if let Err(e) = func.call((), vm) {
                    let exit = e.fast_isinstance(vm.ctx.exceptions.system_exit);
                    vm.run_unraisable(e, Some("Exception ignored in".to_owned()), func);
                    if exit {
                        // Do nothing!
                    }
                }
            }
        }
    }

    fn py_os_before_fork(vm: &VirtualMachine) {
        let before_forkers: Vec<PyObjectRef> = vm.state.before_forkers.lock().clone();
        // functions must be executed in reversed order as they are registered
        // only for before_forkers, refer: test_register_at_fork in test_posix

        run_at_forkers(before_forkers, true, vm);
    }

    fn py_os_after_fork_child(vm: &VirtualMachine) {
        // Reset low-level state before any Python code runs in the child.
        // Signal triggers from the parent must not fire in the child.
        crate::signal::clear_after_fork();
        crate::stdlib::signal::_signal::clear_wakeup_fd_after_fork();

        // Reset weakref stripe locks that may have been held during fork.
        #[cfg(feature = "threading")]
        crate::object::reset_weakref_locks_after_fork();

        // Mark all other threads as done before running Python callbacks
        #[cfg(feature = "threading")]
        crate::stdlib::thread::after_fork_child(vm);

        // Initialize signal handlers for the child's main thread.
        // When forked from a worker thread, the OnceCell is empty.
        vm.signal_handlers.get_or_init(|| {
            Box::new(const { core::cell::RefCell::new([const { None }; crate::signal::NSIG]) })
        });

        let after_forkers_child: Vec<PyObjectRef> = vm.state.after_forkers_child.lock().clone();
        run_at_forkers(after_forkers_child, false, vm);
    }

    fn py_os_after_fork_parent(vm: &VirtualMachine) {
        let after_forkers_parent: Vec<PyObjectRef> = vm.state.after_forkers_parent.lock().clone();
        run_at_forkers(after_forkers_parent, false, vm);
    }

    /// Warn if forking from a multi-threaded process
    fn warn_if_multi_threaded(name: &str, vm: &VirtualMachine) {
        // Only check threading if it was already imported
        // Avoid vm.import() which can execute arbitrary Python code in the fork path
        let threading = match vm
            .sys_module
            .get_attr("modules", vm)
            .and_then(|m| m.get_item("threading", vm))
        {
            Ok(m) => m,
            Err(_) => return,
        };
        let active = threading.get_attr("_active", vm).ok();
        let limbo = threading.get_attr("_limbo", vm).ok();

        let count_dict = |obj: Option<crate::PyObjectRef>| -> usize {
            obj.and_then(|o| o.length_opt(vm))
                .and_then(|r| r.ok())
                .unwrap_or(0)
        };

        let num_threads = count_dict(active) + count_dict(limbo);
        if num_threads > 1 {
            // Use Python warnings module to ensure filters are applied correctly
            let Ok(warnings) = vm.import("warnings", 0) else {
                return;
            };
            let Ok(warn_fn) = warnings.get_attr("warn", vm) else {
                return;
            };

            let pid = unsafe { libc::getpid() };
            let msg = format!(
                "This process (pid={}) is multi-threaded, use of {}() may lead to deadlocks in the child.",
                pid, name
            );

            // Call warnings.warn(message, DeprecationWarning, stacklevel=2)
            // stacklevel=2 to point to the caller of fork()
            let args = crate::function::FuncArgs::new(
                vec![
                    vm.ctx.new_str(msg).into(),
                    vm.ctx.exceptions.deprecation_warning.as_object().to_owned(),
                ],
                crate::function::KwArgs::new(
                    [("stacklevel".to_owned(), vm.ctx.new_int(2).into())]
                        .into_iter()
                        .collect(),
                ),
            );
            let _ = warn_fn.call(args, vm);
        }
    }

    #[pyfunction]
    fn fork(vm: &VirtualMachine) -> i32 {
        warn_if_multi_threaded("fork", vm);

        let pid: i32;
        py_os_before_fork(vm);
        unsafe {
            pid = libc::fork();
        }
        if pid == 0 {
            py_os_after_fork_child(vm);
        } else {
            py_os_after_fork_parent(vm);
        }
        pid
    }

    #[cfg(not(target_os = "redox"))]
    const MKNOD_DIR_FD: bool = cfg!(not(target_vendor = "apple"));

    #[cfg(not(target_os = "redox"))]
    #[derive(FromArgs)]
    struct MknodArgs<'fd> {
        #[pyarg(any)]
        path: OsPath,
        #[pyarg(any)]
        mode: libc::mode_t,
        #[pyarg(any)]
        device: libc::dev_t,
        #[pyarg(flatten)]
        dir_fd: DirFd<'fd, { MKNOD_DIR_FD as usize }>,
    }

    #[cfg(not(target_os = "redox"))]
    impl MknodArgs<'_> {
        fn _mknod(self, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(unsafe {
                libc::mknod(
                    self.path.clone().into_cstring(vm)?.as_ptr(),
                    self.mode,
                    self.device,
                )
            })
        }

        #[cfg(not(target_vendor = "apple"))]
        fn mknod(self, vm: &VirtualMachine) -> PyResult<()> {
            let ret = match self.dir_fd.raw_opt() {
                None => self._mknod(vm)?,
                Some(non_default_fd) => unsafe {
                    libc::mknodat(
                        non_default_fd,
                        self.path.clone().into_cstring(vm)?.as_ptr(),
                        self.mode,
                        self.device,
                    )
                },
            };
            if ret != 0 {
                Err(vm.new_last_errno_error())
            } else {
                Ok(())
            }
        }

        #[cfg(target_vendor = "apple")]
        fn mknod(self, vm: &VirtualMachine) -> PyResult<()> {
            let [] = self.dir_fd.0;
            let ret = self._mknod(vm)?;
            if ret != 0 {
                Err(vm.new_last_errno_error())
            } else {
                Ok(())
            }
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn mknod(args: MknodArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        args.mknod(vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn nice(increment: i32, vm: &VirtualMachine) -> PyResult<i32> {
        use nix::errno::Errno;
        Errno::clear();
        let res = unsafe { libc::nice(increment) };
        if res == -1 && Errno::last_raw() != 0 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(res)
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn sched_get_priority_max(policy: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let max = unsafe { libc::sched_get_priority_max(policy) };
        if max == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(max)
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn sched_get_priority_min(policy: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let min = unsafe { libc::sched_get_priority_min(policy) };
        if min == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(min)
        }
    }

    #[pyfunction]
    fn sched_yield(vm: &VirtualMachine) -> PyResult<()> {
        nix::sched::sched_yield().map_err(|e| e.into_pyexception(vm))
    }

    #[pyfunction]
    fn get_inheritable(fd: BorrowedFd<'_>, vm: &VirtualMachine) -> PyResult<bool> {
        let flags = fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD);
        match flags {
            Ok(ret) => Ok((ret & libc::FD_CLOEXEC) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn set_inheritable(fd: BorrowedFd<'_>, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
        super::set_inheritable(fd, inheritable).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn get_blocking(fd: BorrowedFd<'_>, vm: &VirtualMachine) -> PyResult<bool> {
        let flags = fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFL);
        match flags {
            Ok(ret) => Ok((ret & libc::O_NONBLOCK) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn set_blocking(fd: BorrowedFd<'_>, blocking: bool, vm: &VirtualMachine) -> PyResult<()> {
        let _set_flag = || {
            use nix::fcntl::{FcntlArg, OFlag, fcntl};

            let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
            let mut new_flags = flags;
            new_flags.set(OFlag::from_bits_truncate(libc::O_NONBLOCK), !blocking);
            if flags != new_flags {
                fcntl(fd, FcntlArg::F_SETFL(new_flags))?;
            }
            Ok(())
        };
        _set_flag().map_err(|err: nix::Error| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn pipe(vm: &VirtualMachine) -> PyResult<(OwnedFd, OwnedFd)> {
        use nix::unistd::pipe;
        let (rfd, wfd) = pipe().map_err(|err| err.into_pyexception(vm))?;
        set_inheritable(rfd.as_fd(), false, vm)?;
        set_inheritable(wfd.as_fd(), false, vm)?;
        Ok((rfd, wfd))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "emscripten",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn pipe2(flags: libc::c_int, vm: &VirtualMachine) -> PyResult<(OwnedFd, OwnedFd)> {
        let oflags = fcntl::OFlag::from_bits_truncate(flags);
        nix::unistd::pipe2(oflags).map_err(|err| err.into_pyexception(vm))
    }

    fn _chmod(
        path: OsPath,
        dir_fd: DirFd<'_, 0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let [] = dir_fd.0;
        let err_path = path.clone();
        let body = move || {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs_metadata(&path, follow_symlinks.0)?;
            let mut permissions = meta.permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&path, permissions)
        };
        body().map_err(|err| OSErrorBuilder::with_filename(&err, err_path, vm))
    }

    #[cfg(not(target_os = "redox"))]
    fn _fchmod(fd: BorrowedFd<'_>, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        nix::sys::stat::fchmod(
            fd,
            nix::sys::stat::Mode::from_bits(mode as libc::mode_t).unwrap(),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chmod(
        path: OsPathOrFd<'_>,
        dir_fd: DirFd<'_, 0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match path {
            OsPathOrFd::Path(path) => {
                #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd",))]
                if !follow_symlinks.0 && dir_fd == Default::default() {
                    return lchmod(path, mode, vm);
                }
                _chmod(path, dir_fd, mode, follow_symlinks, vm)
            }
            OsPathOrFd::Fd(fd) => _fchmod(fd.into(), mode, vm),
        }
    }

    #[cfg(target_os = "redox")]
    #[pyfunction]
    fn chmod(
        path: OsPath,
        dir_fd: DirFd<0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        _chmod(path, dir_fd, mode, follow_symlinks, vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchmod(fd: BorrowedFd<'_>, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        _fchmod(fd, mode, vm)
    }

    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd",))]
    #[pyfunction]
    fn lchmod(path: OsPath, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        unsafe extern "C" {
            fn lchmod(path: *const libc::c_char, mode: libc::mode_t) -> libc::c_int;
        }
        let c_path = path.clone().into_cstring(vm)?;
        if unsafe { lchmod(c_path.as_ptr(), mode as libc::mode_t) } == 0 {
            Ok(())
        } else {
            let err = std::io::Error::last_os_error();
            Err(OSErrorBuilder::with_filename(&err, path, vm))
        }
    }

    #[pyfunction]
    fn execv(
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = path.into_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            OsPath::try_from_object(vm, obj)?.into_cstring(vm)
        })?;
        let argv: Vec<&CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty"))?;
        if first.to_bytes().is_empty() {
            return Err(vm.new_value_error("execv() arg 2 first element cannot be empty"));
        }

        unistd::execv(&path, &argv)
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn execve(
        path: OsPath,
        argv: Either<PyListRef, PyTupleRef>,
        env: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = path.into_cstring(vm)?;

        let argv = vm.extract_elements_with(argv.as_ref(), |obj| {
            OsPath::try_from_object(vm, obj)?.into_cstring(vm)
        })?;
        let argv: Vec<&CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execve() arg 2 must not be empty"))?;

        if first.to_bytes().is_empty() {
            return Err(vm.new_value_error("execve() arg 2 first element cannot be empty"));
        }

        let env = env
            .into_iter()
            .map(|(k, v)| -> PyResult<_> {
                let (key, value) = (
                    OsPath::try_from_object(vm, k)?.into_bytes(),
                    OsPath::try_from_object(vm, v)?.into_bytes(),
                );

                if key.is_empty() || memchr::memchr(b'=', &key).is_some() {
                    return Err(vm.new_value_error("illegal environment variable name"));
                }

                let mut entry = key;
                entry.push(b'=');
                entry.extend_from_slice(&value);

                CString::new(entry).map_err(|err| err.into_pyexception(vm))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let env: Vec<&CStr> = env.iter().map(|entry| entry.as_c_str()).collect();

        unistd::execve(&path, &argv, &env).map_err(|err| err.into_pyexception(vm))?;
        Ok(())
    }

    #[pyfunction]
    fn getppid(vm: &VirtualMachine) -> PyObjectRef {
        let ppid = unistd::getppid().as_raw();
        vm.ctx.new_int(ppid).into()
    }

    #[pyfunction]
    fn getgid(vm: &VirtualMachine) -> PyObjectRef {
        let gid = unistd::getgid().as_raw();
        vm.ctx.new_int(gid).into()
    }

    #[pyfunction]
    fn getegid(vm: &VirtualMachine) -> PyObjectRef {
        let egid = unistd::getegid().as_raw();
        vm.ctx.new_int(egid).into()
    }

    #[pyfunction]
    fn getpgid(pid: u32, vm: &VirtualMachine) -> PyResult {
        let pgid =
            unistd::getpgid(Some(Pid::from_raw(pid as i32))).map_err(|e| e.into_pyexception(vm))?;
        Ok(vm.new_pyobj(pgid.as_raw()))
    }

    #[pyfunction]
    fn getpgrp(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(unistd::getpgrp().as_raw()).into()
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn getsid(pid: u32, vm: &VirtualMachine) -> PyResult {
        let sid =
            unistd::getsid(Some(Pid::from_raw(pid as i32))).map_err(|e| e.into_pyexception(vm))?;
        Ok(vm.new_pyobj(sid.as_raw()))
    }

    #[pyfunction]
    fn getuid(vm: &VirtualMachine) -> PyObjectRef {
        let uid = unistd::getuid().as_raw();
        vm.ctx.new_int(uid).into()
    }

    #[pyfunction]
    fn geteuid(vm: &VirtualMachine) -> PyObjectRef {
        let euid = unistd::geteuid().as_raw();
        vm.ctx.new_int(euid).into()
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android")))]
    #[pyfunction]
    fn setgid(gid: Gid, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setgid(gid).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
    #[pyfunction]
    fn setegid(egid: Gid, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setegid(egid).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setpgid(pid: u32, pgid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setpgid(Pid::from_raw(pid as i32), Pid::from_raw(pgid as i32))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setpgrp(vm: &VirtualMachine) -> PyResult<()> {
        // setpgrp() is equivalent to setpgid(0, 0)
        unistd::setpgid(Pid::from_raw(0), Pid::from_raw(0)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn setsid(vm: &VirtualMachine) -> PyResult<()> {
        unistd::setsid()
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn tcgetpgrp(fd: i32, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        use std::os::fd::BorrowedFd;
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        unistd::tcgetpgrp(fd)
            .map(|pid| pid.as_raw())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn tcsetpgrp(fd: i32, pgid: libc::pid_t, vm: &VirtualMachine) -> PyResult<()> {
        use std::os::fd::BorrowedFd;
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        unistd::tcsetpgrp(fd, Pid::from_raw(pgid)).map_err(|err| err.into_pyexception(vm))
    }

    fn try_from_id(vm: &VirtualMachine, obj: PyObjectRef, typ_name: &str) -> PyResult<u32> {
        use core::cmp::Ordering;
        let i = obj
            .try_to_ref::<PyInt>(vm)
            .map_err(|_| {
                vm.new_type_error(format!(
                    "an integer is required (got type {})",
                    obj.class().name()
                ))
            })?
            .try_to_primitive::<i64>(vm)?;

        match i.cmp(&-1) {
            Ordering::Greater => Ok(i.try_into().map_err(|_| {
                vm.new_overflow_error(format!("{typ_name} is larger than maximum"))
            })?),
            Ordering::Less => {
                Err(vm.new_overflow_error(format!("{typ_name} is less than minimum")))
            }
            // -1 means does not change the value
            // In CPython, this is `(uid_t) -1`, rustc gets mad when we try to declare
            // a negative unsigned integer :).
            Ordering::Equal => Ok(-1i32 as u32),
        }
    }

    impl TryFromObject for Uid {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            try_from_id(vm, obj, "uid").map(Self::from_raw)
        }
    }

    impl TryFromObject for Gid {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            try_from_id(vm, obj, "gid").map(Self::from_raw)
        }
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android")))]
    #[pyfunction]
    fn setuid(uid: Uid) -> nix::Result<()> {
        unistd::setuid(uid)
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
    #[pyfunction]
    fn seteuid(euid: Uid) -> nix::Result<()> {
        unistd::seteuid(euid)
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
    #[pyfunction]
    fn setreuid(ruid: Uid, euid: Uid) -> nix::Result<()> {
        let ret = unsafe { libc::setreuid(ruid.as_raw(), euid.as_raw()) };
        nix::Error::result(ret).map(drop)
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresuid(ruid: Uid, euid: Uid, suid: Uid) -> nix::Result<()> {
        unistd::setresuid(ruid, euid, suid)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn openpty(vm: &VirtualMachine) -> PyResult<(OwnedFd, OwnedFd)> {
        let r = nix::pty::openpty(None, None).map_err(|err| err.into_pyexception(vm))?;
        for fd in [&r.master, &r.slave] {
            super::set_inheritable(fd.as_fd(), false).map_err(|e| e.into_pyexception(vm))?;
        }
        Ok((r.master, r.slave))
    }

    #[pyfunction]
    fn ttyname(fd: BorrowedFd<'_>, vm: &VirtualMachine) -> PyResult {
        let name = unistd::ttyname(fd).map_err(|e| e.into_pyexception(vm))?;
        let name = name.into_os_string().into_string().unwrap();
        Ok(vm.ctx.new_str(name).into())
    }

    #[pyfunction]
    fn umask(mask: libc::mode_t) -> libc::mode_t {
        unsafe { libc::umask(mask) }
    }

    #[pyfunction]
    fn uname(vm: &VirtualMachine) -> PyResult<_os::UnameResultData> {
        let info = uname::uname().map_err(|err| err.into_pyexception(vm))?;
        Ok(_os::UnameResultData {
            sysname: info.sysname,
            nodename: info.nodename,
            release: info.release,
            version: info.version,
            machine: info.machine,
        })
    }

    #[pyfunction]
    fn sync() {
        #[cfg(not(any(target_os = "redox", target_os = "android")))]
        unsafe {
            libc::sync();
        }
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn getresuid() -> nix::Result<(u32, u32, u32)> {
        let ret = unistd::getresuid()?;
        Ok((
            ret.real.as_raw(),
            ret.effective.as_raw(),
            ret.saved.as_raw(),
        ))
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn getresgid() -> nix::Result<(u32, u32, u32)> {
        let ret = unistd::getresgid()?;
        Ok((
            ret.real.as_raw(),
            ret.effective.as_raw(),
            ret.saved.as_raw(),
        ))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresgid(rgid: Gid, egid: Gid, sgid: Gid, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setresgid(rgid, egid, sgid).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(any(target_os = "wasi", target_os = "android", target_os = "redox")))]
    #[pyfunction]
    fn setregid(rgid: Gid, egid: Gid) -> nix::Result<()> {
        let ret = unsafe { libc::setregid(rgid.as_raw(), egid.as_raw()) };
        nix::Error::result(ret).map(drop)
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn initgroups(user_name: PyStrRef, gid: Gid, vm: &VirtualMachine) -> PyResult<()> {
        let user = user_name.to_cstring(vm)?;
        unistd::initgroups(&user, gid).map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
    #[pyfunction]
    fn setgroups(
        group_ids: crate::function::ArgIterable<Gid>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let gids = group_ids.iter(vm)?.collect::<Result<Vec<_>, _>>()?;
        unistd::setgroups(&gids).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    fn envp_from_dict(
        env: crate::function::ArgMapping,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<CString>> {
        let items = env.mapping().items(vm)?;

        // Convert items to list if it isn't already
        let items = vm.ctx.new_list(
            items
                .get_iter(vm)?
                .iter(vm)?
                .collect::<PyResult<Vec<_>>>()?,
        );

        items
            .borrow_vec()
            .iter()
            .map(|item| {
                let tuple = item
                    .downcast_ref::<crate::builtins::PyTuple>()
                    .ok_or_else(|| vm.new_type_error("items() should return tuples"))?;
                let tuple_items = tuple.as_slice();
                if tuple_items.len() != 2 {
                    return Err(vm.new_value_error("items() tuples should have exactly 2 elements"));
                }
                Ok((tuple_items[0].clone(), tuple_items[1].clone()))
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_iter()
            .map(|(k, v)| {
                let k = OsPath::try_from_object(vm, k)?.into_bytes();
                let v = OsPath::try_from_object(vm, v)?.into_bytes();
                if k.contains(&0) {
                    return Err(vm.new_value_error("envp dict key cannot contain a nul byte"));
                }
                if k.contains(&b'=') {
                    return Err(vm.new_value_error("envp dict key cannot contain a '=' character"));
                }
                if v.contains(&0) {
                    return Err(vm.new_value_error("envp dict value cannot contain a nul byte"));
                }
                let mut env = k;
                env.push(b'=');
                env.extend(v);
                Ok(unsafe { CString::from_vec_unchecked(env) })
            })
            .collect()
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[derive(FromArgs)]
    pub(super) struct PosixSpawnArgs {
        #[pyarg(positional)]
        path: OsPath,
        #[pyarg(positional)]
        args: crate::function::ArgIterable<OsPath>,
        #[pyarg(positional)]
        env: Option<crate::function::ArgMapping>,
        #[pyarg(named, default)]
        file_actions: Option<crate::function::ArgIterable<PyTupleRef>>,
        #[pyarg(named, default)]
        setsigdef: Option<crate::function::ArgIterable<i32>>,
        #[pyarg(named, default)]
        setpgroup: Option<libc::pid_t>,
        #[pyarg(named, default)]
        resetids: bool,
        #[pyarg(named, default)]
        setsid: bool,
        #[pyarg(named, default)]
        setsigmask: Option<crate::function::ArgIterable<i32>>,
        #[pyarg(named, default)]
        scheduler: Option<PyTupleRef>,
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[derive(num_enum::IntoPrimitive, num_enum::TryFromPrimitive)]
    #[repr(i32)]
    enum PosixSpawnFileActionIdentifier {
        Open,
        Close,
        Dup2,
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    impl PosixSpawnArgs {
        fn spawn(self, spawnp: bool, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
            use crate::TryFromBorrowedObject;

            let path = self
                .path
                .clone()
                .into_cstring(vm)
                .map_err(|_| vm.new_value_error("path should not have nul bytes"))?;

            let mut file_actions =
                nix::spawn::PosixSpawnFileActions::init().map_err(|e| e.into_pyexception(vm))?;
            if let Some(it) = self.file_actions {
                for action in it.iter(vm)? {
                    let action = action?;
                    let (id, args) = action.split_first().ok_or_else(|| {
                        vm.new_type_error("Each file_actions element must be a non-empty tuple")
                    })?;
                    let id = i32::try_from_borrowed_object(vm, id)?;
                    let id = PosixSpawnFileActionIdentifier::try_from(id)
                        .map_err(|_| vm.new_type_error("Unknown file_actions identifier"))?;
                    let args: crate::function::FuncArgs = args.to_vec().into();
                    let ret = match id {
                        PosixSpawnFileActionIdentifier::Open => {
                            let (fd, path, oflag, mode): (_, OsPath, _, _) = args.bind(vm)?;
                            let path = CString::new(path.into_bytes()).map_err(|_| {
                                vm.new_value_error(
                                    "POSIX_SPAWN_OPEN path should not have nul bytes",
                                )
                            })?;
                            let oflag = nix::fcntl::OFlag::from_bits_retain(oflag);
                            let mode = nix::sys::stat::Mode::from_bits_retain(mode);
                            file_actions.add_open(fd, &*path, oflag, mode)
                        }
                        PosixSpawnFileActionIdentifier::Close => {
                            let (fd,) = args.bind(vm)?;
                            file_actions.add_close(fd)
                        }
                        PosixSpawnFileActionIdentifier::Dup2 => {
                            let (fd, newfd) = args.bind(vm)?;
                            file_actions.add_dup2(fd, newfd)
                        }
                    };
                    if let Err(err) = ret {
                        let err = err.into();
                        return Err(OSErrorBuilder::with_filename(&err, self.path, vm));
                    }
                }
            }

            let mut attrp =
                nix::spawn::PosixSpawnAttr::init().map_err(|e| e.into_pyexception(vm))?;
            let mut flags = nix::spawn::PosixSpawnFlags::empty();

            if let Some(sigs) = self.setsigdef {
                use nix::sys::signal;
                let mut set = signal::SigSet::empty();
                for sig in sigs.iter(vm)? {
                    let sig = sig?;
                    let sig = signal::Signal::try_from(sig).map_err(|_| {
                        vm.new_value_error(format!("signal number {sig} out of range"))
                    })?;
                    set.add(sig);
                }
                attrp
                    .set_sigdefault(&set)
                    .map_err(|e| e.into_pyexception(vm))?;
                flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF);
            }

            if let Some(pgid) = self.setpgroup {
                attrp
                    .set_pgroup(nix::unistd::Pid::from_raw(pgid))
                    .map_err(|e| e.into_pyexception(vm))?;
                flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETPGROUP);
            }

            if self.resetids {
                flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_RESETIDS);
            }

            if self.setsid {
                // Note: POSIX_SPAWN_SETSID may not be available on all platforms
                cfg_if::cfg_if! {
                    if #[cfg(any(
                        target_os = "linux",
                        target_os = "haiku",
                        target_os = "solaris",
                        target_os = "illumos",
                        target_os = "hurd",
                    ))] {
                        flags.insert(nix::spawn::PosixSpawnFlags::from_bits_retain(libc::POSIX_SPAWN_SETSID));
                    } else {
                        return Err(vm.new_not_implemented_error(
                            "setsid parameter is not supported on this platform",
                        ));
                    }
                }
            }

            if let Some(sigs) = self.setsigmask {
                use nix::sys::signal;
                let mut set = signal::SigSet::empty();
                for sig in sigs.iter(vm)? {
                    let sig = sig?;
                    let sig = signal::Signal::try_from(sig).map_err(|_| {
                        vm.new_value_error(format!("signal number {sig} out of range"))
                    })?;
                    set.add(sig);
                }
                attrp
                    .set_sigmask(&set)
                    .map_err(|e| e.into_pyexception(vm))?;
                flags.insert(nix::spawn::PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK);
            }

            if let Some(_scheduler) = self.scheduler {
                // TODO: Implement scheduler parameter handling
                // This requires platform-specific sched_param struct handling
                return Err(
                    vm.new_not_implemented_error("scheduler parameter is not yet implemented")
                );
            }

            if !flags.is_empty() {
                attrp.set_flags(flags).map_err(|e| e.into_pyexception(vm))?;
            }

            let args: Vec<CString> = self
                .args
                .iter(vm)?
                .map(|res| {
                    CString::new(res?.into_bytes())
                        .map_err(|_| vm.new_value_error("path should not have nul bytes"))
                })
                .collect::<Result<_, _>>()?;
            let env = if let Some(env_dict) = self.env {
                envp_from_dict(env_dict, vm)?
            } else {
                // env=None means use the current environment
                use rustpython_common::os::ffi::OsStringExt;
                env::vars_os()
                    .map(|(k, v)| {
                        let mut entry = k.into_vec();
                        entry.push(b'=');
                        entry.extend(v.into_vec());
                        CString::new(entry).map_err(|_| {
                            vm.new_value_error("environment string contains null byte")
                        })
                    })
                    .collect::<PyResult<Vec<_>>>()?
            };

            let ret = if spawnp {
                nix::spawn::posix_spawnp(&path, &file_actions, &attrp, &args, &env)
            } else {
                nix::spawn::posix_spawn(&*path, &file_actions, &attrp, &args, &env)
            };
            ret.map(Into::into)
                .map_err(|err| OSErrorBuilder::with_filename(&err.into(), self.path, vm))
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyfunction]
    fn posix_spawn(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        args.spawn(false, vm)
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyfunction]
    fn posix_spawnp(args: PosixSpawnArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        args.spawn(true, vm)
    }

    #[pyfunction(name = "WCOREDUMP")]
    fn wcoredump(status: i32) -> bool {
        libc::WCOREDUMP(status)
    }

    #[pyfunction(name = "WIFCONTINUED")]
    fn wifcontinued(status: i32) -> bool {
        libc::WIFCONTINUED(status)
    }

    #[pyfunction(name = "WIFSTOPPED")]
    fn wifstopped(status: i32) -> bool {
        libc::WIFSTOPPED(status)
    }

    #[pyfunction(name = "WIFSIGNALED")]
    fn wifsignaled(status: i32) -> bool {
        libc::WIFSIGNALED(status)
    }

    #[pyfunction(name = "WIFEXITED")]
    fn wifexited(status: i32) -> bool {
        libc::WIFEXITED(status)
    }

    #[pyfunction(name = "WEXITSTATUS")]
    fn wexitstatus(status: i32) -> i32 {
        libc::WEXITSTATUS(status)
    }

    #[pyfunction(name = "WSTOPSIG")]
    fn wstopsig(status: i32) -> i32 {
        libc::WSTOPSIG(status)
    }

    #[pyfunction(name = "WTERMSIG")]
    fn wtermsig(status: i32) -> i32 {
        libc::WTERMSIG(status)
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn pidfd_open(
        pid: libc::pid_t,
        flags: OptionalArg<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<OwnedFd> {
        let flags = flags.unwrap_or(0);
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) as libc::c_long };
        if fd == -1 {
            Err(vm.new_last_errno_error())
        } else {
            // Safety: syscall returns a new owned file descriptor.
            Ok(unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
        }
    }

    #[pyfunction]
    fn waitpid(pid: libc::pid_t, opt: i32, vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
        let mut status = 0;
        loop {
            let res = unsafe { libc::waitpid(pid, &mut status, opt) };
            if res == -1 {
                if nix::Error::last_raw() == libc::EINTR {
                    vm.check_signals()?;
                    continue;
                }
                return Err(nix::Error::last().into_pyexception(vm));
            }
            return Ok((res, status));
        }
    }

    #[pyfunction]
    fn wait(vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
        waitpid(-1, 0, vm)
    }

    #[pyfunction]
    fn kill(pid: i32, sig: isize, vm: &VirtualMachine) -> PyResult<()> {
        {
            let ret = unsafe { libc::kill(pid, sig as i32) };
            if ret == -1 {
                Err(vm.new_last_errno_error())
            } else {
                Ok(())
            }
        }
    }

    #[pyfunction]
    fn get_terminal_size(
        fd: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<_os::TerminalSizeData> {
        let (columns, lines) = {
            nix::ioctl_read_bad!(winsz, libc::TIOCGWINSZ, libc::winsize);
            let mut w = libc::winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe { winsz(fd.unwrap_or(libc::STDOUT_FILENO), &mut w) }
                .map_err(|err| err.into_pyexception(vm))?;
            (w.ws_col.into(), w.ws_row.into())
        };
        Ok(_os::TerminalSizeData { columns, lines })
    }

    // from libstd:
    // https://github.com/rust-lang/rust/blob/daecab3a784f28082df90cebb204998051f3557d/src/libstd/sys/unix/fs.rs#L1251
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn fcopyfile(
            in_fd: libc::c_int,
            out_fd: libc::c_int,
            state: *mut libc::c_void, // copyfile_state_t (unused)
            flags: u32,               // copyfile_flags_t
        ) -> libc::c_int;
    }

    #[cfg(target_os = "macos")]
    #[pyfunction]
    fn _fcopyfile(in_fd: i32, out_fd: i32, flags: i32, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { fcopyfile(in_fd, out_fd, core::ptr::null_mut(), flags as u32) };
        if ret < 0 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn dup(fd: BorrowedFd<'_>, vm: &VirtualMachine) -> PyResult<OwnedFd> {
        let fd = nix::unistd::dup(fd).map_err(|e| e.into_pyexception(vm))?;
        super::set_inheritable(fd.as_fd(), false)
            .map(|()| fd)
            .map_err(|e| e.into_pyexception(vm))
    }

    #[derive(FromArgs)]
    struct Dup2Args<'fd> {
        #[pyarg(positional)]
        fd: BorrowedFd<'fd>,
        #[pyarg(positional)]
        fd2: OwnedFd,
        #[pyarg(any, default = true)]
        inheritable: bool,
    }

    #[pyfunction]
    fn dup2(args: Dup2Args<'_>, vm: &VirtualMachine) -> PyResult<OwnedFd> {
        let mut fd2 = core::mem::ManuallyDrop::new(args.fd2);
        nix::unistd::dup2(args.fd, &mut fd2).map_err(|e| e.into_pyexception(vm))?;
        let fd2 = core::mem::ManuallyDrop::into_inner(fd2);
        if !args.inheritable {
            super::set_inheritable(fd2.as_fd(), false).map_err(|e| e.into_pyexception(vm))?
        }
        Ok(fd2)
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        vec![
            SupportFunc::new(
                "chmod",
                Some(false),
                Some(false),
                Some(cfg!(any(
                    target_os = "macos",
                    target_os = "freebsd",
                    target_os = "netbsd"
                ))),
            ),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("chroot", Some(false), None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("chown", Some(true), Some(true), Some(true)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("lchown", None, None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("fchown", Some(true), None, Some(true)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("mknod", Some(true), Some(MKNOD_DIR_FD), Some(false)),
            SupportFunc::new("umask", Some(false), Some(false), Some(false)),
            SupportFunc::new("execv", None, None, None),
            SupportFunc::new("pathconf", Some(true), None, None),
            SupportFunc::new("fpathconf", Some(true), None, None),
            SupportFunc::new("fchdir", Some(true), None, None),
        ]
    }

    #[pyfunction]
    fn getlogin(vm: &VirtualMachine) -> PyResult<String> {
        // Get a pointer to the login name string. The string is statically
        // allocated and might be overwritten on subsequent calls to this
        // function or to `cuserid()`. See man getlogin(3) for more information.
        let ptr = unsafe { libc::getlogin() };
        if ptr.is_null() {
            return Err(vm.new_os_error("unable to determine login name"));
        }
        let slice = unsafe { CStr::from_ptr(ptr) };
        slice
            .to_str()
            .map(|s| s.to_owned())
            .map_err(|e| vm.new_unicode_decode_error(format!("unable to decode login name: {e}")))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn getgrouplist(user: PyStrRef, group: u32, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let user = CString::new(user.as_str()).unwrap();
        let gid = Gid::from_raw(group);
        let group_ids = unistd::getgrouplist(&user, gid).map_err(|err| err.into_pyexception(vm))?;
        Ok(group_ids
            .into_iter()
            .map(|gid| vm.new_pyobj(gid.as_raw()))
            .collect())
    }

    #[cfg(not(target_os = "redox"))]
    cfg_if::cfg_if! {
        if #[cfg(all(target_os = "linux", target_env = "gnu"))] {
            type PriorityWhichType = libc::__priority_which_t;
        } else {
            type PriorityWhichType = libc::c_int;
        }
    }
    #[cfg(not(target_os = "redox"))]
    cfg_if::cfg_if! {
        if #[cfg(target_os = "freebsd")] {
            type PriorityWhoType = i32;
        } else {
            type PriorityWhoType = u32;
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn getpriority(
        which: PriorityWhichType,
        who: PriorityWhoType,
        vm: &VirtualMachine,
    ) -> PyResult {
        use nix::errno::Errno;
        Errno::clear();
        let retval = unsafe { libc::getpriority(which, who) };
        if Errno::last_raw() != 0 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(vm.ctx.new_int(retval).into())
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setpriority(
        which: PriorityWhichType,
        who: PriorityWhoType,
        priority: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let retval = unsafe { libc::setpriority(which, who, priority) };
        if retval == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(())
        }
    }

    struct PathconfName(i32);

    impl TryFromObject for PathconfName {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let i = match obj.downcast::<PyInt>() {
                Ok(int) => int.try_to_primitive(vm)?,
                Err(obj) => {
                    let s = obj.downcast::<PyStr>().map_err(|_| {
                        vm.new_type_error(
                            "configuration names must be strings or integers".to_owned(),
                        )
                    })?;
                    s.as_str()
                        .parse::<PathconfVar>()
                        .map_err(|_| vm.new_value_error("unrecognized configuration name"))?
                        as i32
                }
            };
            Ok(Self(i))
        }
    }

    // Copy from [nix::unistd::PathconfVar](https://docs.rs/nix/0.21.0/nix/unistd/enum.PathconfVar.html)
    // Change enum name to fit python doc
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, EnumIter, EnumString)]
    #[repr(i32)]
    #[allow(non_camel_case_types)]
    pub enum PathconfVar {
        #[cfg(any(
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox"
        ))]
        /// Minimum number of bits needed to represent, as a signed integer value,
        /// the maximum size of a regular file allowed in the specified directory.
        PC_FILESIZEBITS = libc::_PC_FILESIZEBITS,
        /// Maximum number of links to a single file.
        PC_LINK_MAX = libc::_PC_LINK_MAX,
        /// Maximum number of bytes in a terminal canonical input line.
        PC_MAX_CANON = libc::_PC_MAX_CANON,
        /// Minimum number of bytes for which space is available in a terminal input
        /// queue; therefore, the maximum number of bytes a conforming application
        /// may require to be typed as input before reading them.
        PC_MAX_INPUT = libc::_PC_MAX_INPUT,
        /// Maximum number of bytes in a filename (not including the terminating
        /// null of a filename string).
        PC_NAME_MAX = libc::_PC_NAME_MAX,
        /// Maximum number of bytes the implementation will store as a pathname in a
        /// user-supplied buffer of unspecified size, including the terminating null
        /// character. Minimum number the implementation will accept as the maximum
        /// number of bytes in a pathname.
        PC_PATH_MAX = libc::_PC_PATH_MAX,
        /// Maximum number of bytes that is guaranteed to be atomic when writing to
        /// a pipe.
        PC_PIPE_BUF = libc::_PC_PIPE_BUF,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "solaris"
        ))]
        /// Symbolic links can be created.
        PC_2_SYMLINKS = libc::_PC_2_SYMLINKS,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        ))]
        /// Minimum number of bytes of storage actually allocated for any portion of
        /// a file.
        PC_ALLOC_SIZE_MIN = libc::_PC_ALLOC_SIZE_MIN,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "openbsd"
        ))]
        /// Recommended increment for file transfer sizes between the
        /// `POSIX_REC_MIN_XFER_SIZE` and `POSIX_REC_MAX_XFER_SIZE` values.
        PC_REC_INCR_XFER_SIZE = libc::_PC_REC_INCR_XFER_SIZE,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        ))]
        /// Maximum recommended file transfer size.
        PC_REC_MAX_XFER_SIZE = libc::_PC_REC_MAX_XFER_SIZE,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        ))]
        /// Minimum recommended file transfer size.
        PC_REC_MIN_XFER_SIZE = libc::_PC_REC_MIN_XFER_SIZE,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox"
        ))]
        ///  Recommended file transfer buffer alignment.
        PC_REC_XFER_ALIGN = libc::_PC_REC_XFER_ALIGN,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "solaris"
        ))]
        /// Maximum number of bytes in a symbolic link.
        PC_SYMLINK_MAX = libc::_PC_SYMLINK_MAX,
        /// The use of `chown` and `fchown` is restricted to a process with
        /// appropriate privileges, and to changing the group ID of a file only to
        /// the effective group ID of the process or to one of its supplementary
        /// group IDs.
        PC_CHOWN_RESTRICTED = libc::_PC_CHOWN_RESTRICTED,
        /// Pathname components longer than {NAME_MAX} generate an error.
        PC_NO_TRUNC = libc::_PC_NO_TRUNC,
        /// This symbol shall be defined to be the value of a character that shall
        /// disable terminal special character handling.
        PC_VDISABLE = libc::_PC_VDISABLE,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "solaris"
        ))]
        /// Asynchronous input or output operations may be performed for the
        /// associated file.
        PC_ASYNC_IO = libc::_PC_ASYNC_IO,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "solaris"
        ))]
        /// Prioritized input or output operations may be performed for the
        /// associated file.
        PC_PRIO_IO = libc::_PC_PRIO_IO,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "solaris"
        ))]
        /// Synchronized input or output operations may be performed for the
        /// associated file.
        PC_SYNC_IO = libc::_PC_SYNC_IO,
        #[cfg(any(target_os = "dragonfly", target_os = "openbsd"))]
        /// The resolution in nanoseconds for all file timestamps.
        PC_TIMESTAMP_RESOLUTION = libc::_PC_TIMESTAMP_RESOLUTION,
    }

    #[cfg(unix)]
    #[pyfunction]
    fn pathconf(
        path: OsPathOrFd<'_>,
        PathconfName(name): PathconfName,
        vm: &VirtualMachine,
    ) -> PyResult<Option<libc::c_long>> {
        use nix::errno::Errno;

        Errno::clear();
        debug_assert_eq!(Errno::last_raw(), 0);
        let raw = match &path {
            OsPathOrFd::Path(path) => {
                let path = path.clone().into_cstring(vm)?;
                unsafe { libc::pathconf(path.as_ptr(), name) }
            }
            OsPathOrFd::Fd(fd) => unsafe { libc::fpathconf(fd.as_raw(), name) },
        };

        if raw == -1 {
            if Errno::last_raw() == 0 {
                Ok(None)
            } else {
                Err(OSErrorBuilder::with_filename(
                    &io::Error::from(Errno::last()),
                    path,
                    vm,
                ))
            }
        } else {
            Ok(Some(raw))
        }
    }

    #[pyfunction]
    fn fpathconf(
        fd: BorrowedFd<'_>,
        name: PathconfName,
        vm: &VirtualMachine,
    ) -> PyResult<Option<libc::c_long>> {
        pathconf(OsPathOrFd::Fd(fd.into()), name, vm)
    }

    #[pyattr]
    fn pathconf_names(vm: &VirtualMachine) -> PyDictRef {
        use strum::IntoEnumIterator;
        let pathname = vm.ctx.new_dict();
        for variant in PathconfVar::iter() {
            // get the name of variant as a string to use as the dictionary key
            let key = vm.ctx.new_str(format!("{variant:?}"));
            // get the enum from the string and convert it to an integer for the dictionary value
            let value = vm.ctx.new_int(variant as u8);
            pathname
                .set_item(&*key, value.into(), vm)
                .expect("dict set_item unexpectedly failed");
        }
        pathname
    }

    #[cfg(not(target_os = "redox"))]
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, EnumIter, EnumString)]
    #[repr(i32)]
    #[allow(non_camel_case_types)]
    pub enum SysconfVar {
        SC_2_CHAR_TERM = libc::_SC_2_CHAR_TERM,
        SC_2_C_BIND = libc::_SC_2_C_BIND,
        SC_2_C_DEV = libc::_SC_2_C_DEV,
        SC_2_FORT_DEV = libc::_SC_2_FORT_DEV,
        SC_2_FORT_RUN = libc::_SC_2_FORT_RUN,
        SC_2_LOCALEDEF = libc::_SC_2_LOCALEDEF,
        SC_2_SW_DEV = libc::_SC_2_SW_DEV,
        SC_2_UPE = libc::_SC_2_UPE,
        SC_2_VERSION = libc::_SC_2_VERSION,
        SC_AIO_LISTIO_MAX = libc::_SC_AIO_LISTIO_MAX,
        SC_AIO_MAX = libc::_SC_AIO_MAX,
        SC_AIO_PRIO_DELTA_MAX = libc::_SC_AIO_PRIO_DELTA_MAX,
        SC_ARG_MAX = libc::_SC_ARG_MAX,
        SC_ASYNCHRONOUS_IO = libc::_SC_ASYNCHRONOUS_IO,
        SC_ATEXIT_MAX = libc::_SC_ATEXIT_MAX,
        SC_BC_BASE_MAX = libc::_SC_BC_BASE_MAX,
        SC_BC_DIM_MAX = libc::_SC_BC_DIM_MAX,
        SC_BC_SCALE_MAX = libc::_SC_BC_SCALE_MAX,
        SC_BC_STRING_MAX = libc::_SC_BC_STRING_MAX,
        SC_CHILD_MAX = libc::_SC_CHILD_MAX,
        SC_CLK_TCK = libc::_SC_CLK_TCK,
        SC_COLL_WEIGHTS_MAX = libc::_SC_COLL_WEIGHTS_MAX,
        SC_DELAYTIMER_MAX = libc::_SC_DELAYTIMER_MAX,
        SC_EXPR_NEST_MAX = libc::_SC_EXPR_NEST_MAX,
        SC_FSYNC = libc::_SC_FSYNC,
        SC_GETGR_R_SIZE_MAX = libc::_SC_GETGR_R_SIZE_MAX,
        SC_GETPW_R_SIZE_MAX = libc::_SC_GETPW_R_SIZE_MAX,
        SC_IOV_MAX = libc::_SC_IOV_MAX,
        SC_JOB_CONTROL = libc::_SC_JOB_CONTROL,
        SC_LINE_MAX = libc::_SC_LINE_MAX,
        SC_LOGIN_NAME_MAX = libc::_SC_LOGIN_NAME_MAX,
        SC_MAPPED_FILES = libc::_SC_MAPPED_FILES,
        SC_MEMLOCK = libc::_SC_MEMLOCK,
        SC_MEMLOCK_RANGE = libc::_SC_MEMLOCK_RANGE,
        SC_MEMORY_PROTECTION = libc::_SC_MEMORY_PROTECTION,
        SC_MESSAGE_PASSING = libc::_SC_MESSAGE_PASSING,
        SC_MQ_OPEN_MAX = libc::_SC_MQ_OPEN_MAX,
        SC_MQ_PRIO_MAX = libc::_SC_MQ_PRIO_MAX,
        SC_NGROUPS_MAX = libc::_SC_NGROUPS_MAX,
        SC_NPROCESSORS_CONF = libc::_SC_NPROCESSORS_CONF,
        SC_NPROCESSORS_ONLN = libc::_SC_NPROCESSORS_ONLN,
        SC_OPEN_MAX = libc::_SC_OPEN_MAX,
        SC_PAGE_SIZE = libc::_SC_PAGE_SIZE,
        #[cfg(any(
            target_os = "linux",
            target_vendor = "apple",
            target_os = "netbsd",
            target_os = "fuchsia"
        ))]
        SC_PASS_MAX = libc::_SC_PASS_MAX,
        SC_PHYS_PAGES = libc::_SC_PHYS_PAGES,
        SC_PRIORITIZED_IO = libc::_SC_PRIORITIZED_IO,
        SC_PRIORITY_SCHEDULING = libc::_SC_PRIORITY_SCHEDULING,
        SC_REALTIME_SIGNALS = libc::_SC_REALTIME_SIGNALS,
        SC_RE_DUP_MAX = libc::_SC_RE_DUP_MAX,
        SC_RTSIG_MAX = libc::_SC_RTSIG_MAX,
        SC_SAVED_IDS = libc::_SC_SAVED_IDS,
        SC_SEMAPHORES = libc::_SC_SEMAPHORES,
        SC_SEM_NSEMS_MAX = libc::_SC_SEM_NSEMS_MAX,
        SC_SEM_VALUE_MAX = libc::_SC_SEM_VALUE_MAX,
        SC_SHARED_MEMORY_OBJECTS = libc::_SC_SHARED_MEMORY_OBJECTS,
        SC_SIGQUEUE_MAX = libc::_SC_SIGQUEUE_MAX,
        SC_STREAM_MAX = libc::_SC_STREAM_MAX,
        SC_SYNCHRONIZED_IO = libc::_SC_SYNCHRONIZED_IO,
        SC_THREADS = libc::_SC_THREADS,
        SC_THREAD_ATTR_STACKADDR = libc::_SC_THREAD_ATTR_STACKADDR,
        SC_THREAD_ATTR_STACKSIZE = libc::_SC_THREAD_ATTR_STACKSIZE,
        SC_THREAD_DESTRUCTOR_ITERATIONS = libc::_SC_THREAD_DESTRUCTOR_ITERATIONS,
        SC_THREAD_KEYS_MAX = libc::_SC_THREAD_KEYS_MAX,
        SC_THREAD_PRIORITY_SCHEDULING = libc::_SC_THREAD_PRIORITY_SCHEDULING,
        SC_THREAD_PRIO_INHERIT = libc::_SC_THREAD_PRIO_INHERIT,
        SC_THREAD_PRIO_PROTECT = libc::_SC_THREAD_PRIO_PROTECT,
        SC_THREAD_PROCESS_SHARED = libc::_SC_THREAD_PROCESS_SHARED,
        SC_THREAD_SAFE_FUNCTIONS = libc::_SC_THREAD_SAFE_FUNCTIONS,
        SC_THREAD_STACK_MIN = libc::_SC_THREAD_STACK_MIN,
        SC_THREAD_THREADS_MAX = libc::_SC_THREAD_THREADS_MAX,
        SC_TIMERS = libc::_SC_TIMERS,
        SC_TIMER_MAX = libc::_SC_TIMER_MAX,
        SC_TTY_NAME_MAX = libc::_SC_TTY_NAME_MAX,
        SC_TZNAME_MAX = libc::_SC_TZNAME_MAX,
        SC_VERSION = libc::_SC_VERSION,
        SC_XOPEN_CRYPT = libc::_SC_XOPEN_CRYPT,
        SC_XOPEN_ENH_I18N = libc::_SC_XOPEN_ENH_I18N,
        SC_XOPEN_LEGACY = libc::_SC_XOPEN_LEGACY,
        SC_XOPEN_REALTIME = libc::_SC_XOPEN_REALTIME,
        SC_XOPEN_REALTIME_THREADS = libc::_SC_XOPEN_REALTIME_THREADS,
        SC_XOPEN_SHM = libc::_SC_XOPEN_SHM,
        SC_XOPEN_UNIX = libc::_SC_XOPEN_UNIX,
        SC_XOPEN_VERSION = libc::_SC_XOPEN_VERSION,
        SC_XOPEN_XCU_VERSION = libc::_SC_XOPEN_XCU_VERSION,
        #[cfg(any(
            target_os = "linux",
            target_vendor = "apple",
            target_os = "netbsd",
            target_os = "fuchsia"
        ))]
        SC_XBS5_ILP32_OFF32 = libc::_SC_XBS5_ILP32_OFF32,
        #[cfg(any(
            target_os = "linux",
            target_vendor = "apple",
            target_os = "netbsd",
            target_os = "fuchsia"
        ))]
        SC_XBS5_ILP32_OFFBIG = libc::_SC_XBS5_ILP32_OFFBIG,
        #[cfg(any(
            target_os = "linux",
            target_vendor = "apple",
            target_os = "netbsd",
            target_os = "fuchsia"
        ))]
        SC_XBS5_LP64_OFF64 = libc::_SC_XBS5_LP64_OFF64,
        #[cfg(any(
            target_os = "linux",
            target_vendor = "apple",
            target_os = "netbsd",
            target_os = "fuchsia"
        ))]
        SC_XBS5_LPBIG_OFFBIG = libc::_SC_XBS5_LPBIG_OFFBIG,
    }

    #[cfg(target_os = "redox")]
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, EnumIter, EnumString)]
    #[repr(i32)]
    #[allow(non_camel_case_types)]
    pub enum SysconfVar {
        SC_ARG_MAX = libc::_SC_ARG_MAX,
        SC_CHILD_MAX = libc::_SC_CHILD_MAX,
        SC_CLK_TCK = libc::_SC_CLK_TCK,
        SC_NGROUPS_MAX = libc::_SC_NGROUPS_MAX,
        SC_OPEN_MAX = libc::_SC_OPEN_MAX,
        SC_STREAM_MAX = libc::_SC_STREAM_MAX,
        SC_TZNAME_MAX = libc::_SC_TZNAME_MAX,
        SC_VERSION = libc::_SC_VERSION,
        SC_PAGE_SIZE = libc::_SC_PAGE_SIZE,
        SC_RE_DUP_MAX = libc::_SC_RE_DUP_MAX,
        SC_LOGIN_NAME_MAX = libc::_SC_LOGIN_NAME_MAX,
        SC_TTY_NAME_MAX = libc::_SC_TTY_NAME_MAX,
        SC_SYMLOOP_MAX = libc::_SC_SYMLOOP_MAX,
        SC_HOST_NAME_MAX = libc::_SC_HOST_NAME_MAX,
    }

    impl SysconfVar {
        pub const SC_PAGESIZE: Self = Self::SC_PAGE_SIZE;
    }

    struct SysconfName(i32);

    impl TryFromObject for SysconfName {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let i = match obj.downcast::<PyInt>() {
                Ok(int) => int.try_to_primitive(vm)?,
                Err(obj) => {
                    let s = obj.downcast::<PyStr>().map_err(|_| {
                        vm.new_type_error(
                            "configuration names must be strings or integers".to_owned(),
                        )
                    })?;
                    s.as_str().parse::<SysconfVar>().or_else(|_| {
                        if s.as_str() == "SC_PAGESIZE" {
                            Ok(SysconfVar::SC_PAGESIZE)
                        } else {
                            Err(vm.new_value_error("unrecognized configuration name"))
                        }
                    })? as i32
                }
            };
            Ok(Self(i))
        }
    }

    #[pyfunction]
    fn sysconf(name: SysconfName, vm: &VirtualMachine) -> PyResult<libc::c_long> {
        crate::common::os::set_errno(0);
        let r = unsafe { libc::sysconf(name.0) };
        if r == -1 && crate::common::os::get_errno() != 0 {
            return Err(vm.new_last_errno_error());
        }
        Ok(r)
    }

    #[pyattr]
    fn sysconf_names(vm: &VirtualMachine) -> PyDictRef {
        use strum::IntoEnumIterator;
        let names = vm.ctx.new_dict();
        for variant in SysconfVar::iter() {
            // get the name of variant as a string to use as the dictionary key
            let key = vm.ctx.new_str(format!("{variant:?}"));
            // get the enum from the string and convert it to an integer for the dictionary value
            let value = vm.ctx.new_int(variant as u8);
            names
                .set_item(&*key, value.into(), vm)
                .expect("dict set_item unexpectedly failed");
        }
        names
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[derive(FromArgs)]
    struct SendFileArgs<'fd> {
        out_fd: BorrowedFd<'fd>,
        in_fd: BorrowedFd<'fd>,
        offset: crate::common::crt_fd::Offset,
        count: i64,
        #[cfg(target_os = "macos")]
        #[pyarg(any, optional)]
        headers: OptionalArg<PyObjectRef>,
        #[cfg(target_os = "macos")]
        #[pyarg(any, optional)]
        trailers: OptionalArg<PyObjectRef>,
        #[cfg(target_os = "macos")]
        #[allow(dead_code)]
        #[pyarg(any, default)]
        // TODO: not implemented
        flags: OptionalArg<i32>,
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn sendfile(args: SendFileArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let mut file_offset = args.offset;

        let res = nix::sys::sendfile::sendfile(
            args.out_fd,
            args.in_fd,
            Some(&mut file_offset),
            args.count as usize,
        )
        .map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_int(res as u64).into())
    }

    #[cfg(target_os = "macos")]
    fn _extract_vec_bytes(
        x: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<crate::function::ArgBytesLike>>> {
        x.into_option()
            .map(|x| {
                let v: Vec<crate::function::ArgBytesLike> = x.try_to_value(vm)?;
                Ok(if v.is_empty() { None } else { Some(v) })
            })
            .transpose()
            .map(Option::flatten)
    }

    #[cfg(target_os = "macos")]
    #[pyfunction]
    fn sendfile(args: SendFileArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let headers = _extract_vec_bytes(args.headers, vm)?;
        let count = headers
            .as_ref()
            .map(|v| v.iter().map(|s| s.len()).sum())
            .unwrap_or(0) as i64
            + args.count;

        let headers = headers
            .as_ref()
            .map(|v| v.iter().map(|b| b.borrow_buf()).collect::<Vec<_>>());
        let headers = headers
            .as_ref()
            .map(|v| v.iter().map(|borrowed| &**borrowed).collect::<Vec<_>>());
        let headers = headers.as_deref();

        let trailers = _extract_vec_bytes(args.trailers, vm)?;
        let trailers = trailers
            .as_ref()
            .map(|v| v.iter().map(|b| b.borrow_buf()).collect::<Vec<_>>());
        let trailers = trailers
            .as_ref()
            .map(|v| v.iter().map(|borrowed| &**borrowed).collect::<Vec<_>>());
        let trailers = trailers.as_deref();

        let (res, written) = nix::sys::sendfile::sendfile(
            args.in_fd,
            args.out_fd,
            args.offset,
            Some(count),
            headers,
            trailers,
        );
        // On macOS, sendfile can return EAGAIN even when some bytes were written.
        // In that case, we should return the number of bytes written rather than
        // raising an exception. Only raise an error if no bytes were written.
        if let Err(err) = res
            && written == 0
        {
            return Err(err.into_pyexception(vm));
        }
        Ok(vm.ctx.new_int(written as u64).into())
    }

    #[cfg(target_os = "linux")]
    unsafe fn sys_getrandom(buf: *mut libc::c_void, buflen: usize, flags: u32) -> isize {
        unsafe { libc::syscall(libc::SYS_getrandom, buf, buflen, flags as usize) as _ }
    }

    #[cfg(target_os = "linux")]
    #[pyfunction]
    fn getrandom(size: isize, flags: OptionalArg<u32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let size = usize::try_from(size)
            .map_err(|_| vm.new_os_error(format!("Invalid argument for size: {size}")))?;
        let mut buf = Vec::with_capacity(size);
        unsafe {
            let len = sys_getrandom(
                buf.as_mut_ptr() as *mut libc::c_void,
                size,
                flags.unwrap_or(0),
            )
            .try_into()
            .map_err(|_| vm.new_last_os_error())?;
            buf.set_len(len);
        }
        Ok(buf)
    }

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::os::module_exec(vm, module)?;
        Ok(())
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "netbsd",
    target_os = "freebsd",
    target_os = "android"
))]
#[pymodule(sub)]
mod posix_sched {
    use crate::{
        AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyInt, PyTupleRef},
        convert::ToPyObject,
        function::FuncArgs,
        types::PyStructSequence,
    };

    #[derive(FromArgs)]
    struct SchedParamArgs {
        #[pyarg(any)]
        sched_priority: PyObjectRef,
    }

    #[pystruct_sequence_data]
    struct SchedParamData {
        pub sched_priority: PyObjectRef,
    }

    #[pyattr]
    #[pystruct_sequence(name = "sched_param", module = "posix", data = "SchedParamData")]
    struct PySchedParam;

    #[pyclass(with(PyStructSequence))]
    impl PySchedParam {
        #[pyslot]
        fn slot_new(
            cls: crate::builtins::PyTypeRef,
            args: FuncArgs,
            vm: &VirtualMachine,
        ) -> PyResult {
            use crate::PyPayload;
            let SchedParamArgs { sched_priority } = args.bind(vm)?;
            let items = vec![sched_priority];
            crate::builtins::PyTuple::new_unchecked(items.into_boxed_slice())
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }

        #[extend_class]
        fn extend_pyclass(ctx: &crate::vm::Context, class: &'static Py<crate::builtins::PyType>) {
            // Override __reduce__ to return (type, (sched_priority,))
            // instead of the generic structseq (type, ((sched_priority,),)).
            // The trait's extend_class checks contains_key before setting default.
            const SCHED_PARAM_REDUCE: crate::function::PyMethodDef =
                crate::function::PyMethodDef::new_const(
                    "__reduce__",
                    |zelf: crate::PyRef<crate::builtins::PyTuple>,
                     vm: &VirtualMachine|
                     -> PyTupleRef {
                        vm.new_tuple((zelf.class().to_owned(), (zelf[0].clone(),)))
                    },
                    crate::function::PyMethodFlags::METHOD,
                    None,
                );
            class.set_attr(
                ctx.intern_str("__reduce__"),
                SCHED_PARAM_REDUCE.to_proper_method(class, ctx),
            );
        }
    }

    #[cfg(not(target_env = "musl"))]
    fn convert_sched_param(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<libc::sched_param> {
        use crate::{builtins::PyTuple, class::StaticType};
        if !obj.fast_isinstance(PySchedParam::static_type()) {
            return Err(vm.new_type_error("must have a sched_param object".to_owned()));
        }
        let tuple = obj.downcast_ref::<PyTuple>().unwrap();
        let priority = tuple[0].clone();
        let priority_type = priority.class().name().to_string();
        let value = priority.downcast::<PyInt>().map_err(|_| {
            vm.new_type_error(format!("an integer is required (got type {priority_type})"))
        })?;
        let sched_priority = value.try_to_primitive(vm)?;
        Ok(libc::sched_param { sched_priority })
    }

    #[pyfunction]
    fn sched_getscheduler(pid: libc::pid_t, vm: &VirtualMachine) -> PyResult<i32> {
        let policy = unsafe { libc::sched_getscheduler(pid) };
        if policy == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(policy)
        }
    }

    #[derive(FromArgs)]
    struct SchedSetschedulerArgs {
        #[pyarg(positional)]
        pid: i32,
        #[pyarg(positional)]
        policy: i32,
        #[pyarg(positional)]
        sched_param: PyObjectRef,
    }

    #[cfg(not(target_env = "musl"))]
    #[pyfunction]
    fn sched_setscheduler(args: SchedSetschedulerArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let libc_sched_param = convert_sched_param(&args.sched_param, vm)?;
        let policy = unsafe { libc::sched_setscheduler(args.pid, args.policy, &libc_sched_param) };
        if policy == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(policy)
        }
    }

    #[pyfunction]
    fn sched_getparam(pid: libc::pid_t, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let param = unsafe {
            let mut param = core::mem::MaybeUninit::uninit();
            if -1 == libc::sched_getparam(pid, param.as_mut_ptr()) {
                return Err(vm.new_last_errno_error());
            }
            param.assume_init()
        };
        Ok(PySchedParam::from_data(
            SchedParamData {
                sched_priority: param.sched_priority.to_pyobject(vm),
            },
            vm,
        ))
    }

    #[derive(FromArgs)]
    struct SchedSetParamArgs {
        #[pyarg(positional)]
        pid: i32,
        #[pyarg(positional)]
        sched_param: PyObjectRef,
    }

    #[cfg(not(target_env = "musl"))]
    #[pyfunction]
    fn sched_setparam(args: SchedSetParamArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let libc_sched_param = convert_sched_param(&args.sched_param, vm)?;
        let ret = unsafe { libc::sched_setparam(args.pid, &libc_sched_param) };
        if ret == -1 {
            Err(vm.new_last_errno_error())
        } else {
            Ok(ret)
        }
    }
}

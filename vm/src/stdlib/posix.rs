use crate::{PyObjectRef, PyResult, VirtualMachine};
use nix;
use std::os::unix::io::RawFd;

pub fn raw_set_inheritable(fd: RawFd, inheritable: bool) -> nix::Result<()> {
    use nix::fcntl;
    let flags = fcntl::FdFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD)?);
    let mut new_flags = flags;
    new_flags.set(fcntl::FdFlag::FD_CLOEXEC, !inheritable);
    if flags != new_flags {
        fcntl::fcntl(fd, fcntl::FcntlArg::F_SETFD(new_flags))?;
    }
    Ok(())
}

pub(super) fn bytes_as_osstr<'a>(
    b: &'a [u8],
    _vm: &VirtualMachine,
) -> PyResult<&'a std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::OsStr::from_bytes(b))
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = module::make_module(vm);
    super::os::extend_module(vm, &module);
    module
}

#[pymodule(name = "posix")]
pub mod module {
    use crate::{
        builtins::{PyDictRef, PyInt, PyListRef, PyStrRef, PyTupleRef, PyTypeRef},
        function::{IntoPyException, IntoPyObject, OptionalArg},
        slots::SlotConstructor,
        stdlib::os::{
            errno_err, DirFd, FollowSymlinks, PathOrFd, PyPathLike, SupportFunc, TargetIsDirectory,
            _os, fs_metadata, IOErrorBuilder,
        },
        utils::{Either, ToCString},
        ItemProtocol, PyObjectRef, PyResult, PyValue, TryFromObject, VirtualMachine,
    };
    use bitflags::bitflags;
    use nix::fcntl;
    use nix::unistd::{self, Gid, Pid, Uid};
    #[allow(unused_imports)] // TODO: use will be unnecessary in edition 2021
    use std::convert::TryFrom;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi as ffi_ext;
    use std::os::unix::io::RawFd;
    use std::{env, fs, io};
    use strum_macros::EnumString;

    #[pyattr]
    use libc::{PRIO_PGRP, PRIO_PROCESS, PRIO_USER};

    #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "linux"))]
    #[pyattr]
    use libc::{SEEK_DATA, SEEK_HOLE};

    #[cfg(not(any(target_os = "redox", target_os = "freebsd")))]
    #[pyattr]
    use libc::O_DSYNC;
    #[pyattr]
    use libc::{O_CLOEXEC, O_NONBLOCK, WNOHANG};
    #[cfg(not(target_os = "redox"))]
    #[pyattr]
    use libc::{O_NDELAY, O_NOCTTY};

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

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd"
    ))]
    #[pyattr]
    const SCHED_RR: i32 = libc::SCHED_RR;
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd"
    ))]
    #[pyattr]
    const SCHED_FIFO: i32 = libc::SCHED_FIFO;
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd"
    ))]
    #[pyattr]
    const SCHED_OTHER: i32 = libc::SCHED_OTHER;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[pyattr]
    const SCHED_IDLE: i32 = libc::SCHED_IDLE;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[pyattr]
    const SCHED_BATCH: i32 = libc::SCHED_BATCH;

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_OPEN: i32 = PosixSpawnFileActionIdentifier::Open as i32;
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_CLOSE: i32 = PosixSpawnFileActionIdentifier::Close as i32;
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    const POSIX_SPAWN_DUP2: i32 = PosixSpawnFileActionIdentifier::Dup2 as i32;

    #[cfg(target_os = "macos")]
    #[pyattr]
    const _COPYFILE_DATA: u32 = 1 << 3;

    // Flags for os_access
    bitflags! {
        pub struct AccessFlags: u8{
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

    fn get_permissions(mode: u32) -> Permissions {
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
        use libc::{c_int, gid_t};
        use nix::errno::Errno;
        use std::ptr;
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
    fn getgroups(vm: &VirtualMachine) -> PyResult {
        let group_ids = getgroups_impl().map_err(|e| e.into_pyexception(vm))?;
        Ok(vm.ctx.new_list(
            group_ids
                .into_iter()
                .map(|gid| vm.ctx.new_int(gid.as_raw()))
                .collect(),
        ))
    }

    #[pyfunction]
    pub(super) fn access(path: PyPathLike, mode: u8, vm: &VirtualMachine) -> PyResult<bool> {
        use std::os::unix::fs::MetadataExt;

        let flags = AccessFlags::from_bits(mode).ok_or_else(|| {
            vm.new_value_error(
            "One of the flags is wrong, there are only 4 possibilities F_OK, R_OK, W_OK and X_OK"
                .to_owned(),
        )
        })?;

        let metadata = fs::metadata(&path.path);

        // if it's only checking for F_OK
        if flags == AccessFlags::F_OK {
            return Ok(metadata.is_ok());
        }

        let metadata = metadata.map_err(|err| err.into_pyexception(vm))?;

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
        use ffi_ext::OsStringExt;

        let environ = vm.ctx.new_dict();
        for (key, value) in env::vars_os() {
            environ
                .set_item(
                    vm.ctx.new_bytes(key.into_vec()),
                    vm.ctx.new_bytes(value.into_vec()),
                    vm,
                )
                .unwrap();
        }

        environ
    }

    #[derive(FromArgs)]
    pub(super) struct SimlinkArgs {
        src: PyPathLike,
        dst: PyPathLike,
        #[pyarg(flatten)]
        _target_is_directory: TargetIsDirectory,
        #[pyarg(flatten)]
        dir_fd: DirFd<{ _os::SYMLINK_DIR_FD as usize }>,
    }

    #[pyfunction]
    pub(super) fn symlink(args: SimlinkArgs, vm: &VirtualMachine) -> PyResult<()> {
        let src = args.src.into_cstring(vm)?;
        let dst = args.dst.into_cstring(vm)?;
        #[cfg(not(target_os = "redox"))]
        {
            nix::unistd::symlinkat(&*src, args.dir_fd.get_opt(), &*dst)
                .map_err(|err| err.into_pyexception(vm))
        }
        #[cfg(target_os = "redox")]
        {
            let [] = args.dir_fd.0;
            let res = unsafe { libc::symlink(src.as_ptr(), dst.as_ptr()) };
            if res < 0 {
                Err(errno_err(vm))
            } else {
                Ok(())
            }
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchdir(fd: RawFd, vm: &VirtualMachine) -> PyResult<()> {
        nix::unistd::fchdir(fd).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chroot(path: PyPathLike, vm: &VirtualMachine) -> PyResult<()> {
        use crate::stdlib::os::IOErrorBuilder;

        nix::unistd::chroot(&*path.path).map_err(|err| {
            // Use `From<nix::Error> for io::Error` when it is available
            IOErrorBuilder::new(io::Error::from_raw_os_error(err as i32))
                .filename(path)
                .into_pyexception(vm)
        })
    }

    // As of now, redox does not seems to support chown command (cf. https://gitlab.redox-os.org/redox-os/coreutils , last checked on 05/07/2020)
    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chown(
        path: PathOrFd,
        uid: isize,
        gid: isize,
        dir_fd: DirFd<1>,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let uid = if uid >= 0 {
            Some(nix::unistd::Uid::from_raw(uid as u32))
        } else if uid == -1 {
            None
        } else {
            return Err(vm.new_os_error(String::from("Specified uid is not valid.")));
        };

        let gid = if gid >= 0 {
            Some(nix::unistd::Gid::from_raw(gid as u32))
        } else if gid == -1 {
            None
        } else {
            return Err(vm.new_os_error(String::from("Specified gid is not valid.")));
        };

        let flag = if follow_symlinks.0 {
            nix::unistd::FchownatFlags::FollowSymlink
        } else {
            nix::unistd::FchownatFlags::NoFollowSymlink
        };

        let dir_fd = dir_fd.get_opt();
        match path {
            PathOrFd::Path(ref p) => {
                nix::unistd::fchownat(dir_fd, p.path.as_os_str(), uid, gid, flag)
            }
            PathOrFd::Fd(fd) => nix::unistd::fchown(fd, uid, gid),
        }
        .map_err(|err| {
            // Use `From<nix::Error> for io::Error` when it is available
            IOErrorBuilder::new(io::Error::from_raw_os_error(err as i32))
                .filename(path)
                .into_pyexception(vm)
        })
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn lchown(path: PyPathLike, uid: isize, gid: isize, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            PathOrFd::Path(path),
            uid,
            gid,
            DirFd::default(),
            FollowSymlinks(false),
            vm,
        )
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchown(fd: i32, uid: isize, gid: isize, vm: &VirtualMachine) -> PyResult<()> {
        chown(
            PathOrFd::Fd(fd),
            uid,
            gid,
            DirFd::default(),
            FollowSymlinks(true),
            vm,
        )
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn nice(increment: i32, vm: &VirtualMachine) -> PyResult<i32> {
        use nix::errno::{errno, Errno};
        Errno::clear();
        let res = unsafe { libc::nice(increment) };
        if res == -1 && errno() != 0 {
            Err(errno_err(vm))
        } else {
            Ok(res)
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn sched_get_priority_max(policy: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let max = unsafe { libc::sched_get_priority_max(policy) };
        if max == -1 {
            Err(errno_err(vm))
        } else {
            Ok(max)
        }
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn sched_get_priority_min(policy: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let min = unsafe { libc::sched_get_priority_min(policy) };
        if min == -1 {
            Err(errno_err(vm))
        } else {
            Ok(min)
        }
    }

    #[pyfunction]
    fn sched_yield(vm: &VirtualMachine) -> PyResult<()> {
        let _ = nix::sched::sched_yield().map_err(|e| e.into_pyexception(vm))?;
        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "sched_param")]
    #[derive(Debug, PyValue)]
    struct SchedParam {
        sched_priority: PyObjectRef,
    }

    impl TryFromObject for SchedParam {
        fn try_from_object(_vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            Ok(SchedParam {
                sched_priority: obj,
            })
        }
    }

    #[pyimpl(with(SlotConstructor))]
    impl SchedParam {
        #[pyproperty]
        fn sched_priority(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.sched_priority.clone().into_pyobject(vm)
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let sched_priority_repr = vm.to_repr(&self.sched_priority)?;
            Ok(format!(
                "posix.sched_param(sched_priority = {})",
                sched_priority_repr.as_str()
            ))
        }

        #[cfg(any(
            target_os = "linux",
            target_os = "netbsd",
            target_os = "freebsd",
            target_os = "android"
        ))]
        fn try_to_libc(&self, vm: &VirtualMachine) -> PyResult<libc::sched_param> {
            use crate::TypeProtocol;
            let priority = self.sched_priority.clone();
            let priority_type = priority.class().name();
            let value = priority.downcast::<PyInt>().map_err(|_| {
                vm.new_type_error(format!(
                    "an integer is required (got type {})",
                    priority_type
                ))
            })?;
            let sched_priority = value.try_to_primitive(vm)?;
            Ok(libc::sched_param { sched_priority })
        }
    }

    #[derive(FromArgs)]
    pub struct SchedParamArg {
        sched_priority: PyObjectRef,
    }
    impl SlotConstructor for SchedParam {
        type Args = SchedParamArg;
        fn py_new(cls: PyTypeRef, arg: Self::Args, vm: &VirtualMachine) -> PyResult {
            SchedParam {
                sched_priority: arg.sched_priority,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[pyfunction]
    fn sched_getscheduler(pid: libc::pid_t, vm: &VirtualMachine) -> PyResult<i32> {
        let policy = unsafe { libc::sched_getscheduler(pid) };
        if policy == -1 {
            Err(errno_err(vm))
        } else {
            Ok(policy)
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[derive(FromArgs)]
    struct SchedSetschedulerArgs {
        #[pyarg(positional)]
        pid: i32,
        #[pyarg(positional)]
        policy: i32,
        #[pyarg(positional)]
        sched_param_obj: crate::PyRef<SchedParam>,
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[pyfunction]
    fn sched_setscheduler(args: SchedSetschedulerArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let libc_sched_param = args.sched_param_obj.try_to_libc(vm)?;
        let policy = unsafe { libc::sched_setscheduler(args.pid, args.policy, &libc_sched_param) };
        if policy == -1 {
            Err(errno_err(vm))
        } else {
            Ok(policy)
        }
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[pyfunction]
    fn sched_getparam(pid: libc::pid_t, vm: &VirtualMachine) -> PyResult<SchedParam> {
        let param = unsafe {
            let mut param = std::mem::MaybeUninit::uninit();
            if -1 == libc::sched_getparam(pid, param.as_mut_ptr()) {
                return Err(errno_err(vm));
            }
            param.assume_init()
        };
        Ok(SchedParam {
            sched_priority: param.sched_priority.into_pyobject(vm),
        })
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[derive(FromArgs)]
    struct SchedSetParamArgs {
        #[pyarg(positional)]
        pid: i32,
        #[pyarg(positional)]
        sched_param_obj: crate::PyRef<SchedParam>,
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "netbsd",
        target_os = "freebsd",
        target_os = "android"
    ))]
    #[pyfunction]
    fn sched_setparam(args: SchedSetParamArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let libc_sched_param = args.sched_param_obj.try_to_libc(vm)?;
        let ret = unsafe { libc::sched_setparam(args.pid, &libc_sched_param) };
        if ret == -1 {
            Err(errno_err(vm))
        } else {
            Ok(ret)
        }
    }

    #[pyfunction]
    fn get_inheritable(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
        let flags = fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFD);
        match flags {
            Ok(ret) => Ok((ret & libc::FD_CLOEXEC) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn set_inheritable(fd: i32, inheritable: bool, vm: &VirtualMachine) -> PyResult<()> {
        super::raw_set_inheritable(fd, inheritable).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn get_blocking(fd: RawFd, vm: &VirtualMachine) -> PyResult<bool> {
        let flags = fcntl::fcntl(fd, fcntl::FcntlArg::F_GETFL);
        match flags {
            Ok(ret) => Ok((ret & libc::O_NONBLOCK) == 0),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn set_blocking(fd: RawFd, blocking: bool, vm: &VirtualMachine) -> PyResult<()> {
        let _set_flag = || {
            use nix::fcntl::{fcntl, FcntlArg, OFlag};

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
    fn pipe(vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
        use nix::unistd::close;
        use nix::unistd::pipe;
        let (rfd, wfd) = pipe().map_err(|err| err.into_pyexception(vm))?;
        set_inheritable(rfd, false, vm)
            .and_then(|_| set_inheritable(wfd, false, vm))
            .map_err(|err| {
                let _ = close(rfd);
                let _ = close(wfd);
                err
            })?;
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
    fn pipe2(flags: libc::c_int, vm: &VirtualMachine) -> PyResult<(RawFd, RawFd)> {
        let oflags = fcntl::OFlag::from_bits_truncate(flags);
        nix::unistd::pipe2(oflags).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn system(command: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
        let cstr = command.to_cstring(vm)?;
        let x = unsafe { libc::system(cstr.as_ptr()) };
        Ok(x)
    }

    fn _chmod(
        path: PyPathLike,
        dir_fd: DirFd<0>,
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
        body().map_err(|err| {
            IOErrorBuilder::new(err)
                .filename(err_path)
                .into_pyexception(vm)
        })
    }

    #[cfg(not(target_os = "redox"))]
    fn _fchmod(fd: RawFd, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        nix::sys::stat::fchmod(
            fd,
            nix::sys::stat::Mode::from_bits(mode as libc::mode_t).unwrap(),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn chmod(
        path: PathOrFd,
        dir_fd: DirFd<0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match path {
            PathOrFd::Path(path) => _chmod(path, dir_fd, mode, follow_symlinks, vm),
            PathOrFd::Fd(fd) => _fchmod(fd, mode, vm),
        }
    }

    #[cfg(target_os = "redox")]
    #[pyfunction]
    fn chmod(
        path: PyPathLike,
        dir_fd: DirFd<0>,
        mode: u32,
        follow_symlinks: FollowSymlinks,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        _chmod(path, dir_fd, mode, follow_symlinks, vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn fchmod(fd: RawFd, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        _fchmod(fd, mode, vm)
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn lchmod(path: PyPathLike, mode: u32, vm: &VirtualMachine) -> PyResult<()> {
        _chmod(path, DirFd::default(), mode, FollowSymlinks(false), vm)
    }

    #[pyfunction]
    fn execv(
        path: PyPathLike,
        argv: Either<PyListRef, PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = path.into_cstring(vm)?;

        let argv = vm.extract_elements_func(argv.as_ref(), |obj| {
            PyStrRef::try_from_object(vm, obj)?.to_cstring(vm)
        })?;
        let argv: Vec<&CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execv() arg 2 must not be empty".to_owned()))?;
        if first.to_bytes().is_empty() {
            return Err(
                vm.new_value_error("execv() arg 2 first element cannot be empty".to_owned())
            );
        }

        unistd::execv(&path, &argv)
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn execve(
        path: PyPathLike,
        argv: Either<PyListRef, PyTupleRef>,
        env: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let path = path.into_cstring(vm)?;

        let argv = vm.extract_elements_func(argv.as_ref(), |obj| {
            PyStrRef::try_from_object(vm, obj)?.to_cstring(vm)
        })?;
        let argv: Vec<&CStr> = argv.iter().map(|entry| entry.as_c_str()).collect();

        let first = argv
            .first()
            .ok_or_else(|| vm.new_value_error("execve() arg 2 must not be empty".to_owned()))?;

        if first.to_bytes().is_empty() {
            return Err(
                vm.new_value_error("execve() arg 2 first element cannot be empty".to_owned())
            );
        }

        let env = env
            .into_iter()
            .map(|(k, v)| -> PyResult<_> {
                let (key, value) = (
                    PyPathLike::try_from_object(vm, k)?.into_bytes(),
                    PyPathLike::try_from_object(vm, v)?.into_bytes(),
                );

                if memchr::memchr(b'=', &key).is_some() {
                    return Err(vm.new_value_error("illegal environment variable name".to_owned()));
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
        vm.ctx.new_int(ppid)
    }

    #[pyfunction]
    fn getgid(vm: &VirtualMachine) -> PyObjectRef {
        let gid = unistd::getgid().as_raw();
        vm.ctx.new_int(gid)
    }

    #[pyfunction]
    fn getegid(vm: &VirtualMachine) -> PyObjectRef {
        let egid = unistd::getegid().as_raw();
        vm.ctx.new_int(egid)
    }

    #[pyfunction]
    fn getpgid(pid: u32, vm: &VirtualMachine) -> PyResult {
        match unistd::getpgid(Some(Pid::from_raw(pid as i32))) {
            Ok(pgid) => Ok(vm.ctx.new_int(pgid.as_raw())),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn getpgrp(vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_int(unistd::getpgrp().as_raw()))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn getsid(pid: u32, vm: &VirtualMachine) -> PyResult {
        match unistd::getsid(Some(Pid::from_raw(pid as i32))) {
            Ok(sid) => Ok(vm.ctx.new_int(sid.as_raw())),
            Err(err) => Err(err.into_pyexception(vm)),
        }
    }

    #[pyfunction]
    fn getuid(vm: &VirtualMachine) -> PyObjectRef {
        let uid = unistd::getuid().as_raw();
        vm.ctx.new_int(uid)
    }

    #[pyfunction]
    fn geteuid(vm: &VirtualMachine) -> PyObjectRef {
        let euid = unistd::geteuid().as_raw();
        vm.ctx.new_int(euid)
    }

    #[pyfunction]
    fn setgid(gid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setgid(Gid::from_raw(gid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setegid(egid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setegid(Gid::from_raw(egid)).map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setpgid(pid: u32, pgid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setpgid(Pid::from_raw(pid as i32), Pid::from_raw(pgid as i32))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setsid(vm: &VirtualMachine) -> PyResult<()> {
        unistd::setsid()
            .map(|_ok| ())
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pyfunction]
    fn setuid(uid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setuid(Uid::from_raw(uid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn seteuid(euid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::seteuid(Uid::from_raw(euid)).map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn setreuid(ruid: u32, euid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setuid(Uid::from_raw(ruid)).map_err(|err| err.into_pyexception(vm))?;
        unistd::seteuid(Uid::from_raw(euid)).map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresuid(ruid: u32, euid: u32, suid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setresuid(
            Uid::from_raw(ruid),
            Uid::from_raw(euid),
            Uid::from_raw(suid),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(not(target_os = "redox"))]
    #[pyfunction]
    fn openpty(vm: &VirtualMachine) -> PyResult {
        let r = nix::pty::openpty(None, None).map_err(|err| err.into_pyexception(vm))?;
        for fd in &[r.master, r.slave] {
            super::raw_set_inheritable(*fd, false).map_err(|e| e.into_pyexception(vm))?;
        }
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(r.master), vm.ctx.new_int(r.slave)]))
    }

    #[pyfunction]
    fn ttyname(fd: i32, vm: &VirtualMachine) -> PyResult {
        let name = unsafe { libc::ttyname(fd) };
        if name.is_null() {
            Err(errno_err(vm))
        } else {
            let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap();
            Ok(vm.ctx.new_utf8_str(name))
        }
    }

    #[pyfunction]
    fn umask(mask: libc::mode_t) -> libc::mode_t {
        unsafe { libc::umask(mask) }
    }

    #[pyfunction]
    fn uname(vm: &VirtualMachine) -> PyResult<_os::UnameResult> {
        let info = uname::uname().map_err(|err| err.into_pyexception(vm))?;
        Ok(_os::UnameResult {
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
    fn getresuid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
        let mut ruid = 0;
        let mut euid = 0;
        let mut suid = 0;
        let ret = unsafe { libc::getresuid(&mut ruid, &mut euid, &mut suid) };
        if ret == 0 {
            Ok((ruid, euid, suid))
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn getresgid(vm: &VirtualMachine) -> PyResult<(u32, u32, u32)> {
        let mut rgid = 0;
        let mut egid = 0;
        let mut sgid = 0;
        let ret = unsafe { libc::getresgid(&mut rgid, &mut egid, &mut sgid) };
        if ret == 0 {
            Ok((rgid, egid, sgid))
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn setresgid(rgid: u32, egid: u32, sgid: u32, vm: &VirtualMachine) -> PyResult<()> {
        unistd::setresgid(
            Gid::from_raw(rgid),
            Gid::from_raw(egid),
            Gid::from_raw(sgid),
        )
        .map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(any(target_os = "android", target_os = "linux", target_os = "openbsd"))]
    #[pyfunction]
    fn setregid(rgid: u32, egid: u32, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { libc::setregid(rgid, egid) };
        if ret == 0 {
            Ok(())
        } else {
            Err(errno_err(vm))
        }
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn initgroups(user_name: PyStrRef, gid: u32, vm: &VirtualMachine) -> PyResult<()> {
        let user = CString::new(user_name.as_str()).unwrap();
        let gid = Gid::from_raw(gid);
        unistd::initgroups(&user, gid).map_err(|err| err.into_pyexception(vm))
    }

    // cfg from nix
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
    #[pyfunction]
    fn setgroups(
        group_ids: crate::function::ArgIterable<u32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let gids = group_ids
            .iter(vm)?
            .map(|entry| match entry {
                Ok(id) => Ok(unistd::Gid::from_raw(id)),
                Err(err) => Err(err),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = unistd::setgroups(&gids);
        ret.map_err(|err| err.into_pyexception(vm))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
    fn envp_from_dict(
        env: crate::protocol::PyMapping,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<CString>> {
        let keys = env.keys(vm)?;
        let values = env.values(vm)?;

        let keys = PyListRef::try_from_object(vm, keys)
            .map_err(|_| vm.new_type_error("env.keys() is not a list".to_owned()))?
            .borrow_vec()
            .to_vec();
        let values = PyListRef::try_from_object(vm, values)
            .map_err(|_| vm.new_type_error("env.values() is not a list".to_owned()))?
            .borrow_vec()
            .to_vec();

        keys.into_iter()
            .zip(values.into_iter())
            .map(|(k, v)| {
                let k = PyPathLike::try_from_object(vm, k)?.into_bytes();
                let v = PyPathLike::try_from_object(vm, v)?.into_bytes();
                if k.contains(&0) {
                    return Err(
                        vm.new_value_error("envp dict key cannot contain a nul byte".to_owned())
                    );
                }
                if k.contains(&b'=') {
                    return Err(vm.new_value_error(
                        "envp dict key cannot contain a '=' character".to_owned(),
                    ));
                }
                if v.contains(&0) {
                    return Err(
                        vm.new_value_error("envp dict value cannot contain a nul byte".to_owned())
                    );
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
        path: PyPathLike,
        #[pyarg(positional)]
        args: crate::function::ArgIterable<PyPathLike>,
        #[pyarg(positional)]
        env: crate::protocol::PyMapping,
        #[pyarg(named, default)]
        file_actions: Option<crate::function::ArgIterable<PyTupleRef>>,
        #[pyarg(named, default)]
        setsigdef: Option<crate::function::ArgIterable<i32>>,
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

            let path = CString::new(self.path.into_bytes())
                .map_err(|_| vm.new_value_error("path should not have nul bytes".to_owned()))?;

            let mut file_actions = unsafe {
                let mut fa = std::mem::MaybeUninit::uninit();
                assert!(libc::posix_spawn_file_actions_init(fa.as_mut_ptr()) == 0);
                fa.assume_init()
            };
            if let Some(it) = self.file_actions {
                for action in it.iter(vm)? {
                    let action = action?;
                    let (id, args) = action.as_slice().split_first().ok_or_else(|| {
                        vm.new_type_error(
                            "Each file_actions element must be a non-empty tuple".to_owned(),
                        )
                    })?;
                    let id = i32::try_from_borrowed_object(vm, id)?;
                    let id = PosixSpawnFileActionIdentifier::try_from(id).map_err(|_| {
                        vm.new_type_error("Unknown file_actions identifier".to_owned())
                    })?;
                    let args: crate::function::FuncArgs = args.to_vec().into();
                    let ret = match id {
                        PosixSpawnFileActionIdentifier::Open => {
                            let (fd, path, oflag, mode): (_, PyPathLike, _, _) = args.bind(vm)?;
                            let path = CString::new(path.into_bytes()).map_err(|_| {
                                vm.new_value_error(
                                    "POSIX_SPAWN_OPEN path should not have nul bytes".to_owned(),
                                )
                            })?;
                            unsafe {
                                libc::posix_spawn_file_actions_addopen(
                                    &mut file_actions,
                                    fd,
                                    path.as_ptr(),
                                    oflag,
                                    mode,
                                )
                            }
                        }
                        PosixSpawnFileActionIdentifier::Close => {
                            let (fd,) = args.bind(vm)?;
                            unsafe {
                                libc::posix_spawn_file_actions_addclose(&mut file_actions, fd)
                            }
                        }
                        PosixSpawnFileActionIdentifier::Dup2 => {
                            let (fd, newfd) = args.bind(vm)?;
                            unsafe {
                                libc::posix_spawn_file_actions_adddup2(&mut file_actions, fd, newfd)
                            }
                        }
                    };
                    if ret != 0 {
                        return Err(errno_err(vm));
                    }
                }
            }

            let mut attrp = unsafe {
                let mut sa = std::mem::MaybeUninit::uninit();
                assert!(libc::posix_spawnattr_init(sa.as_mut_ptr()) == 0);
                sa.assume_init()
            };
            if let Some(sigs) = self.setsigdef {
                use nix::sys::signal;
                let mut set = signal::SigSet::empty();
                for sig in sigs.iter(vm)? {
                    let sig = sig?;
                    let sig = signal::Signal::try_from(sig).map_err(|_| {
                        vm.new_value_error(format!("signal number {} out of range", sig))
                    })?;
                    set.add(sig);
                }
                assert!(
                    unsafe { libc::posix_spawnattr_setsigdefault(&mut attrp, set.as_ref()) } == 0
                );
            }

            let mut args: Vec<CString> = self
                .args
                .iter(vm)?
                .map(|res| {
                    CString::new(res?.into_bytes()).map_err(|_| {
                        vm.new_value_error("path should not have nul bytes".to_owned())
                    })
                })
                .collect::<Result<_, _>>()?;
            let argv: Vec<*mut libc::c_char> = args
                .iter_mut()
                .map(|s| s.as_ptr() as _)
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect();
            let mut env = envp_from_dict(self.env, vm)?;
            let envp: Vec<*mut libc::c_char> = env
                .iter_mut()
                .map(|s| s.as_ptr() as _)
                .chain(std::iter::once(std::ptr::null_mut()))
                .collect();

            let mut pid = 0;
            let ret = unsafe {
                if spawnp {
                    libc::posix_spawnp(
                        &mut pid,
                        path.as_ptr(),
                        &file_actions,
                        &attrp,
                        argv.as_ptr(),
                        envp.as_ptr(),
                    )
                } else {
                    libc::posix_spawn(
                        &mut pid,
                        path.as_ptr(),
                        &file_actions,
                        &attrp,
                        argv.as_ptr(),
                        envp.as_ptr(),
                    )
                }
            };

            if ret == 0 {
                Ok(pid)
            } else {
                Err(errno_err(vm))
            }
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

    #[pyfunction(name = "WIFSIGNALED")]
    fn wifsignaled(status: i32) -> bool {
        libc::WIFSIGNALED(status)
    }
    #[pyfunction(name = "WIFSTOPPED")]
    fn wifstopped(status: i32) -> bool {
        libc::WIFSTOPPED(status)
    }
    #[pyfunction(name = "WIFEXITED")]
    fn wifexited(status: i32) -> bool {
        libc::WIFEXITED(status)
    }
    #[pyfunction(name = "WTERMSIG")]
    fn wtermsig(status: i32) -> i32 {
        libc::WTERMSIG(status)
    }
    #[pyfunction(name = "WSTOPSIG")]
    fn wstopsig(status: i32) -> i32 {
        libc::WSTOPSIG(status)
    }
    #[pyfunction(name = "WEXITSTATUS")]
    fn wexitstatus(status: i32) -> i32 {
        libc::WEXITSTATUS(status)
    }

    #[pyfunction]
    fn waitpid(pid: libc::pid_t, opt: i32, vm: &VirtualMachine) -> PyResult<(libc::pid_t, i32)> {
        let mut status = 0;
        let pid = unsafe { libc::waitpid(pid, &mut status, opt) };
        let pid = nix::Error::result(pid).map_err(|err| err.into_pyexception(vm))?;
        Ok((pid, status))
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
                Err(errno_err(vm))
            } else {
                Ok(())
            }
        }
    }

    #[pyfunction]
    fn get_terminal_size(
        fd: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<_os::PyTerminalSize> {
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
        Ok(_os::PyTerminalSize { columns, lines })
    }

    // from libstd:
    // https://github.com/rust-lang/rust/blob/daecab3a784f28082df90cebb204998051f3557d/src/libstd/sys/unix/fs.rs#L1251
    #[cfg(target_os = "macos")]
    extern "C" {
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
        let ret = unsafe { fcopyfile(in_fd, out_fd, std::ptr::null_mut(), flags as u32) };
        if ret < 0 {
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }

    #[pyfunction]
    fn dup(fd: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let fd = nix::unistd::dup(fd).map_err(|e| e.into_pyexception(vm))?;
        super::raw_set_inheritable(fd, false)
            .map(|()| fd)
            .map_err(|e| {
                let _ = nix::unistd::close(fd);
                e.into_pyexception(vm)
            })
    }

    #[derive(FromArgs)]
    struct Dup2Args {
        #[pyarg(positional)]
        fd: i32,
        #[pyarg(positional)]
        fd2: i32,
        #[pyarg(any, default = "true")]
        inheritable: bool,
    }

    #[pyfunction]
    fn dup2(args: Dup2Args, vm: &VirtualMachine) -> PyResult<i32> {
        let fd = nix::unistd::dup2(args.fd, args.fd2).map_err(|e| e.into_pyexception(vm))?;
        if !args.inheritable {
            super::raw_set_inheritable(fd, false).map_err(|e| {
                let _ = nix::unistd::close(fd);
                e.into_pyexception(vm)
            })?
        }
        Ok(fd)
    }

    pub(crate) fn support_funcs() -> Vec<SupportFunc> {
        vec![
            SupportFunc::new("chmod", Some(false), Some(false), Some(false)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("chroot", Some(false), None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("chown", Some(true), Some(true), Some(true)),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("lchown", None, None, None),
            #[cfg(not(target_os = "redox"))]
            SupportFunc::new("fchown", Some(true), None, Some(true)),
            SupportFunc::new("umask", Some(false), Some(false), Some(false)),
            SupportFunc::new("execv", None, None, None),
            SupportFunc::new("pathconf", Some(true), None, None),
        ]
    }

    /// Return a string containing the name of the user logged in on the
    /// controlling terminal of the process.
    ///
    /// Exceptions:
    ///
    /// - `OSError`: Raised if login name could not be determined (`getlogin()`
    ///   returned a null pointer).
    /// - `UnicodeDecodeError`: Raised if login name contained invalid UTF-8 bytes.
    #[pyfunction]
    fn getlogin(vm: &VirtualMachine) -> PyResult<String> {
        // Get a pointer to the login name string. The string is statically
        // allocated and might be overwritten on subsequent calls to this
        // function or to `cuserid()`. See man getlogin(3) for more information.
        let ptr = unsafe { libc::getlogin() };
        if ptr.is_null() {
            return Err(vm.new_os_error("unable to determine login name".to_owned()));
        }
        let slice = unsafe { CStr::from_ptr(ptr) };
        slice
            .to_str()
            .map(|s| s.to_owned())
            .map_err(|e| vm.new_unicode_decode_error(format!("unable to decode login name: {}", e)))
    }

    // cfg from nix
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "openbsd"
    ))]
    #[pyfunction]
    fn getgrouplist(user: PyStrRef, group: u32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let user = CString::new(user.as_str()).unwrap();
        let gid = Gid::from_raw(group);
        let group_ids = unistd::getgrouplist(&user, gid).map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_list(
            group_ids
                .into_iter()
                .map(|gid| vm.ctx.new_int(gid.as_raw()))
                .collect(),
        ))
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
        use nix::errno::{errno, Errno};
        Errno::clear();
        let retval = unsafe { libc::getpriority(which, who) };
        if errno() != 0 {
            Err(errno_err(vm))
        } else {
            Ok(vm.ctx.new_int(retval))
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
            Err(errno_err(vm))
        } else {
            Ok(())
        }
    }

    struct ConfName(i32);

    impl TryFromObject for ConfName {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let i = match obj.downcast::<PyInt>() {
                Ok(int) => int.try_to_primitive(vm)?,
                Err(obj) => {
                    let s = PyStrRef::try_from_object(vm, obj)?;
                    s.as_str().parse::<PathconfVar>().map_err(|_| {
                        vm.new_value_error("unrecognized configuration name".to_string())
                    })? as i32
                }
            };
            Ok(Self(i))
        }
    }

    // Copy from [nix::unistd::PathconfVar](https://docs.rs/nix/0.21.0/nix/unistd/enum.PathconfVar.html)
    // Change enum name to fit python doc
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, EnumString)]
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
        path: PathOrFd,
        ConfName(name): ConfName,
        vm: &VirtualMachine,
    ) -> PyResult<Option<libc::c_long>> {
        use nix::errno::{self, Errno};

        Errno::clear();
        let raw = match path {
            PathOrFd::Path(path) => {
                let path = CString::new(path.into_bytes())
                    .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;
                unsafe { libc::pathconf(path.as_ptr(), name) }
            }
            PathOrFd::Fd(fd) => unsafe { libc::fpathconf(fd, name) },
        };

        if raw == -1 {
            if errno::errno() == 0 {
                Ok(None)
            } else {
                Err(io::Error::from(Errno::last()).into_pyexception(vm))
            }
        } else {
            Ok(Some(raw))
        }
    }

    #[pyfunction]
    fn fpathconf(fd: i32, name: ConfName, vm: &VirtualMachine) -> PyResult<Option<libc::c_long>> {
        pathconf(PathOrFd::Fd(fd), name, vm)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[derive(FromArgs)]
    struct SendFileArgs {
        out_fd: i32,
        in_fd: i32,
        offset: crate::crt_fd::Offset,
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
    fn sendfile(args: SendFileArgs, vm: &VirtualMachine) -> PyResult {
        let mut file_offset = args.offset;

        let res = nix::sys::sendfile::sendfile(
            args.out_fd,
            args.in_fd,
            Some(&mut file_offset),
            args.count as usize,
        )
        .map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_int(res as u64))
    }

    #[cfg(target_os = "macos")]
    fn _extract_vec_bytes(
        x: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<crate::function::ArgBytesLike>>> {
        let inner = match x.into_option() {
            Some(v) => {
                let v = vm.extract_elements::<crate::function::ArgBytesLike>(&v)?;
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            }
            None => None,
        };
        Ok(inner)
    }

    #[cfg(target_os = "macos")]
    #[pyfunction]
    fn sendfile(args: SendFileArgs, vm: &VirtualMachine) -> PyResult {
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
        res.map_err(|err| err.into_pyexception(vm))?;
        Ok(vm.ctx.new_int(written as u64))
    }
}

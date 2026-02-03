// spell-checker:disable

use crate::vm::{
    builtins::PyListRef,
    function::ArgSequence,
    ospath::OsPath,
    stdlib::posix,
    {PyObjectRef, PyResult, TryFromObject, VirtualMachine},
};
use itertools::Itertools;
use nix::{
    errno::Errno,
    unistd::{self, Pid},
};
use std::{
    io::prelude::*,
    os::fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd, RawFd},
};
use unistd::{Gid, Uid};

use alloc::ffi::CString;

use core::{convert::Infallible as Never, ffi::CStr, marker::PhantomData, ops::Deref};

pub(crate) use _posixsubprocess::module_def;

#[pymodule]
mod _posixsubprocess {
    use rustpython_vm::{AsObject, TryFromBorrowedObject};

    use super::*;
    use crate::vm::{PyResult, VirtualMachine, convert::IntoPyException};

    #[pyfunction]
    fn fork_exec(args: ForkExecArgs<'_>, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        // Check for interpreter shutdown when preexec_fn is used
        if args.preexec_fn.is_some()
            && vm
                .state
                .finalizing
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_python_finalization_error(
                "preexec_fn not supported at interpreter shutdown".to_owned(),
            ));
        }

        let extra_groups = args
            .groups_list
            .as_ref()
            .map(|l| Vec::<Gid>::try_from_borrowed_object(vm, l.as_object()))
            .transpose()?;
        let argv = CharPtrVec::from_iter(args.args.iter());
        let envp = args.env_list.as_ref().map(CharPtrVec::from_iter);
        let procargs = ProcArgs {
            argv: &argv,
            envp: envp.as_deref(),
            extra_groups: extra_groups.as_deref(),
        };
        match unsafe { nix::unistd::fork() }.map_err(|err| err.into_pyexception(vm))? {
            nix::unistd::ForkResult::Child => exec(&args, procargs, vm),
            nix::unistd::ForkResult::Parent { child } => Ok(child.as_raw()),
        }
    }
}

macro_rules! gen_args {
    ($($field:ident: $t:ty),*$(,)?) => {
        #[derive(FromArgs)]
        struct ForkExecArgs<'fd> {
            $(#[pyarg(positional)] $field: $t,)*
        }
    };
}

struct CStrPathLike {
    s: CString,
}
impl TryFromObject for CStrPathLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let s = OsPath::try_from_object(vm, obj)?.into_cstring(vm)?;
        Ok(Self { s })
    }
}
impl AsRef<CStr> for CStrPathLike {
    fn as_ref(&self) -> &CStr {
        &self.s
    }
}

#[derive(Default)]
struct CharPtrVec<'a> {
    vec: Vec<*const libc::c_char>,
    marker: PhantomData<Vec<&'a CStr>>,
}

impl<'a, T: AsRef<CStr>> FromIterator<&'a T> for CharPtrVec<'a> {
    fn from_iter<I: IntoIterator<Item = &'a T>>(iter: I) -> Self {
        let vec = iter
            .into_iter()
            .map(|x| x.as_ref().as_ptr())
            .chain(core::iter::once(core::ptr::null()))
            .collect();
        Self {
            vec,
            marker: PhantomData,
        }
    }
}

impl<'a> Deref for CharPtrVec<'a> {
    type Target = CharPtrSlice<'a>;
    fn deref(&self) -> &Self::Target {
        unsafe {
            &*(self.vec.as_slice() as *const [*const libc::c_char] as *const CharPtrSlice<'a>)
        }
    }
}

#[repr(transparent)]
struct CharPtrSlice<'a> {
    marker: PhantomData<[&'a CStr]>,
    slice: [*const libc::c_char],
}

impl CharPtrSlice<'_> {
    const fn as_ptr(&self) -> *const *const libc::c_char {
        self.slice.as_ptr()
    }
}

#[derive(Copy, Clone)]
struct Fd(BorrowedFd<'static>);

impl TryFromObject for Fd {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match MaybeFd::try_from_object(vm, obj)? {
            MaybeFd::Valid(fd) => Ok(fd),
            MaybeFd::Invalid => Err(vm.new_value_error("invalid fd")),
        }
    }
}

impl Write for Fd {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(unistd::write(self, buf)?)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl IntoRawFd for Fd {
    fn into_raw_fd(self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsFd for Fd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl From<OwnedFd> for Fd {
    fn from(fd: OwnedFd) -> Self {
        Self(unsafe { BorrowedFd::borrow_raw(fd.into_raw_fd()) })
    }
}

#[derive(Copy, Clone)]
enum MaybeFd {
    Valid(Fd),
    Invalid,
}

impl TryFromObject for MaybeFd {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fd = i32::try_from_object(vm, obj)?;
        Ok(if fd == -1 {
            MaybeFd::Invalid
        } else {
            MaybeFd::Valid(Fd(unsafe { BorrowedFd::borrow_raw(fd) }))
        })
    }
}

impl AsRawFd for MaybeFd {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            MaybeFd::Valid(fd) => fd.as_raw_fd(),
            MaybeFd::Invalid => -1,
        }
    }
}

// impl

gen_args! {
    args: ArgSequence<CStrPathLike> /* list */,
    exec_list: ArgSequence<CStrPathLike> /* list */,
    close_fds: bool,
    fds_to_keep: ArgSequence<BorrowedFd<'fd>>,
    cwd: Option<CStrPathLike>,
    env_list: Option<ArgSequence<CStrPathLike>>,
    p2cread: MaybeFd,
    p2cwrite: MaybeFd,
    c2pread: MaybeFd,
    c2pwrite: MaybeFd,
    errread: MaybeFd,
    errwrite: MaybeFd,
    errpipe_read: Fd,
    errpipe_write: Fd,
    restore_signals: bool,
    call_setsid: bool,
    pgid_to_set: libc::pid_t,
    gid: Option<Gid>,
    groups_list: Option<PyListRef>,
    uid: Option<Uid>,
    child_umask: i32,
    preexec_fn: Option<PyObjectRef>,
}

// can't reallocate inside of exec(), so we reallocate prior to fork() and pass this along
struct ProcArgs<'a> {
    argv: &'a CharPtrSlice<'a>,
    envp: Option<&'a CharPtrSlice<'a>>,
    extra_groups: Option<&'a [Gid]>,
}

fn exec(args: &ForkExecArgs<'_>, procargs: ProcArgs<'_>, vm: &VirtualMachine) -> ! {
    let mut ctx = ExecErrorContext::NoExec;
    match exec_inner(args, procargs, &mut ctx, vm) {
        Ok(x) => match x {},
        Err(e) => {
            let mut pipe = args.errpipe_write;
            if matches!(ctx, ExecErrorContext::PreExec) {
                // For preexec_fn errors, use SubprocessError format (errno=0)
                let _ = write!(pipe, "SubprocessError:0:{}", ctx.as_msg());
            } else {
                // errno is written in hex format
                let _ = write!(pipe, "OSError:{:x}:{}", e as i32, ctx.as_msg());
            }
            std::process::exit(255)
        }
    }
}

enum ExecErrorContext {
    NoExec,
    ChDir,
    PreExec,
    Exec,
}

impl ExecErrorContext {
    const fn as_msg(&self) -> &'static str {
        match self {
            Self::NoExec => "noexec",
            Self::ChDir => "noexec:chdir",
            Self::PreExec => "Exception occurred in preexec_fn.",
            Self::Exec => "",
        }
    }
}

fn exec_inner(
    args: &ForkExecArgs<'_>,
    procargs: ProcArgs<'_>,
    ctx: &mut ExecErrorContext,
    vm: &VirtualMachine,
) -> nix::Result<Never> {
    for &fd in args.fds_to_keep.as_slice() {
        if fd.as_raw_fd() != args.errpipe_write.as_raw_fd() {
            posix::set_inheritable(fd, true)?
        }
    }

    for &fd in &[args.p2cwrite, args.c2pread, args.errread] {
        if let MaybeFd::Valid(fd) = fd {
            unistd::close(fd)?;
        }
    }
    unistd::close(args.errpipe_read)?;

    let c2pwrite = match args.c2pwrite {
        MaybeFd::Valid(c2pwrite) if c2pwrite.as_raw_fd() == 0 => {
            let fd = unistd::dup(c2pwrite)?;
            posix::set_inheritable(fd.as_fd(), true)?;
            MaybeFd::Valid(fd.into())
        }
        fd => fd,
    };

    let mut errwrite = args.errwrite;
    loop {
        match errwrite {
            MaybeFd::Valid(fd) if fd.as_raw_fd() == 0 || fd.as_raw_fd() == 1 => {
                let fd = unistd::dup(fd)?;
                posix::set_inheritable(fd.as_fd(), true)?;
                errwrite = MaybeFd::Valid(fd.into());
            }
            _ => break,
        }
    }

    fn dup_into_stdio<F>(fd: MaybeFd, io_fd: i32, dup2_stdio: F) -> nix::Result<()>
    where
        F: Fn(Fd) -> nix::Result<()>,
    {
        match fd {
            MaybeFd::Valid(fd) if fd.as_raw_fd() == io_fd => {
                posix::set_inheritable(fd.as_fd(), true)
            }
            MaybeFd::Valid(fd) => dup2_stdio(fd),
            MaybeFd::Invalid => Ok(()),
        }
    }
    dup_into_stdio(args.p2cread, 0, unistd::dup2_stdin)?;
    dup_into_stdio(c2pwrite, 1, unistd::dup2_stdout)?;
    dup_into_stdio(errwrite, 2, unistd::dup2_stderr)?;

    if let Some(ref cwd) = args.cwd {
        unistd::chdir(cwd.s.as_c_str()).inspect_err(|_| *ctx = ExecErrorContext::ChDir)?
    }

    if args.child_umask >= 0 {
        unsafe { libc::umask(args.child_umask as libc::mode_t) };
    }

    if args.restore_signals {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            libc::signal(libc::SIGXFSZ, libc::SIG_DFL);
        }
    }

    if args.call_setsid {
        unistd::setsid()?;
    }

    if args.pgid_to_set > -1 {
        unistd::setpgid(Pid::from_raw(0), Pid::from_raw(args.pgid_to_set))?;
    }

    if let Some(_groups) = procargs.extra_groups {
        #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "redox")))]
        unistd::setgroups(_groups)?;
    }

    if let Some(gid) = args.gid.filter(|x| x.as_raw() != u32::MAX) {
        let ret = unsafe { libc::setregid(gid.as_raw(), gid.as_raw()) };
        nix::Error::result(ret)?;
    }

    if let Some(uid) = args.uid.filter(|x| x.as_raw() != u32::MAX) {
        let ret = unsafe { libc::setreuid(uid.as_raw(), uid.as_raw()) };
        nix::Error::result(ret)?;
    }

    // Call preexec_fn after all process setup but before closing FDs
    if let Some(ref preexec_fn) = args.preexec_fn {
        match preexec_fn.call((), vm) {
            Ok(_) => {}
            Err(_e) => {
                // Cannot safely stringify exception after fork
                *ctx = ExecErrorContext::PreExec;
                return Err(Errno::UnknownErrno);
            }
        }
    }

    *ctx = ExecErrorContext::Exec;

    if args.close_fds {
        close_fds(KeepFds {
            above: 2,
            keep: &args.fds_to_keep,
        });
    }

    let mut first_err = None;
    for exec in args.exec_list.as_slice() {
        // not using nix's versions of these functions because those allocate the char-ptr array,
        // and we can't allocate
        if let Some(envp) = procargs.envp {
            unsafe { libc::execve(exec.s.as_ptr(), procargs.argv.as_ptr(), envp.as_ptr()) };
        } else {
            unsafe { libc::execv(exec.s.as_ptr(), procargs.argv.as_ptr()) };
        }
        let e = Errno::last();
        if e != Errno::ENOENT && e != Errno::ENOTDIR && first_err.is_none() {
            first_err = Some(e)
        }
    }
    Err(first_err.unwrap_or_else(Errno::last))
}

#[derive(Copy, Clone)]
struct KeepFds<'a> {
    above: i32,
    keep: &'a [BorrowedFd<'a>],
}

impl KeepFds<'_> {
    fn should_keep(self, fd: i32) -> bool {
        fd > self.above
            && self
                .keep
                .binary_search_by_key(&fd, BorrowedFd::as_raw_fd)
                .is_err()
    }
}

fn close_fds(keep: KeepFds<'_>) {
    #[cfg(not(target_os = "redox"))]
    if close_dir_fds(keep).is_ok() {
        return;
    }
    #[cfg(target_os = "redox")]
    if close_filetable_fds(keep).is_ok() {
        return;
    }
    close_fds_brute_force(keep)
}

#[cfg(not(target_os = "redox"))]
fn close_dir_fds(keep: KeepFds<'_>) -> nix::Result<()> {
    use nix::{dir::Dir, fcntl::OFlag};

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
        if fd != dirfd && keep.should_keep(fd) {
            let _ = unistd::close(fd);
        }
    }
    Ok(())
}

#[cfg(target_os = "redox")]
fn close_filetable_fds(keep: KeepFds<'_>) -> nix::Result<()> {
    use nix::fcntl;
    use std::os::fd::{FromRawFd, OwnedFd};
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

        let fd = parser.num as i32;
        if fd != filetable.as_raw_fd() && keep.should_keep(fd) {
            let _ = unistd::close(fd);
        }
        if done {
            break;
        }
    }
    Ok(())
}

fn close_fds_brute_force(keep: KeepFds<'_>) {
    let max_fd = nix::unistd::sysconf(nix::unistd::SysconfVar::OPEN_MAX)
        .ok()
        .flatten()
        .unwrap_or(256) as i32;
    let fds = itertools::chain![
        Some(keep.above),
        keep.keep.iter().map(BorrowedFd::as_raw_fd),
        Some(max_fd)
    ];
    for fd in fds.tuple_windows().flat_map(|(start, end)| start + 1..end) {
        unsafe { libc::close(fd) };
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

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
    convert::Infallible as Never,
    ffi::{CStr, CString},
    io::prelude::*,
    marker::PhantomData,
    ops::Deref,
    os::fd::FromRawFd,
};
use std::{fs::File, os::unix::io::AsRawFd};
use unistd::{Gid, Uid};

pub(crate) use _posixsubprocess::make_module;

#[pymodule]
mod _posixsubprocess {
    use rustpython_vm::{AsObject, TryFromBorrowedObject};

    use super::*;
    use crate::vm::{convert::IntoPyException, PyResult, VirtualMachine};

    #[pyfunction]
    fn fork_exec(args: ForkExecArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        if args.preexec_fn.is_some() {
            return Err(vm.new_not_implemented_error("preexec_fn not supported yet".to_owned()));
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
            nix::unistd::ForkResult::Child => exec(&args, procargs),
            nix::unistd::ForkResult::Parent { child } => Ok(child.as_raw()),
        }
    }
}

macro_rules! gen_args {
    ($($field:ident: $t:ty),*$(,)?) => {
        #[derive(FromArgs)]
        struct ForkExecArgs {
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
        Ok(CStrPathLike { s })
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
            .chain(std::iter::once(std::ptr::null()))
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
    fn as_ptr(&self) -> *const *const libc::c_char {
        self.slice.as_ptr()
    }
}

gen_args! {
    args: ArgSequence<CStrPathLike> /* list */,
    exec_list: ArgSequence<CStrPathLike> /* list */,
    close_fds: bool,
    fds_to_keep: ArgSequence<i32>,
    cwd: Option<CStrPathLike>,
    env_list: Option<ArgSequence<CStrPathLike>>,
    p2cread: i32,
    p2cwrite: i32,
    c2pread: i32,
    c2pwrite: i32,
    errread: i32,
    errwrite: i32,
    errpipe_read: i32,
    errpipe_write: i32,
    restore_signals: bool,
    call_setsid: bool,
    pgid_to_set: libc::pid_t,
    gid: Option<Gid>,
    groups_list: Option<PyListRef>,
    uid: Option<Uid>,
    child_umask: i32,
    preexec_fn: Option<PyObjectRef>,
    _use_vfork: bool,
}

// can't reallocate inside of exec(), so we reallocate prior to fork() and pass this along
struct ProcArgs<'a> {
    argv: &'a CharPtrSlice<'a>,
    envp: Option<&'a CharPtrSlice<'a>>,
    extra_groups: Option<&'a [Gid]>,
}

fn exec(args: &ForkExecArgs, procargs: ProcArgs) -> ! {
    let mut ctx = ExecErrorContext::NoExec;
    match exec_inner(args, procargs, &mut ctx) {
        Ok(x) => match x {},
        Err(e) => {
            let mut pipe =
                std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(args.errpipe_write) });
            let _ = write!(pipe, "OSError:{}:{}", e as i32, ctx.as_msg());
            std::process::exit(255)
        }
    }
}

enum ExecErrorContext {
    NoExec,
    ChDir,
    Exec,
}

impl ExecErrorContext {
    fn as_msg(&self) -> &'static str {
        match self {
            ExecErrorContext::NoExec => "noexec",
            ExecErrorContext::ChDir => "noexec:chdir",
            ExecErrorContext::Exec => "",
        }
    }
}

fn exec_inner(
    args: &ForkExecArgs,
    procargs: ProcArgs,
    ctx: &mut ExecErrorContext,
) -> nix::Result<Never> {
    for &fd in args.fds_to_keep.as_slice() {
        if fd != args.errpipe_write {
            posix::raw_set_inheritable(fd, true)?
        }
    }

    for &fd in &[args.p2cwrite, args.c2pread, args.errread] {
        if fd != -1 {
            unistd::close(fd)?;
        }
    }
    unistd::close(args.errpipe_read)?;

    let c2pwrite = if args.c2pwrite == 0 {
        let fd = unistd::dup(args.c2pwrite)?;
        posix::raw_set_inheritable(fd, true)?;
        fd
    } else {
        args.c2pwrite
    };

    let mut errwrite = args.errwrite;
    while errwrite == 0 || errwrite == 1 {
        errwrite = unistd::dup(errwrite)?;
        posix::raw_set_inheritable(errwrite, true)?;
    }

    let dup_into_stdio = |fd, io_fd| {
        if fd == io_fd {
            posix::raw_set_inheritable(fd, true)
        } else if fd != -1 {
            unistd::dup2(fd, io_fd).map(drop)
        } else {
            Ok(())
        }
    };
    dup_into_stdio(args.p2cread, 0)?;
    dup_into_stdio(c2pwrite, 1)?;
    dup_into_stdio(errwrite, 2)?;

    if let Some(ref cwd) = args.cwd {
        unistd::chdir(cwd.s.as_c_str()).inspect_err(|_| *ctx = ExecErrorContext::ChDir)?
    }

    if args.child_umask >= 0 {
        // TODO: umask(child_umask);
    }

    if args.restore_signals {
        // TODO: restore signals SIGPIPE, SIGXFZ, SIGXFSZ to SIG_DFL
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
    keep: &'a [i32],
}

impl KeepFds<'_> {
    fn should_keep(self, fd: i32) -> bool {
        fd > self.above && self.keep.binary_search(&fd).is_err()
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
    let fd = fcntl::open(
        c"/scheme/thisproc/current/filetable",
        fcntl::OFlag::O_RDONLY,
        nix::sys::stat::Mode::empty(),
    )?;
    let filetable = unsafe { OwnedFd::from_raw_fd(fd) };
    let read_one = || -> nix::Result<_> {
        let mut byte = 0;
        let n = nix::unistd::read(filetable.as_raw_fd(), std::slice::from_mut(&mut byte))?;
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
    let fds = itertools::chain![Some(keep.above), keep.keep.iter().copied(), Some(max_fd)];
    for fd in fds.tuple_windows().flat_map(|(start, end)| start + 1..end) {
        let _ = unistd::close(fd);
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

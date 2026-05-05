// spell-checker:disable

use crate::vm::{
    builtins::PyListRef,
    function::ArgSequence,
    ospath::OsPath,
    {PyObjectRef, PyResult, TryFromObject, VirtualMachine},
};
use rustpython_host_env::posix as host_posix;
use std::{
    io::prelude::*,
    os::fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd, RawFd},
};

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
                .load(core::sync::atomic::Ordering::Acquire)
        {
            return Err(vm.new_python_finalization_error(
                "preexec_fn not supported at interpreter shutdown".to_owned(),
            ));
        }

        let extra_groups = args
            .groups_list
            .as_ref()
            .map(|l| Vec::<RawGid>::try_from_borrowed_object(vm, l.as_object()))
            .map(|res| res.map(|groups| groups.into_iter().map(|gid| gid.0).collect::<Vec<_>>()))
            .transpose()?;
        let argv = args.args.iter().collect::<CharPtrVec<'_>>();
        let envp = args.env_list.as_ref().map(CharPtrVec::from_iter);
        let procargs = ProcArgs {
            argv: &argv,
            envp: envp.as_deref(),
            extra_groups: extra_groups.as_deref(),
        };
        match host_posix::fork().map_err(|err| err.into_pyexception(vm))? {
            0 => exec(&args, procargs, vm),
            child => Ok(child),
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
        host_posix::write_fd(self.as_fd(), buf)
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

#[derive(Copy, Clone)]
struct RawUid(u32);

#[derive(Copy, Clone)]
struct RawGid(u32);

fn try_from_id(vm: &VirtualMachine, obj: PyObjectRef, typ_name: &str) -> PyResult<u32> {
    use core::cmp::Ordering;
    let i = obj
        .try_to_ref::<crate::vm::builtins::PyInt>(vm)
        .map_err(|_| {
            vm.new_type_error(format!(
                "an integer is required (got type {})",
                obj.class().name()
            ))
        })?
        .try_to_primitive::<i64>(vm)?;

    match i.cmp(&-1) {
        Ordering::Greater => Ok(i
            .try_into()
            .map_err(|_| vm.new_overflow_error(format!("{typ_name} is larger than maximum")))?),
        Ordering::Less => Err(vm.new_overflow_error(format!("{typ_name} is less than minimum"))),
        Ordering::Equal => Ok(-1i32 as u32),
    }
}

impl TryFromObject for RawUid {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        try_from_id(vm, obj, "uid").map(Self)
    }
}

impl TryFromObject for RawGid {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        try_from_id(vm, obj, "gid").map(Self)
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
    gid: Option<RawGid>,
    groups_list: Option<PyListRef>,
    uid: Option<RawUid>,
    child_umask: i32,
    preexec_fn: Option<PyObjectRef>,
}

// can't reallocate inside of exec(), so we reallocate prior to fork() and pass this along
struct ProcArgs<'a> {
    argv: &'a CharPtrSlice<'a>,
    envp: Option<&'a CharPtrSlice<'a>>,
    extra_groups: Option<&'a [u32]>,
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
                let errno = e.raw_os_error().unwrap_or(0);
                let _ = write!(pipe, "OSError:{errno:x}:{}", ctx.as_msg());
            }
            rustpython_host_env::os::exit(255)
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
) -> std::io::Result<Never> {
    host_posix::setup_child_fds(
        args.fds_to_keep.as_slice(),
        args.errpipe_write.as_fd(),
        args.p2cread.as_raw_fd(),
        args.p2cwrite.as_raw_fd(),
        args.c2pread.as_raw_fd(),
        args.c2pwrite.as_raw_fd(),
        args.errread.as_raw_fd(),
        args.errwrite.as_raw_fd(),
        args.errpipe_read.as_raw_fd(),
    )?;

    if let Some(ref cwd) = args.cwd {
        host_posix::chdir(cwd.s.as_c_str()).inspect_err(|_| *ctx = ExecErrorContext::ChDir)?
    }

    host_posix::set_umask(args.child_umask);

    if args.restore_signals {
        host_posix::restore_signals();
    }

    host_posix::setsid_if_needed(args.call_setsid)?;
    host_posix::setpgid_if_needed(args.pgid_to_set)?;
    host_posix::setgroups_if_needed(procargs.extra_groups)?;
    host_posix::setregid_if_needed(args.gid.map(|gid| gid.0))?;
    host_posix::setreuid_if_needed(args.uid.map(|uid| uid.0))?;

    // Call preexec_fn after all process setup but before closing FDs
    if let Some(ref preexec_fn) = args.preexec_fn {
        match preexec_fn.call((), vm) {
            Ok(_) => {}
            Err(_e) => {
                // Cannot safely stringify exception after fork
                *ctx = ExecErrorContext::PreExec;
                return Err(std::io::Error::from_raw_os_error(0));
            }
        }
    }

    *ctx = ExecErrorContext::Exec;

    if args.close_fds {
        host_posix::close_fds(2, args.fds_to_keep.as_slice());
    }

    let err = host_posix::exec_replace(
        args.exec_list.as_slice(),
        procargs.argv.as_ptr(),
        procargs.envp.map(CharPtrSlice::as_ptr),
    );
    Err(std::io::Error::from_raw_os_error(err as i32))
}

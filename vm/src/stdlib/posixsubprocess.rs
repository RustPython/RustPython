pub(crate) use _posixsubprocess::make_module;

#[pymodule]
mod _posixsubprocess {
    use super::{exec, CStrPathLike, ForkExecArgs, ProcArgs};
    use crate::exceptions::IntoPyException;
    use crate::pyobject::PyResult;
    use crate::VirtualMachine;

    #[pyfunction]
    fn fork_exec(args: ForkExecArgs, vm: &VirtualMachine) -> PyResult<libc::pid_t> {
        if args.preexec_fn.is_some() {
            return Err(vm.new_not_implemented_error("preexec_fn not supported yet".to_owned()));
        }
        let cstrs_to_ptrs = |cstrs: &[CStrPathLike]| {
            cstrs
                .iter()
                .map(|s| s.s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect::<Vec<_>>()
        };
        let argv = cstrs_to_ptrs(args.args.as_slice());
        let argv = &argv;
        let envp = args.env_list.as_ref().map(|s| cstrs_to_ptrs(s.as_slice()));
        let envp = envp.as_deref();
        match unsafe { nix::unistd::fork() }.map_err(|err| err.into_pyexception(vm))? {
            nix::unistd::ForkResult::Child => exec(&args, ProcArgs { argv, envp }),
            nix::unistd::ForkResult::Parent { child } => Ok(child.as_raw()),
        }
    }
}

use nix::{errno::Errno, fcntl, unistd};
use std::convert::Infallible as Never;
use std::ffi::{CStr, CString};
use std::io::{self, prelude::*};
use std::os::unix::io::AsRawFd;

use super::os;
use crate::pyobject::{PyObjectRef, PyResult, PySequence, TryFromObject};
use crate::VirtualMachine;

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
        let s = os::PyPathLike::try_from_object(vm, obj)?.into_bytes();
        let s = CString::new(s)
            .map_err(|_| vm.new_value_error("embedded null character".to_owned()))?;
        Ok(CStrPathLike { s })
    }
}

gen_args! {
    args: PySequence<CStrPathLike> /* list */, exec_list: PySequence<CStrPathLike> /* list */,
    close_fds: bool, fds_to_keep: PySequence<i32>,
    cwd: Option<CStrPathLike>, env_list: Option<PySequence<CStrPathLike>>,
    p2cread: i32, p2cwrite: i32, c2pread: i32, c2pwrite: i32,
    errread: i32, errwrite: i32, errpipe_read: i32, errpipe_write: i32,
    restore_signals: bool, call_setsid: bool, preexec_fn: Option<PyObjectRef>,
}

// can't reallocate inside of exec(), so we reallocate prior to fork() and pass this along
struct ProcArgs<'a> {
    argv: &'a [*const libc::c_char],
    envp: Option<&'a [*const libc::c_char]>,
}

fn exec(args: &ForkExecArgs, procargs: ProcArgs) -> ! {
    match exec_inner(args, procargs) {
        Ok(x) => match x {},
        Err(e) => {
            let e = e.as_errno().expect("got a non-errno nix error");
            let buf: &mut [u8] = &mut [0; 256];
            let mut cur = io::Cursor::new(&mut *buf);
            // TODO: check if reached preexec, if not then have "noexec" after
            let _ = write!(cur, "OSError:{}:", e as i32);
            let pos = cur.position();
            let _ = unistd::write(args.errpipe_write, &buf[..pos as usize]);
            std::process::exit(255)
        }
    }
}

fn exec_inner(args: &ForkExecArgs, procargs: ProcArgs) -> nix::Result<Never> {
    for &fd in args.fds_to_keep.as_slice() {
        if fd != args.errpipe_write {
            os::raw_set_inheritable(fd, true)?
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
        os::raw_set_inheritable(fd, true)?;
        fd
    } else {
        args.c2pwrite
    };

    let mut errwrite = args.errwrite;
    while errwrite == 0 || errwrite == 1 {
        errwrite = unistd::dup(errwrite)?;
        os::raw_set_inheritable(errwrite, true)?;
    }

    let dup_into_stdio = |fd, io_fd| {
        if fd == io_fd {
            os::raw_set_inheritable(fd, true)
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
        unistd::chdir(cwd.s.as_c_str())?
    }

    if args.restore_signals {
        // TODO: restore signals SIGPIPE, SIGXFZ, SIGXFSZ to SIG_DFL
    }

    if args.call_setsid {
        #[cfg(not(target_os = "redox"))]
        unistd::setsid()?;
    }

    if args.close_fds {
        #[cfg(not(target_os = "redox"))]
        close_fds(3, args.fds_to_keep.as_slice())?;
    }

    let mut first_err = None;
    for exec in args.exec_list.as_slice() {
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
    Err(first_err.unwrap_or_else(Errno::last).into())
}

#[cfg(not(target_os = "redox"))]
fn close_fds(above: i32, keep: &[i32]) -> nix::Result<()> {
    // TODO: close fds by brute force if readdir doesn't work:
    // https://github.com/python/cpython/blob/3.8/Modules/_posixsubprocess.c#L220
    let path = unsafe { CStr::from_bytes_with_nul_unchecked(FD_DIR_NAME) };
    let mut dir = nix::dir::Dir::open(
        path,
        fcntl::OFlag::O_RDONLY | fcntl::OFlag::O_DIRECTORY,
        nix::sys::stat::Mode::empty(),
    )?;
    let dirfd = dir.as_raw_fd();
    for e in dir.iter() {
        if let Some(fd) = pos_int_from_ascii(e?.file_name()) {
            if fd != dirfd && fd > above && !keep.contains(&fd) {
                unistd::close(fd)?
            }
        }
    }
    Ok(())
}

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
))]
const FD_DIR_NAME: &[u8] = b"/dev/fd\0";

#[cfg(any(target_os = "linux", target_os = "android"))]
const FD_DIR_NAME: &[u8] = b"/proc/self/fd\0";

fn pos_int_from_ascii(name: &CStr) -> Option<i32> {
    let mut num = 0;
    for c in name.to_bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        num = num * 10 + i32::from(c - b'0')
    }
    Some(num)
}

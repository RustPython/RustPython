pub(crate) use fcntl::make_module;

#[pymodule]
mod fcntl {
    use crate::vm::{
        builtins::PyIntRef,
        function::{ArgMemoryBuffer, ArgStrOrBytesLike, Either, OptionalArg},
        stdlib::{io, os},
        PyResult, VirtualMachine,
    };

    // TODO: supply these from <asm-generic/fnctl.h> (please file an issue/PR upstream):
    //       LOCK_MAND, LOCK_READ, LOCK_WRITE, LOCK_RW, F_GETSIG, F_SETSIG, F_GETLK64, F_SETLK64,
    //       F_SETLKW64, FASYNC, F_EXLCK, F_SHLCK, DN_ACCESS, DN_MODIFY, DN_CREATE, DN_DELETE,
    //       DN_RENAME, DN_ATTRIB, DN_MULTISHOT
    // NOTE: these are/were from <stropts.h>, which may not be present on systems nowadays:
    //       I_PUSH, I_POP, I_LOOK, I_FLUSH, I_FLUSHBAND, I_SETSIG, I_GETSIG, I_FIND, I_PEEK,
    //       I_SRDOPT, I_GRDOPT, I_NREAD, I_FDINSERT, I_STR, I_SWROPT, I_GWROPT, I_SENDFD,
    //       I_RECVFD, I_LIST, I_ATMARK, I_CKBAND, I_GETBAND, I_CANPUT, I_SETCLTIME, I_GETCLTIME,
    //       I_LINK, I_UNLINK, I_PLINK, I_PUNLINK

    #[pyattr]
    use libc::{FD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};

    #[cfg(not(target_os = "wasi"))]
    #[pyattr]
    use libc::{F_DUPFD, F_DUPFD_CLOEXEC, F_GETLK, F_SETLK, F_SETLKW};

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyattr]
    use libc::{F_GETOWN, F_RDLCK, F_SETOWN, F_UNLCK, F_WRLCK, LOCK_EX, LOCK_NB, LOCK_SH, LOCK_UN};

    #[cfg(target_vendor = "apple")]
    #[pyattr]
    use libc::{F_FULLFSYNC, F_NOCACHE};

    #[cfg(target_os = "freebsd")]
    #[pyattr]
    use libc::{F_DUP2FD, F_DUP2FD_CLOEXEC};

    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use libc::{F_OFD_GETLK, F_OFD_SETLK, F_OFD_SETLKW};

    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    use libc::{
        F_ADD_SEALS, F_GETLEASE, F_GETPIPE_SZ, F_GET_SEALS, F_NOTIFY, F_SEAL_GROW, F_SEAL_SEAL,
        F_SEAL_SHRINK, F_SEAL_WRITE, F_SETLEASE, F_SETPIPE_SZ,
    };

    #[cfg(any(target_os = "dragonfly", target_os = "netbsd", target_vendor = "apple"))]
    #[pyattr]
    use libc::F_GETPATH;

    #[pyfunction]
    fn fcntl(
        io::Fildes(fd): io::Fildes,
        cmd: i32,
        arg: OptionalArg<Either<ArgStrOrBytesLike, PyIntRef>>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let int = match arg {
            OptionalArg::Present(Either::A(arg)) => {
                let mut buf = [0u8; 1024];
                let arg_len;
                {
                    let s = arg.borrow_bytes();
                    arg_len = s.len();
                    buf.get_mut(..arg_len)
                        .ok_or_else(|| vm.new_value_error("fcntl string arg too long".to_owned()))?
                        .copy_from_slice(&*s)
                }
                let ret = unsafe { libc::fcntl(fd, cmd, buf.as_mut_ptr()) };
                if ret < 0 {
                    return Err(os::errno_err(vm));
                }
                return Ok(vm.ctx.new_bytes(buf[..arg_len].to_vec()).into());
            }
            OptionalArg::Present(Either::B(i)) => i.as_u32_mask(),
            OptionalArg::Missing => 0,
        };
        let ret = unsafe { libc::fcntl(fd, cmd, int as i32) };
        if ret < 0 {
            return Err(os::errno_err(vm));
        }
        Ok(vm.new_pyobj(ret))
    }

    #[pyfunction]
    fn ioctl(
        fd: i32,
        request: i32,
        arg: OptionalArg<Either<Either<ArgMemoryBuffer, ArgStrOrBytesLike>, i32>>,
        mutate_flag: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let arg = arg.unwrap_or_else(|| Either::B(0));
        match arg {
            Either::A(buf_kind) => {
                const BUF_SIZE: usize = 1024;
                let mut buf = [0u8; BUF_SIZE + 1]; // nul byte
                let mut fill_buf = |b: &[u8]| {
                    if b.len() > BUF_SIZE {
                        return Err(vm.new_value_error("fcntl string arg too long".to_owned()));
                    }
                    buf[..b.len()].copy_from_slice(b);
                    Ok(b.len())
                };
                let buf_len = match buf_kind {
                    Either::A(rw_arg) => {
                        let mutate_flag = mutate_flag.unwrap_or(true);
                        let mut arg_buf = rw_arg.borrow_buf_mut();
                        if mutate_flag {
                            let ret =
                                unsafe { libc::ioctl(fd, request as _, arg_buf.as_mut_ptr()) };
                            if ret < 0 {
                                return Err(os::errno_err(vm));
                            }
                            return Ok(vm.ctx.new_int(ret).into());
                        }
                        // treat like an immutable buffer
                        fill_buf(&arg_buf)?
                    }
                    Either::B(ro_buf) => fill_buf(&ro_buf.borrow_bytes())?,
                };
                let ret = unsafe { libc::ioctl(fd, request as _, buf.as_mut_ptr()) };
                if ret < 0 {
                    return Err(os::errno_err(vm));
                }
                Ok(vm.ctx.new_bytes(buf[..buf_len].to_vec()).into())
            }
            Either::B(i) => {
                let ret = unsafe { libc::ioctl(fd, request as _, i) };
                if ret < 0 {
                    return Err(os::errno_err(vm));
                }
                Ok(vm.ctx.new_int(ret).into())
            }
        }
    }

    // XXX: at the time of writing, wasi and redox don't have the necessary constants/function
    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn flock(fd: i32, operation: i32, vm: &VirtualMachine) -> PyResult {
        let ret = unsafe { libc::flock(fd, operation) };
        // TODO: add support for platforms that don't have a builtin `flock` syscall
        if ret < 0 {
            return Err(os::errno_err(vm));
        }
        Ok(vm.ctx.new_int(ret).into())
    }

    // XXX: at the time of writing, wasi and redox don't have the necessary constants
    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn lockf(
        fd: i32,
        cmd: i32,
        len: OptionalArg<PyIntRef>,
        start: OptionalArg<PyIntRef>,
        whence: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut l: libc::flock = unsafe { std::mem::zeroed() };
        if cmd == libc::LOCK_UN {
            l.l_type = libc::F_UNLCK
                .try_into()
                .map_err(|e| vm.new_overflow_error(format!("{e}")))?;
        } else if (cmd & libc::LOCK_SH) != 0 {
            l.l_type = libc::F_RDLCK
                .try_into()
                .map_err(|e| vm.new_overflow_error(format!("{e}")))?;
        } else if (cmd & libc::LOCK_EX) != 0 {
            l.l_type = libc::F_WRLCK
                .try_into()
                .map_err(|e| vm.new_overflow_error(format!("{e}")))?;
        } else {
            return Err(vm.new_value_error("unrecognized lockf argument".to_owned()));
        }
        l.l_start = match start {
            OptionalArg::Present(s) => s.try_to_primitive(vm)?,
            OptionalArg::Missing => 0,
        };
        l.l_len = match len {
            OptionalArg::Present(l_) => l_.try_to_primitive(vm)?,
            OptionalArg::Missing => 0,
        };
        l.l_whence = match whence {
            OptionalArg::Present(w) => w
                .try_into()
                .map_err(|e| vm.new_overflow_error(format!("{e}")))?,
            OptionalArg::Missing => 0,
        };
        let ret = unsafe {
            libc::fcntl(
                fd,
                if (cmd & libc::LOCK_NB) != 0 {
                    libc::F_SETLK
                } else {
                    libc::F_SETLKW
                },
                &l,
            )
        };
        if ret < 0 {
            return Err(os::errno_err(vm));
        }
        Ok(vm.ctx.new_int(ret).into())
    }
}

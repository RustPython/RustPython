// spell-checker:disable

pub(crate) use fcntl::module_def;

#[pymodule]
mod fcntl {
    use rustpython_host_env::fcntl as host_fcntl;

    use crate::vm::{
        PyResult, VirtualMachine,
        convert::ToPyException,
        function::{ArgIndex, ArgMemoryBuffer, ArgStrOrBytesLike, Either, OptionalArg},
        stdlib::_io,
    };
    use rustpython_host_env::io::is_interrupted_error;

    fn retry_on_eintr<T, E: ToPyException>(
        vm: &VirtualMachine,
        mut call: impl FnMut() -> Result<T, E>,
        interrupted: impl Fn(&E) -> bool,
    ) -> PyResult<T> {
        loop {
            match vm.allow_threads(&mut call) {
                Ok(val) => return Ok(val),
                Err(err) if interrupted(&err) => vm.check_signals()?,
                Err(err) => return Err(err.to_pyexception(vm)),
            }
        }
    }

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
    use host_fcntl::{F_GETFD, F_GETFL, F_SETFD, F_SETFL, FD_CLOEXEC};

    #[cfg(not(target_os = "wasi"))]
    #[pyattr]
    use host_fcntl::{F_DUPFD, F_DUPFD_CLOEXEC, F_GETLK, F_SETLK, F_SETLKW};

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyattr]
    use host_fcntl::{
        F_GETOWN, F_RDLCK, F_SETOWN, F_UNLCK, F_WRLCK, LOCK_EX, LOCK_NB, LOCK_SH, LOCK_UN,
    };

    #[cfg(target_vendor = "apple")]
    #[pyattr]
    use host_fcntl::{F_FULLFSYNC, F_NOCACHE};

    #[cfg(target_os = "freebsd")]
    #[pyattr]
    use host_fcntl::{F_DUP2FD, F_DUP2FD_CLOEXEC};

    #[cfg(any(target_os = "android", target_os = "linux", target_vendor = "apple"))]
    #[pyattr]
    use host_fcntl::{F_OFD_GETLK, F_OFD_SETLK, F_OFD_SETLKW};

    #[cfg(not(target_os = "wasi"))]
    #[pyattr]
    use host_fcntl::FASYNC;

    #[cfg(target_vendor = "apple")]
    #[pyattr]
    use host_fcntl::{F_GETLEASE, F_GETNOSIGPIPE, F_RDAHEAD, F_SETLEASE, F_SETNOSIGPIPE};

    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    #[pyattr]
    use host_fcntl::{
        F_ADD_SEALS, F_GET_SEALS, F_GETLEASE, F_GETPIPE_SZ, F_NOTIFY, F_SEAL_GROW, F_SEAL_SEAL,
        F_SEAL_SHRINK, F_SEAL_WRITE, F_SETLEASE, F_SETPIPE_SZ,
    };

    #[cfg(any(target_os = "dragonfly", target_os = "netbsd", target_vendor = "apple"))]
    #[pyattr]
    use host_fcntl::F_GETPATH;

    #[cfg(target_os = "linux")]
    #[pyattr]
    use host_fcntl::{FICLONE, FICLONERANGE};

    const BUFFER_SIZE: usize = 1024;
    const BUFFER_GUARD: [u8; 8] = [0, 0xfa, 0x69, 0xc4, 0x67, 0xa3, 0x6c, 0x58];

    struct SyscallBuffer {
        stack: [u8; BUFFER_SIZE + BUFFER_GUARD.len()],
        heap: Option<Vec<u8>>,
        len: usize,
    }

    impl SyscallBuffer {
        fn new(bytes: &[u8], vm: &VirtualMachine) -> PyResult<Self> {
            let len = bytes.len();
            let mut buffer = Self {
                stack: [0; BUFFER_SIZE + BUFFER_GUARD.len()],
                heap: None,
                len,
            };
            if len > BUFFER_SIZE {
                let size = len.checked_add(1).ok_or_else(|| vm.no_memory_error())?;
                let mut heap = vm.new_zeroed_bytes(size)?;
                heap[..len].copy_from_slice(bytes);
                buffer.heap = Some(heap);
            } else {
                buffer.stack[..len].copy_from_slice(bytes);
                buffer.stack[len..len + BUFFER_GUARD.len()].copy_from_slice(&BUFFER_GUARD);
            }
            Ok(buffer)
        }

        fn as_mut_slice(&mut self) -> &mut [u8] {
            self.heap.as_deref_mut().unwrap_or(&mut self.stack)
        }

        fn into_bytes(self, operation: &str, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            // Like CPython, detect writes into the trailing guard. This cannot
            // make a buffer safe for an operation requiring an arbitrary size.
            let valid = match &self.heap {
                Some(heap) => heap[self.len] == 0,
                None => self.stack[self.len..self.len + BUFFER_GUARD.len()] == BUFFER_GUARD,
            };
            if !valid {
                let corruption = if self.heap.is_some() {
                    "Memory corruption"
                } else {
                    "Possible stack corruption"
                };
                return Err(vm.new_system_error(format!(
                    "{corruption} in {operation}() due to buffer overflow. \
                     Provide an argument of sufficient size as determined by the operation."
                )));
            }
            match self.heap {
                Some(mut heap) => {
                    heap.truncate(self.len);
                    Ok(heap)
                }
                None => Ok(self.stack[..self.len].to_vec()),
            }
        }
    }

    #[derive(FromArgs)]
    struct FcntlArg {
        #[pyarg(positional, default = 0)]
        arg: Either<ArgStrOrBytesLike, ArgIndex>,
    }

    #[pyfunction]
    fn fcntl(
        _io::Fildes(fd): _io::Fildes,
        cmd: i32,
        FcntlArg { arg }: FcntlArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        let int = match arg {
            Either::A(arg) => {
                let mut buf = SyscallBuffer::new(&arg.borrow_bytes(), vm)?;
                retry_on_eintr(
                    vm,
                    || host_fcntl::fcntl_with_bytes(fd, cmd, buf.as_mut_slice()),
                    is_interrupted_error,
                )?;
                return Ok(vm.ctx.new_bytes(buf.into_bytes("fcntl", vm)?).into());
            }
            Either::B(i) => i.as_ref().as_u32_mask(),
        };
        let ret = retry_on_eintr(
            vm,
            || host_fcntl::fcntl_int(fd, cmd, int as i32),
            is_interrupted_error,
        )?;
        Ok(vm.new_pyobj(ret))
    }

    #[derive(FromArgs)]
    struct IoctlArgs {
        #[pyarg(positional, default = 0)]
        arg: Either<Either<ArgMemoryBuffer, ArgStrOrBytesLike>, i32>,
        #[pyarg(positional, default = true)]
        mutate_flag: bool,
    }

    #[pyfunction]
    fn ioctl(
        _io::Fildes(fd): _io::Fildes,
        request: i64,
        IoctlArgs { arg, mutate_flag }: IoctlArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mutate_flag = OptionalArg::Present(mutate_flag);
        let request = host_fcntl::normalize_ioctl_request(request);
        match arg {
            Either::A(buf_kind) => {
                let mut buf = match buf_kind {
                    Either::A(rw_arg) => {
                        let mutate_flag = mutate_flag.unwrap_or(true);
                        if mutate_flag {
                            // A terminal or a socket answers an ioctl when it is
                            // ready to, so the call runs detached, and the target's
                            // bytes go in and come back through a buffer of our own
                            // rather than stay locked meanwhile -- `fcntl_ioctl_impl`
                            // copies through one the same way.
                            let mut scratch = vm.new_zeroed_bytes(rw_arg.len())?;
                            scratch.copy_from_slice(&rw_arg.borrow_buf_mut());
                            let ret = retry_on_eintr(
                                vm,
                                || unsafe {
                                    host_fcntl::ioctl_ptr(fd, request, scratch.as_mut_ptr().cast())
                                },
                                is_interrupted_error,
                            )?;
                            rw_arg.borrow_buf_mut().copy_from_slice(&scratch);
                            return Ok(vm.ctx.new_int(ret).into());
                        }
                        // treat like an immutable buffer
                        SyscallBuffer::new(&rw_arg.borrow_buf_mut(), vm)?
                    }
                    Either::B(ro_buf) => SyscallBuffer::new(&ro_buf.borrow_bytes(), vm)?,
                };
                retry_on_eintr(
                    vm,
                    || unsafe {
                        host_fcntl::ioctl_ptr(fd, request, buf.as_mut_slice().as_mut_ptr().cast())
                    },
                    is_interrupted_error,
                )?;
                Ok(vm.ctx.new_bytes(buf.into_bytes("ioctl", vm)?).into())
            }
            Either::B(i) => {
                let ret = retry_on_eintr(
                    vm,
                    || host_fcntl::ioctl_int(fd, request, i),
                    is_interrupted_error,
                )?;
                Ok(vm.ctx.new_int(ret).into())
            }
        }
    }

    // XXX: at the time of writing, wasi and redox don't have the necessary constants/function
    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn flock(_io::Fildes(fd): _io::Fildes, operation: i32, vm: &VirtualMachine) -> PyResult {
        // LOCK_EX without LOCK_NB waits for whoever holds the lock, which may
        // be for good.
        let ret = retry_on_eintr(
            vm,
            || host_fcntl::flock(fd, operation),
            is_interrupted_error,
        )?;
        Ok(vm.ctx.new_int(ret).into())
    }

    #[derive(FromArgs)]
    struct LockfArgs {
        #[pyarg(positional, default = 0)]
        len: ArgIndex,
        #[pyarg(positional, default = 0)]
        start: ArgIndex,
        #[pyarg(positional, default)]
        whence: i32,
    }

    // XXX: at the time of writing, wasi and redox don't have the necessary constants
    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyfunction]
    fn lockf(
        _io::Fildes(fd): _io::Fildes,
        cmd: i32,
        LockfArgs { len, start, whence }: LockfArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let start = start.as_ref().try_to_primitive(vm)?;
        let len = len.as_ref().try_to_primitive(vm)?;
        // F_LOCK and F_TLOCK differ in exactly this: the first one waits.
        let ret = retry_on_eintr(
            vm,
            || host_fcntl::lockf(fd, cmd, len, start, whence),
            |err| match err {
                host_fcntl::LockfError::Io(e) => is_interrupted_error(e),
                _ => false,
            },
        )?;
        Ok(vm.ctx.new_int(ret).into())
    }
}

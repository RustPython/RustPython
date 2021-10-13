pub(crate) use fcntl::make_module;

#[pymodule]
mod fcntl {
    use crate::vm::{
        builtins::PyIntRef,
        function::{ArgMemoryBuffer, ArgStrOrBytesLike, OptionalArg},
        stdlib::{io, os},
        utils::Either,
        PyResult, VirtualMachine,
    };

    #[pyattr]
    use libc::{FD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL};

    #[cfg(not(target_os = "wasi"))]
    #[pyattr]
    use libc::{F_DUPFD, F_DUPFD_CLOEXEC, F_GETLK, F_SETLK, F_SETLKW};

    #[cfg(not(any(target_os = "wasi", target_os = "redox")))]
    #[pyattr]
    use libc::{F_GETOWN, F_RDLCK, F_SETOWN, F_UNLCK, F_WRLCK};

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
}

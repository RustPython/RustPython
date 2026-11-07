// spell-checker:disable

pub(crate) use self::termios::module_def;

#[pymodule]
mod termios {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytes, PyInt, PyList, PyListRef, PyTypeRef},
        convert::ToPyObject,
        stdlib::_io::Fildes,
    };
    use rustpython_host_env::{os::ErrorExt, termios as host_termios};

    // TODO: more ioctl numbers
    // NOTE: B2500000, B3000000, B3500000, B4000000, and CIBAUD
    //       are only available on specific architectures
    // TODO: supply these from <sys/ttydefaults.h> (please file an issue/PR upstream):
    //       CDEL, CDSUSP, CEOF, CEOL, CEOL2, CEOT, CERASE, CESC, CFLUSH, CINTR, CKILL, CLNEXT,
    //       CNUL, COMMON, CQUIT, CRPRNT, CSTART, CSTOP, CSUSP, CSWTCH, CWERASE
    // NOTE: the constant INIT_C_CC is most likely missing nowadays
    // TODO: supply these from <asm-generic/ioctl.h> (please file an issue/PR upstream):
    //       IOCSIZE_MASK, IOCSIZE_SHIFT
    // TODO: supply NCC from <asm-generic/termios.h> (please file an issue/PR upstream)
    // NOTE: I have only found NSWTCH on cygwin, so please alert the RustPython maintainers if it
    //       is present on your system
    // TODO: supply these from <bits/ioctl-types.h> or <linux/tty.h> (please file an issue/PR
    //       upstream):
    //       N_MOUSE, N_PPP, N_SLIP, N_STRIP, N_TTY
    // NOTE: some of these have incomplete coverage in rust libc (please file an issue/PR upstream)
    // TODO: possibly supply these from <asm-generic/ioctls.h> (please file an issue/PR upstream):
    //       TCSBRKP, TIOCGICOUNT, TIOCGLCKTRMIOS, TIOCSERCONFIG, TIOCSERGETLSR, TIOCSERGETMULTI,
    //       TIOCSERGSTRUCT, TIOCSERGWILD, TIOCSERSETMULTI, TIOCSERSWILD, TIOCSER_TEMT,
    //       TIOCSLCKTRMIOS, TIOCSSERIAL, TIOCTTYGSTRUCT
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "illumos",
        target_os = "linux",
        target_os = "macos",
        target_os = "openbsd",
        target_os = "solaris"
    ))]
    #[pyattr]
    use host_termios::TAB3;
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use host_termios::TCSASOFT;
    #[cfg(target_os = "macos")]
    #[pyattr]
    use host_termios::{
        _POSIX_VDISABLE, ALTWERASE, B7200, B14400, B28800, B76800, CIGNORE, EXTPROC, IUTF8, MDMBUF,
        NOKERNINFO, ONOEOT, OXTABS, VDSUSP, VSTATUS,
    };
    #[pyattr]
    use host_termios::{
        B0, B50, B75, B110, B134, B150, B200, B300, B600, B1200, B1800, B2400, B4800, B9600,
        B19200, B38400, B57600, B115200, B230400, BRKINT, CLOCAL, CREAD, CRTSCTS, CS5, CS6, CS7,
        CS8, CSIZE, CSTOPB, ECHO, ECHOCTL, ECHOE, ECHOK, ECHOKE, ECHONL, ECHOPRT, EXTA, EXTB,
        FLUSHO, HUPCL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, IGNPAR, IMAXBEL, INLCR, INPCK, ISIG,
        ISTRIP, IXANY, IXOFF, IXON, NCCS, NOFLSH, OCRNL, ONLCR, ONLRET, ONOCR, OPOST, PARENB,
        PARMRK, PARODD, PENDIN, TCIFLUSH, TCIOFF, TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON,
        TCSADRAIN, TCSAFLUSH, TCSANOW, TOSTOP, VDISCARD, VEOF, VEOL, VEOL2, VERASE, VINTR, VKILL,
        VLNEXT, VMIN, VQUIT, VREPRINT, VSTART, VSTOP, VSUSP, VTIME, VWERASE,
    };
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "illumos",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "solaris"
    ))]
    #[pyattr]
    use host_termios::{B460800, B921600};
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use host_termios::{
        B500000, B576000, B1000000, B1152000, B1500000, B2000000, B2500000, B3000000, B3500000,
        B4000000, CBAUDEX,
    };
    #[cfg(any(
        target_os = "android",
        target_os = "illumos",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris"
    ))]
    #[pyattr]
    use host_termios::{
        BS0, BS1, BSDLY, CR0, CR1, CR2, CR3, CRDLY, FF0, FF1, FFDLY, NL0, NL1, NLDLY, OFDEL, OFILL,
        TAB1, TAB2, VT0, VT1, VTDLY,
    };
    #[cfg(any(
        target_os = "android",
        target_os = "illumos",
        target_os = "linux",
        target_os = "solaris"
    ))]
    #[pyattr]
    use host_termios::{CBAUD, CIBAUD, IUCLC, OLCUC, XCASE};
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    #[pyattr]
    use host_termios::{CSTART, CSTOP, CSWTCH};
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use host_termios::{FIOASYNC, TIOCGETD, TIOCSETD};
    #[pyattr]
    use host_termios::{FIOCLEX, FIONBIO, TIOCGWINSZ, TIOCSWINSZ};
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
    use host_termios::{
        FIONCLEX, FIONREAD, TIOCEXCL, TIOCM_CAR, TIOCM_CD, TIOCM_CTS, TIOCM_DSR, TIOCM_DTR,
        TIOCM_LE, TIOCM_RI, TIOCM_RNG, TIOCM_RTS, TIOCM_SR, TIOCM_ST, TIOCMBIC, TIOCMBIS, TIOCMGET,
        TIOCMSET, TIOCNXCL, TIOCSCTTY,
    };
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use host_termios::{
        IBSHIFT, TCFLSH, TCGETA, TCGETS, TCSBRK, TCSETA, TCSETAF, TCSETAW, TCSETS, TCSETSF,
        TCSETSW, TCXONC, TIOCGSERIAL, TIOCGSOFTCAR, TIOCINQ, TIOCLINUX, TIOCSSOFTCAR, XTABS,
    };
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "illumos",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris"
    ))]
    #[pyattr]
    use host_termios::{TAB0, TABDLY};
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos"
    ))]
    #[pyattr]
    use host_termios::{TIOCCONS, TIOCGPGRP, TIOCOUTQ, TIOCSPGRP, TIOCSTI};
    #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    use host_termios::{
        TIOCNOTTY, TIOCPKT, TIOCPKT_DATA, TIOCPKT_DOSTOP, TIOCPKT_FLUSHREAD, TIOCPKT_FLUSHWRITE,
        TIOCPKT_NOSTOP, TIOCPKT_START, TIOCPKT_STOP,
    };
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use host_termios::{VSWTC, VSWTC as VSWTCH};
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    #[pyattr]
    use host_termios::{VSWTCH, VSWTCH as VSWTC};

    #[pyfunction]
    fn tcgetattr(Fildes(fd): Fildes, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let termios = host_termios::tcgetattr(fd).map_err(|e| termios_error(e, vm))?;
        let noncanon = (termios.c_lflag & host_termios::ICANON) == 0;
        let cc = termios
            .c_cc
            .iter()
            .enumerate()
            .map(|(i, &c)| match i {
                host_termios::VMIN | host_termios::VTIME if noncanon => vm.ctx.new_int(c).into(),
                _ => vm.ctx.new_bytes(vec![c as _]).into(),
            })
            .collect::<Vec<_>>();
        let out = vec![
            termios.c_iflag.to_pyobject(vm),
            termios.c_oflag.to_pyobject(vm),
            termios.c_cflag.to_pyobject(vm),
            termios.c_lflag.to_pyobject(vm),
            host_termios::cfgetispeed(&termios).to_pyobject(vm),
            host_termios::cfgetospeed(&termios).to_pyobject(vm),
            vm.ctx.new_list(cc).into(),
        ];
        Ok(out)
    }

    #[pyfunction]
    fn tcsetattr(
        Fildes(fd): Fildes,
        when: i32,
        attributes: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let attributes = attributes
            .downcast::<PyList>()
            .ok()
            .filter(|attributes| attributes.borrow_vec().len() == 7)
            .ok_or_else(|| vm.new_type_error("tcsetattr, arg 3: must be 7 element list"))?;
        let mut termios = host_termios::tcgetattr(fd).map_err(|e| termios_error(e, vm))?;

        // Read each item after converting the previous one: __index__ can
        // replace later elements of the attributes list.
        let long_at = |index| -> PyResult<libc::c_long> {
            list_item(&attributes, index, vm)?
                .try_index(vm)?
                .try_to_primitive(vm)
        };
        termios.c_iflag = long_at(0)? as _;
        termios.c_oflag = long_at(1)? as _;
        termios.c_cflag = long_at(2)? as _;
        termios.c_lflag = long_at(3)? as _;
        let ispeed = long_at(4)?;
        let ospeed = long_at(5)?;

        let cc = list_item(&attributes, 6, vm)?
            .downcast::<PyList>()
            .ok()
            .filter(|cc| cc.borrow_vec().len() == NCCS)
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "tcsetattr: attributes[6] must be {NCCS} element list"
                ))
            })?;
        for (cc, x) in termios.c_cc.iter_mut().zip(cc.borrow_vec().iter()) {
            *cc = if let Some(c) = x
                .downcast_ref::<PyBytes>()
                .filter(|b| b.as_bytes().len() == 1)
            {
                c.as_bytes()[0] as _
            } else if let Some(i) = x.downcast_ref::<PyInt>() {
                // CPython checks C long range, then casts to cc_t. Negative
                // values and values larger than a byte therefore wrap.
                i.try_to_primitive::<libc::c_long>(vm)? as _
            } else {
                return Err(vm.new_type_error(
                    "tcsetattr: elements of attributes must be bytes objects of length 1 or integers",
                ));
            };
        }

        // Validate all Python values before asking libc to validate the speeds.
        host_termios::cfsetispeed(&mut termios, ispeed as _).map_err(|e| termios_error(e, vm))?;
        host_termios::cfsetospeed(&mut termios, ospeed as _).map_err(|e| termios_error(e, vm))?;

        host_termios::tcsetattr(fd, when, &termios).map_err(|e| termios_error(e, vm))?;

        Ok(())
    }

    #[pyfunction]
    fn tcsendbreak(Fildes(fd): Fildes, duration: i32, vm: &VirtualMachine) -> PyResult<()> {
        host_termios::tcsendbreak(fd, duration).map_err(|e| termios_error(e, vm))?;
        Ok(())
    }

    #[pyfunction]
    fn tcdrain(Fildes(fd): Fildes, vm: &VirtualMachine) -> PyResult<()> {
        host_termios::tcdrain(fd).map_err(|e| termios_error(e, vm))?;
        Ok(())
    }

    #[pyfunction]
    fn tcflush(Fildes(fd): Fildes, queue: i32, vm: &VirtualMachine) -> PyResult<()> {
        host_termios::tcflush(fd, queue).map_err(|e| termios_error(e, vm))?;
        Ok(())
    }

    #[pyfunction]
    fn tcflow(Fildes(fd): Fildes, action: i32, vm: &VirtualMachine) -> PyResult<()> {
        host_termios::tcflow(fd, action).map_err(|e| termios_error(e, vm))?;
        Ok(())
    }

    #[pyfunction]
    fn tcgetwinsize(Fildes(fd): Fildes, vm: &VirtualMachine) -> PyResult<(u16, u16)> {
        let size = host_termios::tcgetwinsize(fd).map_err(|e| termios_error(e, vm))?;
        Ok(size)
    }

    #[pyfunction]
    fn tcsetwinsize(Fildes(fd): Fildes, winsize: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let seq = winsize.sequence_unchecked();
        if !seq.check() || seq.length(vm).ok() != Some(2) {
            return Err(vm.new_type_error("tcsetwinsize, arg 2: must be a two-item sequence"));
        }
        // A row's __index__ may change the next item in the sequence.
        let row: libc::c_long = seq.get_item(0, vm)?.try_index(vm)?.try_to_primitive(vm)?;
        let col: libc::c_long = seq.get_item(1, vm)?.try_index(vm)?.try_to_primitive(vm)?;

        host_termios::tcsetwinsize(fd, row, col).map_err(|e| match e {
            host_termios::SetWinSizeError::Io(e) => termios_error(e, vm),
            host_termios::SetWinSizeError::OutOfRange => {
                vm.new_overflow_error("winsize value(s) out of range.")
            }
        })?;
        Ok(())
    }

    fn list_item(list: &PyListRef, index: usize, vm: &VirtualMachine) -> PyResult {
        list.borrow_vec()
            .get(index)
            .cloned()
            .ok_or_else(|| vm.new_index_error("list index out of range"))
    }

    fn termios_error(err: std::io::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception(
            error_type(vm),
            vec![
                vm.ctx.new_int(err.posix_errno()).into(),
                vm.ctx.new_str(err.to_string()).into(),
            ],
        )
    }

    #[pyattr(name = "error", once)]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type("termios", "error", None)
    }
}

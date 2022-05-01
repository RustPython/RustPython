pub(crate) use self::termios::make_module;

#[pymodule]
mod termios {
    use crate::vm::{
        builtins::{PyBaseExceptionRef, PyBytes, PyInt, PyListRef, PyTypeRef},
        convert::ToPyObject,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use termios::Termios;

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
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    #[pyattr]
    use libc::{CSTART, CSTOP, CSWTCH};
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use libc::{FIOASYNC, TIOCGETD, TIOCSETD};
    #[pyattr]
    use libc::{FIOCLEX, FIONBIO, TIOCGWINSZ, TIOCSWINSZ};
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
        FIONCLEX, FIONREAD, TIOCEXCL, TIOCMBIC, TIOCMBIS, TIOCMGET, TIOCMSET, TIOCM_CAR, TIOCM_CD,
        TIOCM_CTS, TIOCM_DSR, TIOCM_DTR, TIOCM_LE, TIOCM_RI, TIOCM_RNG, TIOCM_RTS, TIOCM_SR,
        TIOCM_ST, TIOCNXCL, TIOCSCTTY,
    };
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use libc::{
        IBSHIFT, TCFLSH, TCGETA, TCGETS, TCSBRK, TCSETA, TCSETAF, TCSETAW, TCSETS, TCSETSF,
        TCSETSW, TCXONC, TIOCGSERIAL, TIOCGSOFTCAR, TIOCINQ, TIOCLINUX, TIOCSSOFTCAR, XTABS,
    };
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "macos"
    ))]
    #[pyattr]
    use libc::{TIOCCONS, TIOCGPGRP, TIOCOUTQ, TIOCSPGRP, TIOCSTI};
    #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "macos"))]
    #[pyattr]
    use libc::{
        TIOCNOTTY, TIOCPKT, TIOCPKT_DATA, TIOCPKT_DOSTOP, TIOCPKT_FLUSHREAD, TIOCPKT_FLUSHWRITE,
        TIOCPKT_NOSTOP, TIOCPKT_START, TIOCPKT_STOP,
    };
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
    use termios::os::target::TAB3;
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    #[pyattr]
    use termios::os::target::TCSASOFT;
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use termios::os::target::{
        B1000000, B1152000, B1500000, B2000000, B2500000, B3000000, B3500000, B4000000, B500000,
        B576000, CBAUDEX,
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
    use termios::os::target::{B460800, B921600};
    #[cfg(any(
        target_os = "android",
        target_os = "illumos",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris"
    ))]
    #[pyattr]
    use termios::os::target::{
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
    use termios::os::target::{CBAUD, CIBAUD, IUCLC, OLCUC, XCASE};
    #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "illumos",
        target_os = "linux",
        target_os = "macos",
        target_os = "solaris"
    ))]
    #[pyattr]
    use termios::os::target::{TAB0, TABDLY};
    #[cfg(any(target_os = "android", target_os = "linux"))]
    #[pyattr]
    use termios::os::target::{VSWTC, VSWTC as VSWTCH};
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    #[pyattr]
    use termios::os::target::{VSWTCH, VSWTCH as VSWTC};
    #[pyattr]
    use termios::{
        os::target::{
            B115200, B230400, B57600, CRTSCTS, ECHOCTL, ECHOKE, ECHOPRT, EXTA, EXTB, FLUSHO,
            IMAXBEL, NCCS, PENDIN, VDISCARD, VEOL2, VLNEXT, VREPRINT, VWERASE,
        },
        B0, B110, B1200, B134, B150, B1800, B19200, B200, B2400, B300, B38400, B4800, B50, B600,
        B75, B9600, BRKINT, CLOCAL, CREAD, CS5, CS6, CS7, CS8, CSIZE, CSTOPB, ECHO, ECHOE, ECHOK,
        ECHONL, HUPCL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, IGNPAR, INLCR, INPCK, ISIG, ISTRIP,
        IXANY, IXOFF, IXON, NOFLSH, OCRNL, ONLCR, ONLRET, ONOCR, OPOST, PARENB, PARMRK, PARODD,
        TCIFLUSH, TCIOFF, TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON, TCSADRAIN, TCSAFLUSH, TCSANOW,
        TOSTOP, VEOF, VEOL, VERASE, VINTR, VKILL, VMIN, VQUIT, VSTART, VSTOP, VSUSP, VTIME,
    };

    #[pyfunction]
    fn tcgetattr(fd: i32, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let termios = Termios::from_fd(fd).map_err(|e| termios_error(e, vm))?;
        let noncanon = (termios.c_lflag & termios::ICANON) == 0;
        let cc = termios
            .c_cc
            .iter()
            .enumerate()
            .map(|(i, &c)| match i {
                termios::VMIN | termios::VTIME if noncanon => vm.ctx.new_int(c).into(),
                _ => vm.ctx.new_bytes(vec![c as u8]).into(),
            })
            .collect::<Vec<_>>();
        let out = vec![
            termios.c_iflag.to_pyobject(vm),
            termios.c_oflag.to_pyobject(vm),
            termios.c_cflag.to_pyobject(vm),
            termios.c_lflag.to_pyobject(vm),
            termios::cfgetispeed(&termios).to_pyobject(vm),
            termios::cfgetospeed(&termios).to_pyobject(vm),
            vm.ctx.new_list(cc).into(),
        ];
        Ok(out)
    }

    #[pyfunction]
    fn tcsetattr(fd: i32, when: i32, attributes: PyListRef, vm: &VirtualMachine) -> PyResult<()> {
        let [iflag, oflag, cflag, lflag, ispeed, ospeed, cc] =
            <&[PyObjectRef; 7]>::try_from(&*attributes.borrow_vec())
                .map_err(|_| {
                    vm.new_type_error("tcsetattr, arg 3: must be 7 element list".to_owned())
                })?
                .clone();
        let mut termios = Termios::from_fd(fd).map_err(|e| termios_error(e, vm))?;
        termios.c_iflag = iflag.try_into_value(vm)?;
        termios.c_oflag = oflag.try_into_value(vm)?;
        termios.c_cflag = cflag.try_into_value(vm)?;
        termios.c_lflag = lflag.try_into_value(vm)?;
        termios::cfsetispeed(&mut termios, ispeed.try_into_value(vm)?)
            .map_err(|e| termios_error(e, vm))?;
        termios::cfsetospeed(&mut termios, ospeed.try_into_value(vm)?)
            .map_err(|e| termios_error(e, vm))?;
        let cc = PyListRef::try_from_object(vm, cc)?;
        let cc = cc.borrow_vec();
        let cc = <&[PyObjectRef; NCCS]>::try_from(&*cc).map_err(|_| {
            vm.new_type_error(format!(
                "tcsetattr: attributes[6] must be {} element list",
                NCCS
            ))
        })?;
        for (cc, x) in termios.c_cc.iter_mut().zip(cc.iter()) {
            *cc = if let Some(c) = x.payload::<PyBytes>().filter(|b| b.as_bytes().len() == 1) {
                c.as_bytes()[0] as _
            } else if let Some(i) = x.payload::<PyInt>() {
                i.try_to_primitive(vm)?
            } else {
                return Err(vm.new_type_error(
                    "tcsetattr: elements of attributes must be characters or integers".to_owned(),
                ));
            };
        }

        termios::tcsetattr(fd, when, &termios).map_err(|e| termios_error(e, vm))?;

        Ok(())
    }

    fn termios_error(err: std::io::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception(
            error_type(vm),
            vec![
                err.raw_os_error().to_pyobject(vm),
                vm.ctx.new_str(err.to_string()).into(),
            ],
        )
    }

    #[pyattr(name = "error", once)]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "termios",
            "error",
            Some(vec![vm.ctx.exceptions.os_error.clone()]),
        )
    }
}

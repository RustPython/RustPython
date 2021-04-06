pub(crate) use self::termios::make_module;

#[pymodule]
mod termios {
    use crate::builtins::{int, PyBytes, PyInt, PyListRef, PyTypeRef};
    use crate::common::borrow::BorrowValue;
    use crate::exceptions::PyBaseExceptionRef;
    use crate::pyobject::{IntoPyObject, PyObjectRef, PyResult, TryFromObject};
    use crate::VirtualMachine;
    use std::convert::TryFrom;
    use termios::Termios;

    #[pyattr]
    use termios::{
        os::target::NCCS, B0, B110, B1200, B134, B150, B1800, B19200, B200, B2400, B300, B38400,
        B4800, B50, B600, B75, B9600, BRKINT, CLOCAL, CREAD, CS5, CS6, CS7, CS8, CSIZE, CSTOPB,
        ECHO, ECHOE, ECHOK, ECHONL, HUPCL, ICANON, ICRNL, IEXTEN, IGNBRK, IGNCR, IGNPAR, INLCR,
        INPCK, ISIG, ISTRIP, IXANY, IXOFF, IXON, NOFLSH, OCRNL, ONLCR, ONLRET, ONOCR, OPOST,
        PARENB, PARMRK, PARODD, TCIFLUSH, TCIOFF, TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON,
        TCSADRAIN, TCSAFLUSH, TCSANOW, TOSTOP, VEOF, VEOL, VERASE, VINTR, VKILL, VMIN, VQUIT,
        VSTART, VSTOP, VSUSP, VTIME,
    };

    #[pyfunction]
    fn tcgetattr(fd: i32, vm: &VirtualMachine) -> PyResult {
        let termios = Termios::from_fd(fd).map_err(|e| termios_error(e, vm))?;
        let noncanon = (termios.c_lflag & termios::ICANON) == 0;
        let cc = termios
            .c_cc
            .iter()
            .enumerate()
            .map(|(i, &c)| match i {
                termios::VMIN | termios::VTIME if noncanon => vm.ctx.new_int(c),
                _ => vm.ctx.new_bytes(vec![c as u8]),
            })
            .collect::<Vec<_>>();
        let out = vec![
            termios.c_iflag.into_pyobject(vm),
            termios.c_oflag.into_pyobject(vm),
            termios.c_cflag.into_pyobject(vm),
            termios.c_lflag.into_pyobject(vm),
            termios::cfgetispeed(&termios).into_pyobject(vm),
            termios::cfgetospeed(&termios).into_pyobject(vm),
            vm.ctx.new_list(cc),
        ];
        Ok(vm.ctx.new_list(out))
    }

    #[pyfunction]
    fn tcsetattr(fd: i32, when: i32, attributes: PyListRef, vm: &VirtualMachine) -> PyResult<()> {
        let [iflag, oflag, cflag, lflag, ispeed, ospeed, cc] =
            <&[PyObjectRef; 7]>::try_from(&**attributes.borrow_value())
                .map_err(|_| {
                    vm.new_type_error("tcsetattr, arg 3: must be 7 element list".to_owned())
                })?
                .clone();
        let mut termios = Termios::from_fd(fd).map_err(|e| termios_error(e, vm))?;
        termios.c_iflag = TryFromObject::try_from_object(vm, iflag)?;
        termios.c_oflag = TryFromObject::try_from_object(vm, oflag)?;
        termios.c_cflag = TryFromObject::try_from_object(vm, cflag)?;
        termios.c_lflag = TryFromObject::try_from_object(vm, lflag)?;
        termios::cfsetispeed(&mut termios, TryFromObject::try_from_object(vm, ispeed)?)
            .map_err(|e| termios_error(e, vm))?;
        termios::cfsetospeed(&mut termios, TryFromObject::try_from_object(vm, ospeed)?)
            .map_err(|e| termios_error(e, vm))?;
        let cc = PyListRef::try_from_object(vm, cc)?;
        {
            let cc = cc.borrow_value();
            let cc = <&[PyObjectRef; NCCS]>::try_from(&**cc).map_err(|_| {
                vm.new_type_error(format!(
                    "tcsetattr: attributes[6] must be {} element list",
                    NCCS
                ))
            })?;
            for (cc, x) in termios.c_cc.iter_mut().zip(cc.iter()) {
                *cc = if let Some(c) = x
                    .payload::<PyBytes>()
                    .filter(|b| b.borrow_value().len() == 1)
                {
                    c.borrow_value()[0] as _
                } else if let Some(i) = x.payload::<PyInt>() {
                    int::try_to_primitive(i.borrow_value(), vm)?
                } else {
                    return Err(vm.new_type_error(
                        "tcsetattr: elements of attributes must be characters or integers"
                            .to_owned(),
                    ));
                };
            }
        }

        termios::tcsetattr(fd, when, &termios).map_err(|e| termios_error(e, vm))?;

        Ok(())
    }

    fn termios_error(err: std::io::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception(
            get_termios_error(vm),
            vec![
                err.raw_os_error().into_pyobject(vm),
                vm.ctx.new_str(err.to_string()),
            ],
        )
    }

    #[pyattr(name = "error")]
    fn get_termios_error(vm: &VirtualMachine) -> PyTypeRef {
        rustpython_common::static_cell! {
            static TERMIOS_ERROR: PyTypeRef;
        }
        TERMIOS_ERROR
            .get_or_init(|| {
                vm.ctx.new_class(
                    "termios.error",
                    &vm.ctx.exceptions.os_error,
                    Default::default(),
                )
            })
            .clone()
    }
}

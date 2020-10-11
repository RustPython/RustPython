use crate::builtins::bytes::PyBytes;
use crate::builtins::pystr::PyStr;
use crate::exceptions::PyBaseExceptionRef;
use crate::pyobject::{BorrowValue, PyObjectRef, PyResult};
use crate::VirtualMachine;
use std::{fmt, io};

pub trait Write {
    type Error;
    fn write_fmt(&mut self, args: fmt::Arguments) -> Result<(), Self::Error>;
}

impl<W> Write for W
where
    W: io::Write,
{
    type Error = io::Error;
    fn write_fmt(&mut self, args: fmt::Arguments) -> io::Result<()> {
        <W as io::Write>::write_fmt(self, args)
    }
}

pub struct PyWriter<'vm>(pub PyObjectRef, pub &'vm VirtualMachine);

impl Write for PyWriter<'_> {
    type Error = PyBaseExceptionRef;
    fn write_fmt(&mut self, args: fmt::Arguments) -> Result<(), Self::Error> {
        let PyWriter(obj, vm) = self;
        vm.call_method(obj, "write", (args.to_string(),)).map(drop)
    }
}

pub fn file_readline(obj: &PyObjectRef, size: Option<usize>, vm: &VirtualMachine) -> PyResult {
    let args = size.map_or_else(Vec::new, |size| vec![vm.ctx.new_int(size)]);
    let ret = vm.call_method(obj, "readline", args)?;
    let eof_err = || {
        vm.new_exception(
            vm.ctx.exceptions.eof_error.clone(),
            vec![vm.ctx.new_str("EOF when reading a line".to_owned())],
        )
    };
    let ret = match_class!(match ret {
        s @ PyStr => {
            let sval = s.borrow_value();
            if sval.is_empty() {
                return Err(eof_err());
            }
            if let Some(nonl) = sval.strip_suffix('\n') {
                vm.ctx.new_str(nonl.to_owned())
            } else {
                s.into_object()
            }
        }
        b @ PyBytes => {
            let buf = b.borrow_value();
            if buf.is_empty() {
                return Err(eof_err());
            }
            if buf.last() == Some(&b'\n') {
                vm.ctx.new_bytes(buf[..buf.len() - 1].to_owned())
            } else {
                b.into_object()
            }
        }
        _ => return Err(vm.new_type_error("object.readline() returned non-string".to_owned())),
    });
    Ok(ret)
}

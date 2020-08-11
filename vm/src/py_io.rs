use crate::exceptions::PyBaseExceptionRef;
use crate::pyobject::PyObjectRef;
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
        vm.call_method(obj, "write", vec![vm.ctx.new_str(args.to_string())])
            .map(drop)
    }
}

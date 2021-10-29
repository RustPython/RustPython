use crate::{
    builtins::{PyBaseExceptionRef, PyBytes, PyStr},
    common::ascii,
    PyObject, PyObjectRef, PyResult, VirtualMachine,
};
use std::{fmt, io, ops};

pub trait Write {
    type Error;
    fn write_fmt(&mut self, args: fmt::Arguments) -> Result<(), Self::Error>;
}

#[repr(transparent)]
pub struct IoWriter<T>(pub T);

impl<T> IoWriter<T> {
    pub fn from_ref(x: &mut T) -> &mut Self {
        // SAFETY: IoWriter is repr(transparent) over T
        unsafe { &mut *(x as *mut T as *mut Self) }
    }
}

impl<T> ops::Deref for IoWriter<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T> ops::DerefMut for IoWriter<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<W> Write for IoWriter<W>
where
    W: io::Write,
{
    type Error = io::Error;
    fn write_fmt(&mut self, args: fmt::Arguments) -> io::Result<()> {
        <W as io::Write>::write_fmt(&mut self.0, args)
    }
}

impl Write for String {
    type Error = fmt::Error;
    fn write_fmt(&mut self, args: fmt::Arguments) -> fmt::Result {
        <String as fmt::Write>::write_fmt(self, args)
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

pub fn file_readline(obj: &PyObject, size: Option<usize>, vm: &VirtualMachine) -> PyResult {
    let args = size.map_or_else(Vec::new, |size| vec![vm.ctx.new_int(size).into()]);
    let ret = vm.call_method(obj, "readline", args)?;
    let eof_err = || {
        vm.new_exception(
            vm.ctx.exceptions.eof_error.clone(),
            vec![vm.ctx.new_str(ascii!("EOF when reading a line")).into()],
        )
    };
    let ret = match_class!(match ret {
        s @ PyStr => {
            let sval = s.as_str();
            if sval.is_empty() {
                return Err(eof_err());
            }
            if let Some(nonl) = sval.strip_suffix('\n') {
                vm.ctx.new_str(nonl).into()
            } else {
                s.into()
            }
        }
        b @ PyBytes => {
            let buf = b.as_bytes();
            if buf.is_empty() {
                return Err(eof_err());
            }
            if buf.last() == Some(&b'\n') {
                vm.ctx.new_bytes(buf[..buf.len() - 1].to_owned()).into()
            } else {
                b.into()
            }
        }
        _ => return Err(vm.new_type_error("object.readline() returned non-string".to_owned())),
    });
    Ok(ret)
}

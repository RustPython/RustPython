use core::fmt;

use rustpython_common::wtf8::{Wtf8, Wtf8Buf};

use crate::{
    PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyStr, PyUtf8Str},
    convert::{ToPyException, ToPyObject},
    exceptions::cstring_error,
};

pub fn hash_iter<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    vm.state.hash_secret.hash_iter(iter, |obj| obj.hash(vm))
}

impl ToPyObject for core::convert::Infallible {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {}
    }
}

pub trait ToCString: AsRef<Wtf8> {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<alloc::ffi::CString> {
        alloc::ffi::CString::new(self.as_ref().as_bytes()).map_err(|err| err.to_pyexception(vm))
    }
    fn ensure_no_nul(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.as_ref().as_bytes().contains(&b'\0') {
            Err(cstring_error(vm))
        } else {
            Ok(())
        }
    }
}

impl ToCString for &str {}
impl ToCString for PyStr {}
impl ToCString for PyUtf8Str {}

pub(crate) fn collection_repr<'a, I>(
    class_name: Option<&str>,
    prefix: &str,
    suffix: &str,
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<Wtf8Buf>
where
    I: core::iter::Iterator<Item = &'a PyObjectRef>,
{
    let mut repr = Wtf8Buf::new();
    if let Some(name) = class_name {
        repr.push_str(name);
        repr.push_char('(');
    }
    repr.push_str(prefix);
    {
        let mut parts_iter = iter.map(|o| o.repr(vm));
        let first = parts_iter
            .next()
            .transpose()?
            .expect("this is not called for empty collection");
        repr.push_wtf8(first.as_wtf8());
        for part in parts_iter {
            repr.push_str(", ");
            repr.push_wtf8(part?.as_wtf8());
        }
    }
    repr.push_str(suffix);
    if class_name.is_some() {
        repr.push_char(')');
    }

    Ok(repr)
}

/// Wrapper around a bytes vector that implements [`fmt::Write`].
///
/// # Safety
/// Don't assume the contents of the internal vector are valid UTF-8/WTF-8.
pub(crate) struct VecFmtWriter(pub Vec<u8>);

impl fmt::Write for VecFmtWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.extend(s.bytes());
        Ok(())
    }
}

/// Wrapper around a bytes slice that implements [`fmt::Write`].
///
/// # Errors
/// [`fmt::Error`] is returned if the string doesn't fit into the internal buffer. This
/// implementation writes as many bytes as can fit before returning an error.
/// Check [`Self::remainder`] if the amount of bytes that can be written is important.
pub(crate) struct SliceFmtWriter<'slice> {
    buf: &'slice mut [u8],
    written: usize,
}

impl<'slice> SliceFmtWriter<'slice> {
    pub(crate) const fn new(buf: &'slice mut [u8]) -> Self {
        Self { buf, written: 0 }
    }

    /// Amount of space left in the internal buffer.
    pub(crate) const fn remainder(&self) -> usize {
        self.buf.len() - self.written
    }

    /// Amount of bytes written to the internal buffer.
    pub(crate) const fn written(&self) -> usize {
        self.written
    }
}

impl fmt::Write for SliceFmtWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let to_copy = self.remainder().min(s.len());
        self.buf[self.written..self.written + to_copy].copy_from_slice(&s.as_bytes()[..to_copy]);
        self.written += to_copy;
        if to_copy == s.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

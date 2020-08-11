use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use std::mem::size_of;
use std::ops::Deref;

use super::objint::PyIntRef;
use super::objiter;
use super::objsequence::SequenceIndex;
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use crate::bytesinner::{
    ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
    ByteInnerTranslateOptions, PyBytesInner,
};
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject,
    PyArithmaticValue::{self, *},
    PyClassImpl, PyComparisonValue, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::pystr::{self, PyCommonString};
use crate::vm::VirtualMachine;
use rustpython_common::hash::PyHash;

/// "bytes(iterable_of_ints) -> bytes\n\
/// bytes(string, encoding[, errors]) -> bytes\n\
/// bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
/// bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
/// bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
/// - an iterable yielding integers in range(256)\n  \
/// - a text string encoded using the specified encoding\n  \
/// - any object implementing the buffer API.\n  \
/// - an integer";
#[pyclass(name = "bytes")]
#[derive(Clone, Debug)]
pub struct PyBytes {
    inner: PyBytesInner,
}

pub type PyBytesRef = PyRef<PyBytes>;

impl<'a> BorrowValue<'a> for PyBytes {
    type Borrowed = &'a [u8];

    fn borrow_value(&'a self) -> Self::Borrowed {
        &self.inner.elements
    }
}

impl From<Vec<u8>> for PyBytes {
    fn from(elements: Vec<u8>) -> Self {
        Self {
            inner: PyBytesInner { elements },
        }
    }
}

impl IntoPyObject for Vec<u8> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytes(self)
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl PyValue for PyBytes {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytes_type()
    }
}

pub(crate) fn init(context: &PyContext) {
    PyBytes::extend_class(context, &context.types.bytes_type);
    let bytes_type = &context.types.bytes_type;
    extend_class!(context, bytes_type, {
        "maketrans" => context.new_method(PyBytesInner::maketrans),
    });
    PyBytesIterator::extend_class(context, &context.types.bytesiterator_type);
}

#[pyimpl(flags(BASETYPE))]
impl PyBytes {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesRef> {
        PyBytes {
            inner: options.get_value(vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    pub(crate) fn repr(&self) -> String {
        format!("b'{}'", self.inner.repr())
    }

    #[pymethod(name = "__len__")]
    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.inner.eq(other, vm)
    }
    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.inner.ge(other, vm)
    }
    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.inner.le(other, vm)
    }
    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.inner.gt(other, vm)
    }
    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.inner.lt(other, vm)
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self) -> PyHash {
        self.inner.hash()
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyBytesIterator {
        PyBytesIterator {
            position: AtomicCell::new(0),
            bytes: zelf,
        }
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> PyResult<usize> {
        Ok(size_of::<Self>() + self.inner.elements.len() * size_of::<u8>())
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyArithmaticValue<PyBytes> {
        if let Ok(other) = PyBytesInner::try_from_object(vm, other) {
            Implemented(self.inner.add(other).into())
        } else {
            NotImplemented
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        self.inner.getitem(needle, vm)
    }

    #[pymethod(name = "isalnum")]
    fn isalnum(&self) -> bool {
        self.inner.isalnum()
    }

    #[pymethod(name = "isalpha")]
    fn isalpha(&self) -> bool {
        self.inner.isalpha()
    }

    #[pymethod(name = "isascii")]
    fn isascii(&self) -> bool {
        self.inner.isascii()
    }

    #[pymethod(name = "isdigit")]
    fn isdigit(&self) -> bool {
        self.inner.isdigit()
    }

    #[pymethod(name = "islower")]
    fn islower(&self) -> bool {
        self.inner.islower()
    }

    #[pymethod(name = "isspace")]
    fn isspace(&self) -> bool {
        self.inner.isspace()
    }

    #[pymethod(name = "isupper")]
    fn isupper(&self) -> bool {
        self.inner.isupper()
    }

    #[pymethod(name = "istitle")]
    fn istitle(&self) -> bool {
        self.inner.istitle()
    }

    #[pymethod(name = "lower")]
    fn lower(&self) -> Self {
        self.inner.lower().into()
    }

    #[pymethod(name = "upper")]
    fn upper(&self) -> Self {
        self.inner.upper().into()
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(&self) -> Self {
        self.inner.capitalize().into()
    }

    #[pymethod(name = "swapcase")]
    fn swapcase(&self) -> Self {
        self.inner.swapcase().into()
    }

    #[pymethod(name = "hex")]
    fn hex(&self) -> String {
        self.inner.hex()
    }

    #[pymethod]
    fn fromhex(string: PyStringRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(PyBytesInner::fromhex(string.borrow_value(), vm)?.into())
    }

    #[pymethod(name = "center")]
    fn center(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.center(options, vm)?.into())
    }

    #[pymethod(name = "ljust")]
    fn ljust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.ljust(options, vm)?.into())
    }

    #[pymethod(name = "rjust")]
    fn rjust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.rjust(options, vm)?.into())
    }

    #[pymethod(name = "count")]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.count(options, vm)
    }

    #[pymethod(name = "join")]
    fn join(&self, iter: PyIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.join(iter, vm)?.into())
    }

    #[pymethod(name = "endswith")]
    fn endswith(&self, options: pystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.inner.elements[..].py_startsendswith(
            options,
            "endswith",
            "bytes",
            |s, x: &PyBytesInner| s.ends_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod(name = "startswith")]
    fn startswith(
        &self,
        options: pystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.elements[..].py_startsendswith(
            options,
            "startswith",
            "bytes",
            |s, x: &PyBytesInner| s.starts_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod(name = "find")]
    fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod(name = "index")]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod(name = "rfind")]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod(name = "rindex")]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod(name = "translate")]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        Ok(self.inner.translate(options, vm)?.into())
    }

    #[pymethod(name = "strip")]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.strip(chars).into()
    }

    #[pymethod(name = "lstrip")]
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.lstrip(chars).into()
    }

    #[pymethod(name = "rstrip")]
    fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.rstrip(chars).into()
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a bytes object with the given prefix string removed if present.
    ///
    /// If the bytes starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original bytes.
    #[pymethod(name = "removeprefix")]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner.removeprefix(prefix).into()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a bytes object with the given suffix string removed if present.
    ///
    /// If the bytes ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original bytes.
    #[pymethod(name = "removesuffix")]
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
        self.inner.removesuffix(suffix).into()
    }

    #[pymethod(name = "split")]
    fn split(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner
            .split(options, |s, vm| vm.ctx.new_bytes(s.to_vec()), vm)
    }

    #[pymethod(name = "rsplit")]
    fn rsplit(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner
            .rsplit(options, |s, vm| vm.ctx.new_bytes(s.to_vec()), vm)
    }

    #[pymethod(name = "partition")]
    fn partition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sub = PyBytesInner::try_from_object(vm, sep.clone())?;
        let (front, has_mid, back) = self.inner.partition(&sub, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new())
            },
            vm.ctx.new_bytes(back),
        ]))
    }

    #[pymethod(name = "rpartition")]
    fn rpartition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sub = PyBytesInner::try_from_object(vm, sep.clone())?;
        let (back, has_mid, front) = self.inner.rpartition(&sub, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new())
            },
            vm.ctx.new_bytes(back),
        ]))
    }

    #[pymethod(name = "expandtabs")]
    fn expandtabs(&self, options: pystr::ExpandTabsArgs) -> Self {
        self.inner.expandtabs(options).into()
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(&self, options: pystr::SplitLinesArgs, vm: &VirtualMachine) -> PyResult {
        let lines = self
            .inner
            .splitlines(options, |x| vm.ctx.new_bytes(x.to_vec()));
        Ok(vm.ctx.new_list(lines))
    }

    #[pymethod(name = "zfill")]
    fn zfill(&self, width: isize) -> Self {
        self.inner.zfill(width).into()
    }

    #[pymethod(name = "replace")]
    fn replace(
        &self,
        old: PyBytesInner,
        new: PyBytesInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        Ok(self.inner.replace(old, new, count, vm)?.into())
    }

    #[pymethod(name = "title")]
    fn title(&self) -> Self {
        self.inner.title().into()
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<PyBytes> {
        if value > 0 && self.inner.len() as isize > std::isize::MAX / value {
            return Err(vm.new_overflow_error("repeated bytes are too long".to_owned()));
        }
        Ok(self.inner.repeat(value).into())
    }

    #[pymethod(name = "__mod__")]
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let formatted = self.inner.cformat(values, vm)?;
        Ok(vm.ctx.new_bytes(formatted.as_bytes().to_owned()))
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    /// Return a string decoded from the given bytes.
    /// Default encoding is 'utf-8'.
    /// Default errors is 'strict', meaning that encoding errors raise a UnicodeError.
    /// Other possible values are 'ignore', 'replace'
    /// For a list of possible encodings,
    /// see https://docs.python.org/3/library/codecs.html#standard-encodings
    /// currently, only 'utf-8' and 'ascii' emplemented
    #[pymethod(name = "decode")]
    fn decode(
        zelf: PyRef<Self>,
        encoding: OptionalArg<PyStringRef>,
        errors: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyStringRef> {
        let encoding = encoding.into_option();
        vm.decode(zelf.into_object(), encoding.clone(), errors.into_option())?
            .downcast::<PyString>()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                     encode arbitrary types",
                    encoding.as_ref().map_or("utf-8", |s| s.borrow_value()),
                    obj.lease_class().name,
                ))
            })
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyBytesIterator {
    position: AtomicCell<usize>,
    bytes: PyBytesRef,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytesiterator_type()
    }
}

#[pyimpl]
impl PyBytesIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<u8> {
        let pos = self.position.fetch_add(1);
        if let Some(&ret) = self.bytes.borrow_value().get(pos) {
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
}

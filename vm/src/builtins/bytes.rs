use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::borrow::{BorrowedValue, BorrowedValueMut};
use std::mem::size_of;
use std::ops::Deref;

use super::dict::PyDictRef;
use super::int::PyIntRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::builtins::tuple::PyTupleRef;
use crate::bytesinner::{
    bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
};
use crate::common::hash::PyHash;
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, Either, IntoPyObject, PyClassImpl, PyComparisonValue, PyContext, PyIterable,
    PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::slots::{BufferProtocol, Comparable, Hashable, Iterable, PyComparisonOp, PyIter};
use crate::vm::VirtualMachine;
use crate::{
    anystr::{self, AnyStr},
    byteslike::PyBytesLike,
};

use crate::builtins::memory::{Buffer, BufferOptions};

/// "bytes(iterable_of_ints) -> bytes\n\
/// bytes(string, encoding[, errors]) -> bytes\n\
/// bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
/// bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
/// bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
/// - an iterable yielding integers in range(256)\n  \
/// - a text string encoded using the specified encoding\n  \
/// - any object implementing the buffer API.\n  \
/// - an integer";
#[pyclass(module = false, name = "bytes")]
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

impl From<PyBytesInner> for PyBytes {
    fn from(inner: PyBytesInner) -> Self {
        Self { inner }
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
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytes_type
    }
}

pub(crate) fn init(context: &PyContext) {
    PyBytes::extend_class(context, &context.types.bytes_type);
    let bytes_type = &context.types.bytes_type;
    extend_class!(context, bytes_type, {
        "maketrans" => context.new_method("maketrans", PyBytesInner::maketrans),
    });
    PyBytesIterator::extend_class(context, &context.types.bytes_iterator_type);
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, BufferProtocol, Iterable))]
impl PyBytes {
    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        options.get_bytes(cls, vm)
    }

    #[pymethod(name = "__repr__")]
    pub(crate) fn repr(&self) -> String {
        format!("b'{}'", self.inner.repr())
    }

    #[pymethod(name = "__len__")]
    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.inner.elements.len() * size_of::<u8>()
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyBytesLike, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytes(self.inner.add(&*other.borrow_value()))
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
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.getitem("byte", needle, vm)
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
    pub(crate) fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.inner.hex(sep, bytes_per_sep, vm)
    }

    #[pymethod]
    fn fromhex(string: PyStrRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
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
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
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
        options: anystr::StartsEndsWithArgs,
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
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner.expandtabs(options).into()
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        let lines = self
            .inner
            .splitlines(options, |x| vm.ctx.new_bytes(x.to_vec()));
        vm.ctx.new_list(lines)
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
    fn modulo(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let formatted = self.inner.cformat(values, vm)?;
        Ok(formatted.into())
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
    #[pymethod]
    fn decode(zelf: PyRef<Self>, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(zelf.into_object(), args, vm)
    }

    #[pymethod(magic)]
    fn getnewargs(&self, vm: &VirtualMachine) -> PyTupleRef {
        let param: Vec<PyObjectRef> = self
            .inner
            .elements
            .iter()
            .map(|x| x.into_pyobject(vm))
            .collect();
        PyTupleRef::with_elements(param, &vm.ctx)
    }

    #[pymethod(magic)]
    fn reduce_ex(
        zelf: PyRef<Self>,
        _proto: usize,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        Self::reduce(zelf, vm)
    }

    #[pymethod(magic)]
    fn reduce(
        zelf: PyRef<Self>,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let bytes = PyBytes::from(zelf.inner.elements.clone()).into_pyobject(vm);
        (
            zelf.as_object().clone_class(),
            PyTupleRef::with_elements(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

impl BufferProtocol for PyBytes {
    fn get_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        let buf = BytesBuffer {
            bytes: zelf.clone(),
            options: BufferOptions {
                len: zelf.len(),
                ..Default::default()
            },
        };
        Ok(Box::new(buf))
    }
}

#[derive(Debug)]
struct BytesBuffer {
    bytes: PyBytesRef,
    options: BufferOptions,
}

impl Buffer for BytesBuffer {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.bytes.borrow_value().into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        unreachable!("bytes is not mutable")
    }

    fn release(&self) {}

    fn get_options(&self) -> &BufferOptions {
        &self.options
    }
}

impl Hashable for PyBytes {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.inner.hash(vm))
    }
}

impl Comparable for PyBytes {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Ok(if let Some(res) = op.identical_optimization(zelf, other) {
            res.into()
        } else if other.isinstance(&vm.ctx.types.memoryview_type)
            && op != PyComparisonOp::Eq
            && op != PyComparisonOp::Ne
        {
            return Err(vm.new_type_error(format!(
                "'{}' not supported between instances of '{}' and '{}'",
                op.operator_token(),
                zelf.class().name,
                other.class().name
            )));
        } else {
            zelf.inner.cmp(other, op, vm)
        })
    }
}

impl Iterable for PyBytes {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyBytesIterator {
            position: AtomicCell::new(0),
            bytes: zelf,
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "bytes_iterator")]
#[derive(Debug)]
pub struct PyBytesIterator {
    position: AtomicCell<usize>,
    bytes: PyBytesRef,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytes_iterator_type
    }
}

#[pyimpl(with(PyIter))]
impl PyBytesIterator {}
impl PyIter for PyBytesIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let pos = zelf.position.fetch_add(1);
        if let Some(&ret) = zelf.bytes.borrow_value().get(pos) {
            Ok(vm.ctx.new_int(ret))
        } else {
            Err(vm.new_stop_iteration())
        }
    }
}

impl TryFromObject for PyBytes {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyBytesInner::try_from_object(vm, obj).map(|x| x.into())
    }
}

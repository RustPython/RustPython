use super::{PositionIterInternal, PyDictRef, PyIntRef, PyStrRef, PyTuple, PyTupleRef, PyTypeRef};
use crate::{
    anystr::{self, AnyStr},
    builtins::PyType,
    bytesinner::{
        bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
    },
    common::{hash::PyHash, lock::PyMutex},
    function::{
        ArgBytesLike, ArgIterable, IntoPyObject, IntoPyResult, OptionalArg, OptionalOption,
    },
    protocol::{BufferDescriptor, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods},
    types::{
        AsBuffer, AsMapping, Callable, Comparable, Constructor, Hashable, IterNext,
        IterNextIterable, Iterable, PyComparisonOp, Unconstructible,
    },
    utils::Either,
    IdProtocol, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef, PyObjectView,
    PyObjectWrap, PyRef, PyResult, PyValue, TryFromBorrowedObject, TypeProtocol, VirtualMachine,
};
use bstr::ByteSlice;
use std::mem::size_of;
use std::ops::Deref;

#[pyclass(module = false, name = "bytes")]
#[derive(Clone, Debug)]
pub struct PyBytes {
    inner: PyBytesInner,
}

pub type PyBytesRef = PyRef<PyBytes>;

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
        vm.ctx.new_bytes(self).into()
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl AsRef<[u8]> for PyBytes {
    fn as_ref(&self) -> &[u8] {
        &self.inner.elements
    }
}
impl AsRef<[u8]> for PyBytesRef {
    fn as_ref(&self) -> &[u8] {
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
    PyBytesIterator::extend_class(context, &context.types.bytes_iterator_type);
}

impl Constructor for PyBytes {
    type Args = ByteInnerNewOptions;

    fn py_new(cls: PyTypeRef, options: Self::Args, vm: &VirtualMachine) -> PyResult {
        options.get_bytes(cls, vm).into_pyresult(vm)
    }
}

impl PyBytes {
    pub fn new_ref(data: Vec<u8>, ctx: &PyContext) -> PyRef<Self> {
        PyRef::new_ref(Self::from(data), ctx.types.bytes_type.clone(), None)
    }
}

#[pyimpl(
    flags(BASETYPE),
    with(AsMapping, Hashable, Comparable, AsBuffer, Iterable, Constructor)
)]
impl PyBytes {
    #[pymethod(magic)]
    pub(crate) fn repr(&self) -> String {
        self.inner.repr(None)
    }

    #[pymethod(magic)]
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner.elements
    }

    #[pymethod(magic)]
    fn bytes(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyRef<Self> {
        if zelf.is(&vm.ctx.types.bytes_type) {
            zelf
        } else {
            PyBytes::from(zelf.inner.clone()).into_ref(vm)
        }
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.inner.elements.len() * size_of::<u8>()
    }

    #[pymethod(magic)]
    fn add(&self, other: ArgBytesLike) -> Vec<u8> {
        self.inner.add(&*other.borrow_buf())
    }

    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    #[pymethod]
    fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(from, to, vm)
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.getitem("byte", needle, vm) // byte != Self::NAME
    }

    #[pymethod]
    fn isalnum(&self) -> bool {
        self.inner.isalnum()
    }

    #[pymethod]
    fn isalpha(&self) -> bool {
        self.inner.isalpha()
    }

    #[pymethod]
    fn isascii(&self) -> bool {
        self.inner.isascii()
    }

    #[pymethod]
    fn isdigit(&self) -> bool {
        self.inner.isdigit()
    }

    #[pymethod]
    fn islower(&self) -> bool {
        self.inner.islower()
    }

    #[pymethod]
    fn isspace(&self) -> bool {
        self.inner.isspace()
    }

    #[pymethod]
    fn isupper(&self) -> bool {
        self.inner.isupper()
    }

    #[pymethod]
    fn istitle(&self) -> bool {
        self.inner.istitle()
    }

    #[pymethod]
    fn lower(&self) -> Self {
        self.inner.lower().into()
    }

    #[pymethod]
    fn upper(&self) -> Self {
        self.inner.upper().into()
    }

    #[pymethod]
    fn capitalize(&self) -> Self {
        self.inner.capitalize().into()
    }

    #[pymethod]
    fn swapcase(&self) -> Self {
        self.inner.swapcase().into()
    }

    #[pymethod]
    pub(crate) fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.inner.hex(sep, bytes_per_sep, vm)
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex(string.as_str(), vm)?;
        let bytes = vm.ctx.new_bytes(bytes).into();
        PyType::call(&cls, vec![bytes].into(), vm)
    }

    #[pymethod]
    fn center(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.count(options, vm)
    }

    #[pymethod]
    fn join(&self, iter: ArgIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyBytes> {
        Ok(self.inner.join(iter, vm)?.into())
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(&self.inner.elements[..], self.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_startsendswith(
            affix,
            "endswith",
            "bytes",
            |s, x: &PyBytesInner| s.ends_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod]
    fn startswith(
        &self,
        options: anystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(&self.inner.elements[..], self.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_startsendswith(
            affix,
            "startswith",
            "bytes",
            |s, x: &PyBytesInner| s.starts_with(&x.elements[..]),
            vm,
        )
    }

    #[pymethod]
    fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        Ok(self.inner.translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.strip(chars).into()
    }

    #[pymethod]
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.lstrip(chars).into()
    }

    #[pymethod]
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
    #[pymethod]
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
    #[pymethod]
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
        self.inner.removesuffix(suffix).into()
    }

    #[pymethod]
    fn split(
        &self,
        options: ByteInnerSplitOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        self.inner
            .split(options, |s, vm| vm.ctx.new_bytes(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn rsplit(
        &self,
        options: ByteInnerSplitOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        self.inner
            .rsplit(options, |s, vm| vm.ctx.new_bytes(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn partition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let sub = PyBytesInner::try_from_borrowed_object(vm, &sep)?;
        let (front, has_mid, back) = self.inner.partition(&sub, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new()).into()
            },
            vm.ctx.new_bytes(back),
        )))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let sub = PyBytesInner::try_from_borrowed_object(vm, &sep)?;
        let (back, has_mid, front) = self.inner.rpartition(&sub, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytes(front),
            if has_mid {
                sep
            } else {
                vm.ctx.new_bytes(Vec::new()).into()
            },
            vm.ctx.new_bytes(back),
        )))
    }

    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner.expandtabs(options).into()
    }

    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        self.inner
            .splitlines(options, |x| vm.ctx.new_bytes(x.to_vec()).into())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> Self {
        self.inner.zfill(width).into()
    }

    #[pymethod]
    fn replace(
        &self,
        old: PyBytesInner,
        new: PyBytesInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        Ok(self.inner.replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn title(&self) -> Self {
        self.inner.title().into()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        if value == 1 && zelf.class().is(&vm.ctx.types.bytes_type) {
            // Special case: when some `bytes` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `bytes` itself, not its subclasses.
            return Ok(zelf);
        }
        zelf.inner
            .mul(value, vm)
            .map(|x| Self::from(x).into_ref(vm))
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let formatted = self.inner.cformat(values, vm)?;
        Ok(formatted.into())
    }

    #[pymethod(magic)]
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
        bytes_decode(zelf.into(), args, vm)
    }

    #[pymethod(magic)]
    fn getnewargs(&self, vm: &VirtualMachine) -> PyTupleRef {
        let param: Vec<PyObjectRef> = self
            .inner
            .elements
            .iter()
            .map(|x| x.into_pyobject(vm))
            .collect();
        PyTuple::new_ref(param, &vm.ctx)
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
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyBytes>().as_bytes().into(),
    obj_bytes_mut: |_| panic!(),
    release: |_| {},
    retain: |_| {},
};

impl AsBuffer for PyBytes {
    fn as_buffer(zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buf = PyBuffer::new(
            zelf.to_owned().into_object(),
            BufferDescriptor::simple(zelf.len(), true),
            &BUFFER_METHODS,
        );
        Ok(buf)
    }
}

impl AsMapping for PyBytes {
    fn as_mapping(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: None,
        }
    }

    #[inline]
    fn length(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        Self::downcast_ref(&zelf, vm).map(|zelf| Ok(zelf.len()))?
    }

    #[inline]
    fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::downcast_ref(&zelf, vm).map(|zelf| zelf.getitem(needle, vm))?
    }

    #[cold]
    fn ass_subscript(
        zelf: PyObjectRef,
        _needle: PyObjectRef,
        _value: Option<PyObjectRef>,
        _vm: &VirtualMachine,
    ) -> PyResult<()> {
        unreachable!("ass_subscript not implemented for {}", zelf.class())
    }
}

impl Hashable for PyBytes {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.inner.hash(vm))
    }
}

impl Comparable for PyBytes {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
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
                zelf.class().name(),
                other.class().name()
            )));
        } else {
            zelf.inner.cmp(other, op, vm)
        })
    }
}

impl Iterable for PyBytes {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyBytesIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_object(vm))
    }
}

#[pyclass(module = false, name = "bytes_iterator")]
#[derive(Debug)]
pub struct PyBytesIterator {
    internal: PyMutex<PositionIterInternal<PyBytesRef>>,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytes_iterator_type
    }
}

#[pyimpl(with(Constructor, IterNext))]
impl PyBytesIterator {
    #[pymethod(magic)]
    fn length_hint(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.len())
    }

    #[pymethod(magic)]
    fn reduce(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }

    #[pymethod(magic)]
    fn setstate(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }
}
impl Unconstructible for PyBytesIterator {}

impl IterNextIterable for PyBytesIterator {}
impl IterNext for PyBytesIterator {
    fn next(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|bytes, pos| {
            Ok(PyIterReturn::from_result(
                bytes
                    .as_bytes()
                    .get(pos)
                    .map(|&x| vm.new_pyobj(x))
                    .ok_or(None),
            ))
        })
    }
}

impl TryFromBorrowedObject for PyBytes {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
        PyBytesInner::try_from_borrowed_object(vm, obj).map(|x| x.into())
    }
}

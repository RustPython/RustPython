use super::{
    PositionIterInternal, PyDictRef, PyIntRef, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef,
};
use crate::{
    anystr::{self, AnyStr},
    atomic_func,
    bytesinner::{
        bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
    },
    class::PyClassImpl,
    common::{hash::PyHash, lock::PyMutex},
    convert::{ToPyObject, ToPyResult},
    function::Either,
    function::{ArgBytesLike, ArgIterable, OptionalArg, OptionalOption, PyComparisonValue},
    protocol::{
        BufferDescriptor, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods, PyNumberMethods,
        PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, Hashable,
        IterNext, IterNextIterable, Iterable, PyComparisonOp, Unconstructible,
    },
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, TryFromObject, VirtualMachine,
};
use bstr::ByteSlice;
use once_cell::sync::Lazy;
use std::{mem::size_of, ops::Deref};

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

impl ToPyObject for Vec<u8> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
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

impl PyPayload for PyBytes {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.bytes_type
    }
}

pub(crate) fn init(context: &Context) {
    PyBytes::extend_class(context, context.types.bytes_type);
    PyBytesIterator::extend_class(context, context.types.bytes_iterator_type);
}

impl Constructor for PyBytes {
    type Args = ByteInnerNewOptions;

    fn py_new(cls: PyTypeRef, options: Self::Args, vm: &VirtualMachine) -> PyResult {
        options.get_bytes(cls, vm).to_pyresult(vm)
    }
}

impl PyBytes {
    pub fn new_ref(data: Vec<u8>, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::from(data), ctx.types.bytes_type.to_owned(), None)
    }
}

#[pyclass(
    flags(BASETYPE),
    with(
        AsMapping,
        AsSequence,
        Hashable,
        Comparable,
        AsBuffer,
        Iterable,
        Constructor,
        AsNumber
    )
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
        if zelf.is(vm.ctx.types.bytes_type) {
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
        self.inner.add(&other.borrow_buf())
    }

    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    #[pystaticmethod]
    fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(from, to, vm)
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "byte")? {
            SequenceIndex::Int(i) => self
                .inner
                .elements
                .getitem_by_index(vm, i)
                .map(|x| vm.ctx.new_int(x).into()),
            SequenceIndex::Slice(slice) => self
                .inner
                .elements
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_bytes(x).into()),
        }
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
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
    fn lstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyBytesInner>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let stripped = zelf.inner.lstrip(chars);
        if stripped == zelf.as_bytes().to_vec() {
            zelf
        } else {
            vm.ctx.new_bytes(stripped)
        }
    }

    #[pymethod]
    fn rstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyBytesInner>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let stripped = zelf.inner.rstrip(chars);
        if stripped == zelf.as_bytes().to_vec() {
            zelf
        } else {
            vm.ctx.new_bytes(stripped)
        }
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
        if value == 1 && zelf.class().is(vm.ctx.types.bytes_type) {
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
            .map(|x| x.to_pyobject(vm))
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
        let bytes = PyBytes::from(zelf.inner.elements.clone()).to_pyobject(vm);
        (
            zelf.class().to_owned(),
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
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buf = PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(zelf.len(), true),
            &BUFFER_METHODS,
        );
        Ok(buf)
    }
}

impl AsMapping for PyBytes {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(PyBytes::mapping_downcast(mapping).len())),
            subscript: atomic_func!(
                |mapping, needle, vm| PyBytes::mapping_downcast(mapping)._getitem(needle, vm)
            ),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsSequence for PyBytes {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: Lazy<PySequenceMethods> = Lazy::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyBytes::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                PyBytes::sequence_downcast(seq)
                    .inner
                    .concat(other, vm)
                    .map(|x| vm.ctx.new_bytes(x).into())
            }),
            repeat: atomic_func!(|seq, n, vm| {
                Ok(vm
                    .ctx
                    .new_bytes(PyBytes::sequence_downcast(seq).repeat(n))
                    .into())
            }),
            item: atomic_func!(|seq, i, vm| {
                PyBytes::sequence_downcast(seq)
                    .inner
                    .elements
                    .getitem_by_index(vm, i)
                    .map(|x| vm.ctx.new_bytes(vec![x]).into())
            }),
            contains: atomic_func!(|seq, other, vm| {
                let other =
                    <Either<PyBytesInner, PyIntRef>>::try_from_object(vm, other.to_owned())?;
                PyBytes::sequence_downcast(seq).contains(other, vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyBytes {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: Lazy<PyNumberMethods> = Lazy::new(|| PyNumberMethods {
            remainder: atomic_func!(|number, other, vm| {
                PyBytes::number_downcast(number)
                    .mod_(other.to_owned(), vm)
                    .to_pyresult(vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        });
        &AS_NUMBER
    }
}

impl Hashable for PyBytes {
    #[inline]
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.inner.hash(vm))
    }
}

impl Comparable for PyBytes {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Ok(if let Some(res) = op.identical_optimization(zelf, other) {
            res.into()
        } else if other.fast_isinstance(vm.ctx.types.memoryview_type)
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
        .into_pyobject(vm))
    }
}

#[pyclass(module = false, name = "bytes_iterator")]
#[derive(Debug)]
pub struct PyBytesIterator {
    internal: PyMutex<PositionIterInternal<PyBytesRef>>,
}

impl PyPayload for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.bytes_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
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
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

use super::{
    PositionIterInternal, PyDictRef, PyGenericAlias, PyIntRef, PyStrRef, PyTuple, PyTupleRef,
    PyType, PyTypeRef,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, TryFromObject, VirtualMachine,
    anystr::{self, AnyStr},
    atomic_func,
    bytes_inner::{
        ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
        ByteInnerTranslateOptions, DecodeArgs, PyBytesInner, bytes_decode,
    },
    class::PyClassImpl,
    common::{hash::PyHash, lock::PyMutex},
    convert::{ToPyObject, ToPyResult},
    function::{
        ArgBytesLike, ArgIndex, ArgIterable, Either, FuncArgs, OptionalArg, OptionalOption,
        PyComparisonValue,
    },
    protocol::{
        BufferDescriptor, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods, PyNumberMethods,
        PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, Hashable,
        IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
    },
};
use bstr::ByteSlice;
use core::{mem::size_of, ops::Deref};
use std::sync::LazyLock;

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
        self.as_bytes()
    }
}

impl AsRef<[u8]> for PyBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}
impl AsRef<[u8]> for PyBytesRef {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PyPayload for PyBytes {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytes_type
    }
}

pub(crate) fn init(context: &Context) {
    PyBytes::extend_class(context, context.types.bytes_type);
    PyBytesIterator::extend_class(context, context.types.bytes_iterator_type);
}

impl Constructor for PyBytes {
    type Args = Vec<u8>;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let options: ByteInnerNewOptions = args.bind(vm)?;

        // Optimizations for exact bytes type
        if cls.is(vm.ctx.types.bytes_type) {
            // Return empty bytes singleton
            if options.source.is_missing()
                && options.encoding.is_missing()
                && options.errors.is_missing()
            {
                return Ok(vm.ctx.empty_bytes.clone().into());
            }

            // Return exact bytes as-is
            if let OptionalArg::Present(ref obj) = options.source
                && options.encoding.is_missing()
                && options.errors.is_missing()
                && let Ok(b) = obj.clone().downcast_exact::<PyBytes>(vm)
            {
                return Ok(b.into_pyref().into());
            }
        }

        // Handle __bytes__ method - may return PyBytes directly
        if let OptionalArg::Present(ref obj) = options.source
            && options.encoding.is_missing()
            && options.errors.is_missing()
            && let Some(bytes_method) = vm.get_method(obj.clone(), identifier!(vm, __bytes__))
        {
            let bytes = bytes_method?.call((), vm)?;
            // If exact bytes type and __bytes__ returns bytes, use it directly
            if cls.is(vm.ctx.types.bytes_type)
                && let Ok(b) = bytes.clone().downcast::<PyBytes>()
            {
                return Ok(b.into());
            }
            // Otherwise convert to Vec<u8>
            let inner = PyBytesInner::try_from_borrowed_object(vm, &bytes)?;
            let payload = Self::py_new(&cls, inner.elements, vm)?;
            return payload.into_ref_with_type(vm, cls).map(Into::into);
        }

        // Fallback to get_bytearray_inner
        let elements = options.get_bytearray_inner(vm)?.elements;

        // Return empty bytes singleton for exact bytes types
        if elements.is_empty() && cls.is(vm.ctx.types.bytes_type) {
            return Ok(vm.ctx.empty_bytes.clone().into());
        }

        let payload = Self::py_new(&cls, elements, vm)?;
        payload.into_ref_with_type(vm, cls).map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, elements: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self::from(elements))
    }
}

impl PyBytes {
    #[deprecated(note = "use PyBytes::from(...).into_ref() instead")]
    pub fn new_ref(data: Vec<u8>, ctx: &Context) -> PyRef<Self> {
        Self::from(data).into_ref(ctx)
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "byte")? {
            SequenceIndex::Int(i) => self
                .getitem_by_index(vm, i)
                .map(|x| vm.ctx.new_int(x).into()),
            SequenceIndex::Slice(slice) => self
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_bytes(x).into()),
        }
    }
}

impl PyRef<PyBytes> {
    fn repeat(self, count: isize, vm: &VirtualMachine) -> PyResult<Self> {
        if count == 1 && self.class().is(vm.ctx.types.bytes_type) {
            // Special case: when some `bytes` is multiplied by `1`,
            // nothing really happens, we need to return an object itself
            // with the same `id()` to be compatible with CPython.
            // This only works for `bytes` itself, not its subclasses.
            return Ok(self);
        }
        self.inner
            .mul(count, vm)
            .map(|x| PyBytes::from(x).into_ref(&vm.ctx))
    }
}

#[pyclass(
    itemsize = 1,
    flags(BASETYPE, _MATCH_SELF),
    with(
        Py,
        PyRef,
        AsMapping,
        AsSequence,
        Hashable,
        Comparable,
        AsBuffer,
        Iterable,
        Constructor,
        AsNumber,
        Representable,
    )
)]
impl PyBytes {
    #[inline]
    pub const fn __len__(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        size_of::<Self>() + self.len() * size_of::<u8>()
    }

    fn __add__(&self, other: ArgBytesLike) -> Vec<u8> {
        self.inner.add(&other.borrow_buf())
    }

    fn __contains__(
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

    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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
    fn fromhex(cls: PyTypeRef, string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex_object(string, vm)?;
        let bytes = vm.ctx.new_bytes(bytes).into();
        PyType::call(&cls, vec![bytes].into(), vm)
    }

    #[pymethod]
    fn center(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.count(options, vm)
    }

    #[pymethod]
    fn join(&self, iter: ArgIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.join(iter, vm)?.into())
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let (affix, substr) =
            match options.prepare(self.as_bytes(), self.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "endswith",
            "bytes",
            |s, x: PyBytesInner| s.ends_with(x.as_bytes()),
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
            match options.prepare(self.as_bytes(), self.len(), |s, r| s.get_bytes(r)) {
                Some(x) => x,
                None => return Ok(false),
            };
        substr.py_starts_ends_with(
            &affix,
            "startswith",
            "bytes",
            |s, x: PyBytesInner| s.starts_with(x.as_bytes()),
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
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn translate(&self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner.strip(chars).into()
    }

    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner.removeprefix(prefix).into()
    }

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
    ) -> PyResult<Self> {
        Ok(self.inner.replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn title(&self) -> Self {
        self.inner.title().into()
    }

    fn __mul__(zelf: PyRef<Self>, value: ArgIndex, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.repeat(value.into_int_ref().try_to_primitive(vm)?, vm)
    }

    fn __mod__(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let formatted = self.inner.cformat(values, vm)?;
        Ok(formatted.into())
    }

    #[pymethod]
    fn __getnewargs__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let param: Vec<PyObjectRef> = self.elements().map(|x| x.to_pyobject(vm)).collect();
        PyTuple::new_ref(param, &vm.ctx)
    }

    // TODO: Uncomment when Python adds __class_getitem__ to bytes
    // #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyBytes> {
    #[pymethod]
    fn __reduce_ex__(
        &self,
        _proto: usize,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        self.__reduce__(vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let bytes = PyBytes::from(self.to_vec()).to_pyobject(vm);
        (
            self.class().to_owned(),
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            self.as_object().dict(),
        )
    }
}

#[pyclass]
impl PyRef<PyBytes> {
    #[pymethod]
    fn __bytes__(self, vm: &VirtualMachine) -> Self {
        if self.is(vm.ctx.types.bytes_type) {
            self
        } else {
            PyBytes::from(self.inner.clone()).into_ref(&vm.ctx)
        }
    }

    #[pymethod]
    fn lstrip(self, chars: OptionalOption<PyBytesInner>, vm: &VirtualMachine) -> Self {
        let stripped = self.inner.lstrip(chars);
        if stripped == self.as_bytes() {
            self
        } else {
            vm.ctx.new_bytes(stripped.to_vec())
        }
    }

    #[pymethod]
    fn rstrip(self, chars: OptionalOption<PyBytesInner>, vm: &VirtualMachine) -> Self {
        let stripped = self.inner.rstrip(chars);
        if stripped == self.as_bytes() {
            self
        } else {
            vm.ctx.new_bytes(stripped.to_vec())
        }
    }

    /// Return a string decoded from the given bytes.
    /// Default encoding is 'utf-8'.
    /// Default errors is 'strict', meaning that encoding errors raise a UnicodeError.
    /// Other possible values are 'ignore', 'replace'
    /// For a list of possible encodings,
    /// see https://docs.python.org/3/library/codecs.html#standard-encodings
    /// currently, only 'utf-8' and 'ascii' implemented
    #[pymethod]
    fn decode(self, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(self.into(), args, vm)
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
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
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
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyBytes::sequence_downcast(seq).len())),
            concat: atomic_func!(|seq, other, vm| {
                PyBytes::sequence_downcast(seq)
                    .inner
                    .concat(other, vm)
                    .map(|x| vm.ctx.new_bytes(x).into())
            }),
            repeat: atomic_func!(|seq, n, vm| {
                let zelf = seq.obj.to_owned().downcast::<PyBytes>().map_err(|_| {
                    vm.new_type_error("bad argument type for built-in operation".to_owned())
                })?;
                zelf.repeat(n, vm).to_pyresult(vm)
            }),
            item: atomic_func!(|seq, i, vm| {
                PyBytes::sequence_downcast(seq)
                    .as_bytes()
                    .getitem_by_index(vm, i)
                    .map(|x| vm.ctx.new_bytes(vec![x]).into())
            }),
            contains: atomic_func!(|seq, other, vm| {
                let other =
                    <Either<PyBytesInner, PyIntRef>>::try_from_object(vm, other.to_owned())?;
                PyBytes::sequence_downcast(seq).__contains__(other, vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for PyBytes {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            remainder: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyBytes>() {
                    a.__mod__(b.to_owned(), vm).to_pyresult(vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Hashable for PyBytes {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.inner.hash(vm))
    }
}

impl Comparable for PyBytes {
    fn cmp(
        zelf: &Py<Self>,
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

impl Representable for PyBytes {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.inner.repr_bytes(vm)
    }
}

#[pyclass(module = false, name = "bytes_iterator")]
#[derive(Debug)]
pub struct PyBytesIterator {
    internal: PyMutex<PositionIterInternal<PyBytesRef>>,
}

impl PyPayload for PyBytesIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytes_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl PyBytesIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.len())
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.internal
            .lock()
            .builtins_iter_reduce(|x| x.clone().into(), vm)
    }

    #[pymethod]
    fn __setstate__(&self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(state, |obj, pos| pos.min(obj.len()), vm)
    }
}

impl SelfIter for PyBytesIterator {}
impl IterNext for PyBytesIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

impl<'a> TryFromBorrowedObject<'a> for PyBytes {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        PyBytesInner::try_from_borrowed_object(vm, obj).map(|x| x.into())
    }
}

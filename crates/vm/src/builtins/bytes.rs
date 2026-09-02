use super::{
    PositionIterInternal, PyDictRef, PyGenericAlias, PyStrRef, PyTuple, PyTupleRef, PyType,
    PyTypeRef, iter::builtins_iter, locked_next,
};
use crate::common::lock::LazyLock;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, TryFromObject, VirtualMachine,
    anystr::{self, AnyStr},
    atomic_func,
    byte::bytes_from_object,
    bytes_inner::{
        ByteInnerFindOptions, ByteInnerHexOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerSplitOptions, ByteInnerSub, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
        bytes_decode,
    },
    class::{PyClassDef, PyClassImpl},
    common::{hash::PyHash, lock::PyMutex},
    convert::{ToPyObject, ToPyResult},
    function::{
        ArgBytesLike, ArgIndex, FuncArgs, OptionalArg, OptionalOption, PyComparisonValue,
        check_meth_o, check_no_kwargs, check_noargs, check_positional,
    },
    protocol::{
        BufferDescriptor, BufferFlags, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods,
        PyNumberMethods, PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, Hashable,
        IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
    },
};
use bstr::ByteSlice;
use core::{mem::size_of, ops::Deref};
use memchr::memchr;

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

pub(crate) fn init(context: &'static Context) {
    PyBytes::extend_class(context, context.types.bytes_type);
    PyBytesIterator::extend_class(context, context.types.bytes_iterator_type);
}

impl Constructor for PyBytes {
    type Args = Vec<u8>;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if args.args.len() > 3 {
            return Err(vm.new_type_error(format!(
                "bytes() takes at most 3 arguments ({} given)",
                args.args.len()
            )));
        }
        ByteInnerNewOptions::check_encoding_errors(&args, "bytes", vm)?;
        let options: ByteInnerNewOptions = args.bind_for(vm, Self::NAME)?;

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
                && let Ok(b) = obj.clone().downcast_exact::<Self>(vm)
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
                && let Ok(b) = bytes.clone().downcast::<Self>()
            {
                return Ok(b.into());
            }
            // Otherwise convert to Vec<u8>
            let inner = PyBytesInner::try_from_borrowed_object(vm, &bytes)?;
            let payload = Self::py_new(&cls, inner.elements, vm)?;
            return payload.into_ref_with_type(vm, cls).map(Into::into);
        }

        let elements = options.get_inner(bytes_from_object, vm)?.elements;

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

    /// Check bytes for interior NULs.
    #[inline]
    #[must_use]
    pub fn contains_nuls(&self) -> bool {
        memchr(b'\0', self.as_bytes()).is_some()
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
    #[must_use]
    pub const fn __len__(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        size_of::<Self>() + self.len() * size_of::<u8>()
    }

    #[pyslot]
    fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf.downcast_ref::<Self>().expect("expected bytes");
        PyBytesInner::warn_on_str("str() on a bytes instance", vm)?;
        Ok(vm.ctx.new_str(zelf.inner.repr_bytes(vm)?))
    }

    fn __add__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        // bytes_concat: "can't concat %.100s to %.100s"
        let class_name = other.class().slot_name().to_string();
        let other = <ArgBytesLike as TryFromObject>::try_from_object(vm, other)
            .map_err(|_| vm.new_type_error(format!("can't concat {class_name} to bytes")))?;
        Ok(self.inner.add(&other.borrow_buf()))
    }

    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        let needle = ByteInnerSub::from_contains_arg(needle, vm)?;
        self.inner.contains(needle, vm)
    }

    #[pystaticmethod]
    fn maketrans(func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        check_no_kwargs(vm, "bytes.maketrans", &func_args)?;
        check_positional(vm, "maketrans", func_args.args.len(), 2, 2)?;
        let (from, to): (PyBytesInner, PyBytesInner) = func_args.bind(vm)?;
        PyBytesInner::maketrans(from, to, vm)
    }

    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    #[pymethod]
    fn isalnum(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isalnum", &func_args)?;
        Ok(self.inner.isalnum())
    }

    #[pymethod]
    fn isalpha(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isalpha", &func_args)?;
        Ok(self.inner.isalpha())
    }

    #[pymethod]
    fn isascii(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isascii", &func_args)?;
        Ok(self.inner.isascii())
    }

    #[pymethod]
    fn isdigit(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isdigit", &func_args)?;
        Ok(self.inner.isdigit())
    }

    #[pymethod]
    fn islower(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.islower", &func_args)?;
        Ok(self.inner.islower())
    }

    #[pymethod]
    fn isspace(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isspace", &func_args)?;
        Ok(self.inner.isspace())
    }

    #[pymethod]
    fn isupper(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.isupper", &func_args)?;
        Ok(self.inner.isupper())
    }

    #[pymethod]
    fn istitle(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_noargs(vm, "bytes.istitle", &func_args)?;
        Ok(self.inner.istitle())
    }

    #[pymethod]
    fn lower(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytes.lower", &func_args)?;
        Ok(self.inner.lower().into())
    }

    #[pymethod]
    fn upper(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytes.upper", &func_args)?;
        Ok(self.inner.upper().into())
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
    pub(crate) fn hex(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<String> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "hex() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let options: ByteInnerHexOptions = func_args.bind(vm)?;
        let (sep, bytes_per_sep) = options.resolve(vm)?;
        Ok(self.inner.hex(sep, bytes_per_sep))
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex_object(string, vm)?;
        let bytes = vm.ctx.new_bytes(bytes).into();
        PyType::call(&cls, vec![bytes].into(), vm)
    }

    #[pymethod]
    fn center(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.center", &func_args)?;
        check_positional(vm, "center", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner.center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.ljust", &func_args)?;
        check_positional(vm, "ljust", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner.ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.rjust", &func_args)?;
        check_positional(vm, "rjust", func_args.args.len(), 1, 2)?;
        let options: ByteInnerPaddingOptions = func_args.bind(vm)?;
        Ok(self.inner.rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytes.count", &func_args)?;
        check_positional(vm, "count", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        self.inner.count(options, vm)
    }

    #[pymethod]
    fn join(&self, iter: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.join(iter, vm)?.into())
    }

    #[pymethod]
    fn endswith(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_no_kwargs(vm, "bytes.endswith", &func_args)?;
        check_positional(vm, "endswith", func_args.args.len(), 1, 3)?;
        let options: anystr::StartsEndsWithArgs = func_args.bind(vm)?;
        let (affix, substr) =
            match options.prepare(self.as_bytes(), self.len(), |s, r| s.get_bytes(r), vm)? {
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
    fn startswith(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        check_no_kwargs(vm, "bytes.startswith", &func_args)?;
        check_positional(vm, "startswith", func_args.args.len(), 1, 3)?;
        let options: anystr::StartsEndsWithArgs = func_args.bind(vm)?;
        let (affix, substr) =
            match options.prepare(self.as_bytes(), self.len(), |s, r| s.get_bytes(r), vm)? {
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
    fn find(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        check_no_kwargs(vm, "bytes.find", &func_args)?;
        check_positional(vm, "find", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn index(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytes.index", &func_args)?;
        check_positional(vm, "index", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner.find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("subsection not found"))
    }

    #[pymethod]
    fn rfind(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        check_no_kwargs(vm, "bytes.rfind", &func_args)?;
        check_positional(vm, "rfind", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<usize> {
        check_no_kwargs(vm, "bytes.rindex", &func_args)?;
        check_positional(vm, "rindex", func_args.args.len(), 1, 3)?;
        let options: ByteInnerFindOptions = func_args.bind(vm)?;
        let index = self.inner.find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("subsection not found"))
    }

    #[pymethod]
    fn translate(&self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.strip", &func_args)?;
        check_positional(vm, "strip", func_args.args.len(), 0, 1)?;
        let (chars,): (OptionalOption<PyBytesInner>,) = func_args.bind(vm)?;
        Ok(self.inner.strip(chars).into())
    }

    #[pymethod]
    fn removeprefix(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytes.removeprefix", &func_args)?;
        let (prefix,): (PyBytesInner,) = func_args.bind(vm)?;
        Ok(self.inner.removeprefix(prefix).into())
    }

    #[pymethod]
    fn removesuffix(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytes.removesuffix", &func_args)?;
        let (suffix,): (PyBytesInner,) = func_args.bind(vm)?;
        Ok(self.inner.removesuffix(suffix).into())
    }

    #[pymethod]
    fn split(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() + func_args.kwargs.len() > 2 {
            return Err(vm.new_type_error(format!(
                "split() takes at most 2 arguments ({} given)",
                func_args.args.len() + func_args.kwargs.len()
            )));
        }
        let options: ByteInnerSplitOptions = func_args.bind(vm)?;
        self.inner
            .split(options, |s, vm| vm.ctx.new_bytes(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn rsplit(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 2 optional arguments
        if func_args.args.len() + func_args.kwargs.len() > 2 {
            return Err(vm.new_type_error(format!(
                "rsplit() takes at most 2 arguments ({} given)",
                func_args.args.len() + func_args.kwargs.len()
            )));
        }
        let options: ByteInnerSplitOptions = func_args.bind(vm)?;
        self.inner
            .rsplit(options, |s, vm| vm.ctx.new_bytes(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn partition(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        check_meth_o(vm, "bytes.partition", &func_args)?;
        let (sep,): (PyObjectRef,) = func_args.bind(vm)?;
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
    fn rpartition(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        check_meth_o(vm, "bytes.rpartition", &func_args)?;
        let (sep,): (PyObjectRef,) = func_args.bind(vm)?;
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
    fn expandtabs(&self, options: anystr::ExpandTabsArgs, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner.expandtabs(options, vm)?.into())
    }

    #[pymethod]
    fn splitlines(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // clinic signature: max 1 optional argument
        if func_args.args.len() + func_args.kwargs.len() > 1 {
            return Err(vm.new_type_error(format!(
                "splitlines() takes at most 1 argument ({} given)",
                func_args.args.len() + func_args.kwargs.len()
            )));
        }
        let options: anystr::SplitLinesArgs = func_args.bind(vm)?;
        Ok(self
            .inner
            .splitlines(options, |x| vm.ctx.new_bytes(x.to_vec()).into()))
    }

    #[pymethod]
    fn zfill(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_meth_o(vm, "bytes.zfill", &func_args)?;
        let (width,): (PyObjectRef,) = func_args.bind(vm)?;
        let width = crate::builtins::to_c_ssize_t(&width, vm)?;
        Ok(self.inner.zfill(width, vm)?.into())
    }

    #[pymethod]
    fn replace(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.replace", &func_args)?;
        check_positional(vm, "replace", func_args.args.len(), 2, 3)?;
        type ReplaceArgs = (PyBytesInner, PyBytesInner, OptionalArg<isize>);
        let (old, new, count): ReplaceArgs = func_args.bind(vm)?;
        Ok(self.inner.replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn title(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_noargs(vm, "bytes.title", &func_args)?;
        Ok(self.inner.title().into())
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
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
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
    fn lstrip(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.lstrip", &func_args)?;
        check_positional(vm, "lstrip", func_args.args.len(), 0, 1)?;
        let (chars,): (OptionalOption<PyBytesInner>,) = func_args.bind(vm)?;
        let stripped = self.inner.lstrip(chars);
        Ok(if stripped == self.as_bytes() {
            self
        } else {
            vm.ctx.new_bytes(stripped.to_vec())
        })
    }

    #[pymethod]
    fn rstrip(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
        check_no_kwargs(vm, "bytes.rstrip", &func_args)?;
        check_positional(vm, "rstrip", func_args.args.len(), 0, 1)?;
        let (chars,): (OptionalOption<PyBytesInner>,) = func_args.bind(vm)?;
        let stripped = self.inner.rstrip(chars);
        Ok(if stripped == self.as_bytes() {
            self
        } else {
            vm.ctx.new_bytes(stripped.to_vec())
        })
    }

    /// Return a string decoded from the given bytes.
    /// Default encoding is 'utf-8'.
    /// Default errors is 'strict', meaning that encoding errors raise a UnicodeError.
    /// Other possible values are 'ignore', 'replace'
    /// For a list of possible encodings,
    /// see https://docs.python.org/3/library/codecs.html#standard-encodings
    /// currently, only 'utf-8' and 'ascii' implemented
    #[pymethod]
    fn decode(self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // decode() takes at most 2 optional arguments
        if func_args.args.len() > 2 {
            return Err(vm.new_type_error(format!(
                "decode() takes at most 2 arguments ({} given)",
                func_args.args.len()
            )));
        }
        let args: DecodeArgs = func_args.bind(vm)?;
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
    fn slot_as_buffer(
        zelf: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer"))?;
        flags.fill_info_check(true, vm)?;
        Self::as_buffer(zelf, vm)
    }

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
                    .map_err(|_| {
                        // bytes_concat: "can't concat %.100s to %.100s"
                        vm.new_type_error(format!(
                            "can't concat {} to bytes",
                            other.class().slot_name()
                        ))
                    })
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
                PyBytes::sequence_downcast(seq).__contains__(other.to_owned(), vm)
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
                zelf.class().slot_name(),
                other.class().slot_name()
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
pub(crate) struct PyBytesIterator {
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
        let func = builtins_iter(vm);
        self.internal.lock().reduce(
            func,
            |x| x.clone().into(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        )
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
        locked_next(&zelf.internal, |bytes, pos| {
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

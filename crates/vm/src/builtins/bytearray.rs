//! Implementation of the python bytearray object.
use super::{
    PositionIterInternal, PyBytes, PyBytesRef, PyDictRef, PyGenericAlias, PyIntRef, PyStrRef,
    PyTuple, PyTupleRef, PyType, PyTypeRef,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    anystr::{self, AnyStr},
    atomic_func,
    byte::{bytes_from_object, value_from_object},
    bytes_inner::{
        ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
        ByteInnerTranslateOptions, DecodeArgs, PyBytesInner, bytes_decode,
    },
    class::PyClassImpl,
    common::{
        atomic::{AtomicUsize, Ordering},
        lock::{
            PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyMutex, PyRwLock,
            PyRwLockReadGuard, PyRwLockWriteGuard,
        },
    },
    convert::{ToPyObject, ToPyResult},
    function::{
        ArgBytesLike, ArgIterable, ArgSize, Either, OptionalArg, OptionalOption, PyComparisonValue,
    },
    protocol::{
        BufferDescriptor, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
        PyMappingMethods, PyNumberMethods, PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor,
        DefaultConstructor, Initializer, IterNext, Iterable, PyComparisonOp, Representable,
        SelfIter,
    },
};
use bstr::ByteSlice;
use core::mem::size_of;

#[pyclass(module = false, name = "bytearray", unhashable = true)]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicUsize,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl From<PyBytesInner> for PyByteArray {
    fn from(inner: PyBytesInner) -> Self {
        Self::from_inner(inner)
    }
}

impl From<Vec<u8>> for PyByteArray {
    fn from(elements: Vec<u8>) -> Self {
        Self::from(PyBytesInner { elements })
    }
}

impl PyPayload for PyByteArray {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytearray_type
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &Context) {
    PyByteArray::extend_class(context, context.types.bytearray_type);
    PyByteArrayIterator::extend_class(context, context.types.bytearray_iterator_type);
}

impl PyByteArray {
    #[deprecated(note = "use PyByteArray::from(...).into_ref() instead")]
    pub fn new_ref(data: Vec<u8>, ctx: &Context) -> PyRef<Self> {
        Self::from(data).into_ref(ctx)
    }

    const fn from_inner(inner: PyBytesInner) -> Self {
        Self {
            inner: PyRwLock::new(inner),
            exports: AtomicUsize::new(0),
        }
    }

    pub fn borrow_buf(&self) -> PyMappedRwLockReadGuard<'_, [u8]> {
        PyRwLockReadGuard::map(self.inner.read(), |inner| &*inner.elements)
    }

    pub fn borrow_buf_mut(&self) -> PyMappedRwLockWriteGuard<'_, Vec<u8>> {
        PyRwLockWriteGuard::map(self.inner.write(), |inner| &mut inner.elements)
    }

    fn repeat(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
        self.inner().mul(value, vm).map(|x| x.into())
    }

    fn _setitem_by_index(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &value)?;
        self.borrow_buf_mut().setitem_by_index(vm, i, value)
    }

    fn _setitem(
        zelf: &Py<Self>,
        needle: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => zelf._setitem_by_index(i, value, vm),
            SequenceIndex::Slice(slice) => {
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytes_from_object(vm, &value)?
                };
                if let Some(mut w) = zelf.try_resizable_opt() {
                    w.elements.setitem_by_slice(vm, slice, &items)
                } else {
                    zelf.borrow_buf_mut()
                        .setitem_by_slice_no_resize(vm, slice, &items)
                }
            }
        }
    }

    fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => self
                .borrow_buf()
                .getitem_by_index(vm, i)
                .map(|x| vm.ctx.new_int(x).into()),
            SequenceIndex::Slice(slice) => self
                .borrow_buf()
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_bytearray(x).into()),
        }
    }

    pub fn _delitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object(vm, needle, "bytearray")? {
            SequenceIndex::Int(i) => self.try_resizable(vm)?.elements.delitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => {
                // TODO: delete 0 elements don't need resizable
                self.try_resizable(vm)?.elements.delitem_by_slice(vm, slice)
            }
        }
    }

    fn irepeat(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult<()> {
        if n == 1 {
            return Ok(());
        }
        let mut w = match zelf.try_resizable(vm) {
            Ok(w) => w,
            Err(err) => {
                return if zelf.borrow_buf().is_empty() {
                    // We can multiple an empty vector by any integer
                    Ok(())
                } else {
                    Err(err)
                };
            }
        };

        w.imul(n, vm)
    }
}

#[pyclass(
    flags(BASETYPE, _MATCH_SELF),
    with(
        Py,
        PyRef,
        Constructor,
        Initializer,
        Comparable,
        AsBuffer,
        AsMapping,
        AsSequence,
        AsNumber,
        Iterable,
        Representable
    )
)]
impl PyByteArray {
    #[cfg(debug_assertions)]
    #[pygetset]
    fn exports(&self) -> usize {
        self.exports.load(Ordering::Relaxed)
    }

    #[inline]
    fn inner(&self) -> PyRwLockReadGuard<'_, PyBytesInner> {
        self.inner.read()
    }
    #[inline]
    fn inner_mut(&self) -> PyRwLockWriteGuard<'_, PyBytesInner> {
        self.inner.write()
    }

    #[pymethod]
    fn __alloc__(&self) -> usize {
        self.inner().capacity()
    }

    fn __len__(&self) -> usize {
        self.borrow_buf().len()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        size_of::<Self>() + self.borrow_buf().len() * size_of::<u8>()
    }

    fn __add__(&self, other: ArgBytesLike) -> Self {
        self.inner().add(&other.borrow_buf()).into()
    }

    fn __contains__(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner().contains(needle, vm)
    }

    fn __iadd__(
        zelf: PyRef<Self>,
        other: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?
            .elements
            .extend(&*other.borrow_buf());
        Ok(zelf)
    }

    fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
    }

    pub fn __delitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(&needle, vm)
    }

    #[pystaticmethod]
    fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(from, to, vm)
    }

    #[pymethod]
    fn isalnum(&self) -> bool {
        self.inner().isalnum()
    }

    #[pymethod]
    fn isalpha(&self) -> bool {
        self.inner().isalpha()
    }

    #[pymethod]
    fn isascii(&self) -> bool {
        self.inner().isascii()
    }

    #[pymethod]
    fn isdigit(&self) -> bool {
        self.inner().isdigit()
    }

    #[pymethod]
    fn islower(&self) -> bool {
        self.inner().islower()
    }

    #[pymethod]
    fn isspace(&self) -> bool {
        self.inner().isspace()
    }

    #[pymethod]
    fn isupper(&self) -> bool {
        self.inner().isupper()
    }

    #[pymethod]
    fn istitle(&self) -> bool {
        self.inner().istitle()
    }

    #[pymethod]
    fn lower(&self) -> Self {
        self.inner().lower().into()
    }

    #[pymethod]
    fn upper(&self) -> Self {
        self.inner().upper().into()
    }

    #[pymethod]
    fn capitalize(&self) -> Self {
        self.inner().capitalize().into()
    }

    #[pymethod]
    fn swapcase(&self) -> Self {
        self.inner().swapcase().into()
    }

    #[pymethod]
    fn hex(
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.inner().hex(sep, bytes_per_sep, vm)
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex_object(string, vm)?;
        let bytes = vm.ctx.new_bytes(bytes);
        let args = vec![bytes.into()].into();
        PyType::call(&cls, args, vm)
    }

    #[pymethod]
    fn center(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner().center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner().ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(&self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner().rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner().count(options, vm)
    }

    #[pymethod]
    fn join(&self, iter: ArgIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner().join(iter, vm)?.into())
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let borrowed = self.borrow_buf();
        let (affix, substr) =
            match options.prepare(&*borrowed, borrowed.len(), |s, r| s.get_bytes(r)) {
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
        let borrowed = self.borrow_buf();
        let (affix, substr) =
            match options.prepare(&*borrowed, borrowed.len(), |s, r| s.get_bytes(r)) {
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
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner().find(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn translate(&self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(self.inner().translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().strip(chars).into()
    }

    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner().removeprefix(prefix).into()
    }

    #[pymethod]
    fn removesuffix(&self, suffix: PyBytesInner) -> Self {
        self.inner().removesuffix(suffix).to_vec().into()
    }

    #[pymethod]
    fn split(
        &self,
        options: ByteInnerSplitOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        self.inner()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn rsplit(
        &self,
        options: ByteInnerSplitOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        self.inner()
            .rsplit(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn partition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
        let value = self.inner();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        )))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let value = self.inner();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.new_tuple((
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        )))
    }

    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner().expandtabs(options).into()
    }

    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        self.inner()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()).into())
    }

    #[pymethod]
    fn zfill(&self, width: isize) -> Self {
        self.inner().zfill(width).into()
    }

    #[pymethod]
    fn replace(
        &self,
        old: PyBytesInner,
        new: PyBytesInner,
        count: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(self.inner().replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn copy(&self) -> Self {
        self.borrow_buf().to_vec().into()
    }

    #[pymethod]
    fn title(&self) -> Self {
        self.inner().title().into()
    }

    fn __mul__(&self, value: ArgSize, vm: &VirtualMachine) -> PyResult<Self> {
        self.repeat(value.into(), vm)
    }

    fn __imul__(zelf: PyRef<Self>, value: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(&zelf, value.into(), vm)?;
        Ok(zelf)
    }

    fn __mod__(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let formatted = self.inner().cformat(values, vm)?;
        Ok(formatted.into())
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_buf_mut().reverse();
    }

    // TODO: Uncomment when Python adds __class_getitem__ to bytearray
    // #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

#[pyclass]
impl Py<PyByteArray> {
    fn __setitem__(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyByteArray::_setitem(self, &needle, value, vm)
    }

    #[pymethod]
    fn pop(&self, index: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<u8> {
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .wrap_index(index.unwrap_or(-1))
            .ok_or_else(|| vm.new_index_error("index out of range"))?;
        Ok(elements.remove(index))
    }

    #[pymethod]
    fn insert(&self, index: isize, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements.saturate_index(index);
        elements.insert(index, value);
        Ok(())
    }

    #[pymethod]
    fn append(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        self.try_resizable(vm)?.elements.push(value);
        Ok(())
    }

    #[pymethod]
    fn remove(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .find_byte(value)
            .ok_or_else(|| vm.new_value_error("value not found in bytearray"))?;
        elements.remove(index);
        Ok(())
    }

    #[pymethod]
    fn extend(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.is(&object) {
            PyByteArray::irepeat(self, 2, vm)
        } else {
            let items = bytes_from_object(vm, &object)?;
            self.try_resizable(vm)?.elements.extend(items);
            Ok(())
        }
    }

    #[pymethod]
    fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.try_resizable(vm)?.elements.clear();
        Ok(())
    }

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
        let bytes = PyBytes::from(self.borrow_buf().to_vec()).to_pyobject(vm);
        (
            self.class().to_owned(),
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            self.as_object().dict(),
        )
    }
}

#[pyclass]
impl PyRef<PyByteArray> {
    #[pymethod]
    fn lstrip(self, chars: OptionalOption<PyBytesInner>, vm: &VirtualMachine) -> Self {
        let inner = self.inner();
        let stripped = inner.lstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            self
        } else {
            vm.ctx.new_pyref(PyByteArray::from(stripped.to_vec()))
        }
    }

    #[pymethod]
    fn rstrip(self, chars: OptionalOption<PyBytesInner>, vm: &VirtualMachine) -> Self {
        let inner = self.inner();
        let stripped = inner.rstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            self
        } else {
            vm.ctx.new_pyref(PyByteArray::from(stripped.to_vec()))
        }
    }

    #[pymethod]
    fn decode(self, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(self.into(), args, vm)
    }
}

impl DefaultConstructor for PyByteArray {}

impl Initializer for PyByteArray {
    type Args = ByteInnerNewOptions;

    fn init(zelf: PyRef<Self>, options: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // First unpack bytearray and *then* get a lock to set it.
        let mut inner = options.get_bytearray_inner(vm)?;
        core::mem::swap(&mut *zelf.inner_mut(), &mut inner);
        Ok(())
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(zelf, other) {
            return Ok(res.into());
        }
        Ok(zelf.inner().cmp(other, op, vm))
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyByteArray>().borrow_buf().into(),
    obj_bytes_mut: |buffer| {
        PyMappedRwLockWriteGuard::map(buffer.obj_as::<PyByteArray>().borrow_buf_mut(), |x| {
            x.as_mut_slice()
        })
        .into()
    },
    release: |buffer| {
        buffer
            .obj_as::<PyByteArray>()
            .exports
            .fetch_sub(1, Ordering::Release);
    },
    retain: |buffer| {
        buffer
            .obj_as::<PyByteArray>()
            .exports
            .fetch_add(1, Ordering::Release);
    },
};

impl AsBuffer for PyByteArray {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(zelf.__len__(), false),
            &BUFFER_METHODS,
        ))
    }
}

impl BufferResizeGuard for PyByteArray {
    type Resizable<'a> = PyRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable_opt(&self) -> Option<Self::Resizable<'_>> {
        let w = self.inner.write();
        (self.exports.load(Ordering::SeqCst) == 0).then_some(w)
    }
}

impl AsMapping for PyByteArray {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(
                PyByteArray::mapping_downcast(mapping).__len__()
            )),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyByteArray::mapping_downcast(mapping).__getitem__(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyByteArray::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.__setitem__(needle.to_owned(), value, vm)
                } else {
                    zelf.__delitem__(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyByteArray {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyByteArray::sequence_downcast(seq).__len__())),
            concat: atomic_func!(|seq, other, vm| {
                PyByteArray::sequence_downcast(seq)
                    .inner()
                    .concat(other, vm)
                    .map(|x| PyByteArray::from(x).into_pyobject(vm))
            }),
            repeat: atomic_func!(|seq, n, vm| {
                PyByteArray::sequence_downcast(seq)
                    .repeat(n, vm)
                    .map(|x| x.into_pyobject(vm))
            }),
            item: atomic_func!(|seq, i, vm| {
                PyByteArray::sequence_downcast(seq)
                    .borrow_buf()
                    .getitem_by_index(vm, i)
                    .map(|x| vm.ctx.new_bytes(vec![x]).into())
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyByteArray::sequence_downcast(seq);
                if let Some(value) = value {
                    zelf._setitem_by_index(i, value, vm)
                } else {
                    zelf.borrow_buf_mut().delitem_by_index(vm, i)
                }
            }),
            contains: atomic_func!(|seq, other, vm| {
                let other =
                    <Either<PyBytesInner, PyIntRef>>::try_from_object(vm, other.to_owned())?;
                PyByteArray::sequence_downcast(seq).__contains__(other, vm)
            }),
            inplace_concat: atomic_func!(|seq, other, vm| {
                let other = ArgBytesLike::try_from_object(vm, other.to_owned())?;
                let zelf = PyByteArray::sequence_downcast(seq).to_owned();
                PyByteArray::__iadd__(zelf, other, vm).map(|x| x.into())
            }),
            inplace_repeat: atomic_func!(|seq, n, vm| {
                let zelf = PyByteArray::sequence_downcast(seq).to_owned();
                PyByteArray::irepeat(&zelf, n, vm)?;
                Ok(zelf.into())
            }),
        };
        &AS_SEQUENCE
    }
}

impl AsNumber for PyByteArray {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            remainder: Some(|a, b, vm| {
                if let Some(a) = a.downcast_ref::<PyByteArray>() {
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

impl Iterable for PyByteArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyByteArrayIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

impl Representable for PyByteArray {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = zelf.class();
        let class_name = class.name();
        zelf.inner().repr_with_name(&class_name, vm)
    }
}

#[pyclass(module = false, name = "bytearray_iterator")]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    internal: PyMutex<PositionIterInternal<PyByteArrayRef>>,
}

impl PyPayload for PyByteArrayIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytearray_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl PyByteArrayIterator {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.__len__())
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
            .set_state(state, |obj, pos| pos.min(obj.__len__()), vm)
    }
}

impl SelfIter for PyByteArrayIterator {}
impl IterNext for PyByteArrayIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|bytearray, pos| {
            let buf = bytearray.borrow_buf();
            Ok(PyIterReturn::from_result(
                buf.get(pos).map(|&x| vm.new_pyobj(x)).ok_or(None),
            ))
        })
    }
}

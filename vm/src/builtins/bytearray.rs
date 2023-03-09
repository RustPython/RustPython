//! Implementation of the python bytearray object.
use super::{
    PositionIterInternal, PyBytes, PyBytesRef, PyDictRef, PyIntRef, PyStrRef, PyTuple, PyTupleRef,
    PyType, PyTypeRef,
};
use crate::{
    anystr::{self, AnyStr},
    atomic_func,
    byte::{bytes_from_object, value_from_object},
    bytesinner::{
        bytes_decode, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerSplitOptions, ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
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
        ArgBytesLike, ArgIterable, ArgSize, Either, FuncArgs, OptionalArg, OptionalOption,
        PyComparisonValue,
    },
    protocol::{
        BufferDescriptor, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
        PyMappingMethods, PyNumberMethods, PySequenceMethods,
    },
    sliceable::{SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, Initializer,
        IterNext, IterNextIterable, Iterable, PyComparisonOp, Unconstructible,
    },
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
};
use bstr::ByteSlice;
use std::mem::size_of;

#[pyclass(module = false, name = "bytearray", unhashable = true)]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicUsize,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    pub fn new_ref(data: Vec<u8>, ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(Self::from(data), ctx.types.bytearray_type.to_owned(), None)
    }

    fn from_inner(inner: PyBytesInner) -> Self {
        PyByteArray {
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
}

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
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.bytearray_type
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &Context) {
    PyByteArray::extend_class(context, context.types.bytearray_type);
    PyByteArrayIterator::extend_class(context, context.types.bytearray_iterator_type);
}

#[pyclass(
    flags(BASETYPE),
    with(
        Constructor,
        Initializer,
        Comparable,
        AsBuffer,
        AsMapping,
        AsSequence,
        AsNumber,
        Iterable
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

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let class = zelf.class();
        let class_name = class.name();
        zelf.inner().repr(Some(&class_name), vm)
    }

    #[pymethod(magic)]
    fn alloc(&self) -> usize {
        self.inner().capacity()
    }

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self.borrow_buf().len()
    }

    #[pymethod(magic)]
    fn sizeof(&self) -> usize {
        size_of::<Self>() + self.borrow_buf().len() * size_of::<u8>()
    }

    #[pymethod(magic)]
    fn add(&self, other: ArgBytesLike) -> Self {
        self.inner().add(&other.borrow_buf()).into()
    }

    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner().contains(needle, vm)
    }

    fn _setitem_by_index(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &value)?;
        self.borrow_buf_mut().setitem_by_index(vm, i, value)
    }

    fn _setitem(
        zelf: PyRef<Self>,
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

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::_setitem(zelf, &needle, value, vm)
    }

    #[pymethod(magic)]
    fn iadd(zelf: PyRef<Self>, other: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?
            .elements
            .extend(&*other.borrow_buf());
        Ok(zelf)
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
                .map(|x| Self::new_ref(x, &vm.ctx).into()),
        }
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self._getitem(&needle, vm)
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

    #[pymethod(magic)]
    pub fn delitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(&needle, vm)
    }

    #[pystaticmethod]
    fn maketrans(from: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(from, to, vm)
    }

    #[pymethod]
    fn pop(zelf: PyRef<Self>, index: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<u8> {
        let elements = &mut zelf.try_resizable(vm)?.elements;
        let index = elements
            .wrap_index(index.unwrap_or(-1))
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        Ok(elements.remove(index))
    }

    #[pymethod]
    fn insert(
        zelf: PyRef<Self>,
        index: isize,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut zelf.try_resizable(vm)?.elements;
        let index = elements.saturate_index(index);
        elements.insert(index, value);
        Ok(())
    }

    #[pymethod]
    fn append(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        zelf.try_resizable(vm)?.elements.push(value);
        Ok(())
    }

    #[pymethod]
    fn remove(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &object)?;
        let elements = &mut zelf.try_resizable(vm)?.elements;
        if let Some(index) = elements.find_byte(value) {
            elements.remove(index);
            Ok(())
        } else {
            Err(vm.new_value_error("value not found in bytearray".to_owned()))
        }
    }

    #[pymethod]
    fn extend(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if zelf.is(&object) {
            Self::irepeat(&zelf, 2, vm)
        } else {
            let items = bytes_from_object(vm, &object)?;
            zelf.try_resizable(vm)?.elements.extend(items);
            Ok(())
        }
    }

    fn irepeat(zelf: &crate::Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult<()> {
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
    fn fromhex(cls: PyTypeRef, string: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex(string.as_str(), vm)?;
        let bytes = vm.ctx.new_bytes(bytes);
        PyType::call(&cls, vec![bytes.into()].into(), vm)
    }

    #[pymethod]
    fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().center(options, vm)?.into())
    }

    #[pymethod]
    fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().ljust(options, vm)?.into())
    }

    #[pymethod]
    fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().rjust(options, vm)?.into())
    }

    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner().count(options, vm)
    }

    #[pymethod]
    fn join(&self, iter: ArgIterable<PyBytesInner>, vm: &VirtualMachine) -> PyResult<PyByteArray> {
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
        substr.py_startsendswith(
            affix,
            "endswith",
            "bytes",
            |s, x: &PyBytesInner| s.ends_with(x.as_bytes()),
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
        substr.py_startsendswith(
            affix,
            "startswith",
            "bytes",
            |s, x: &PyBytesInner| s.starts_with(x.as_bytes()),
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
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.inner().find(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found".to_owned()))
    }

    #[pymethod]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().translate(options, vm)?.into())
    }

    #[pymethod]
    fn strip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().strip(chars).into()
    }

    #[pymethod]
    fn lstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyBytesInner>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let inner = zelf.inner();
        let stripped = inner.lstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            zelf
        } else {
            vm.new_pyref(PyByteArray::from(stripped.to_vec()))
        }
    }

    #[pymethod]
    fn rstrip(
        zelf: PyRef<Self>,
        chars: OptionalOption<PyBytesInner>,
        vm: &VirtualMachine,
    ) -> PyRef<Self> {
        let inner = zelf.inner();
        let stripped = inner.rstrip(chars);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            zelf
        } else {
            vm.new_pyref(PyByteArray::from(stripped.to_vec()))
        }
    }

    /// removeprefix($self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given prefix string removed if present.
    ///
    /// If the bytearray starts with the prefix string, return string[len(prefix):]
    /// Otherwise, return a copy of the original bytearray.
    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> Self {
        self.inner().removeprefix(prefix).into()
    }

    /// removesuffix(self, prefix, /)
    ///
    ///
    /// Return a bytearray object with the given suffix string removed if present.
    ///
    /// If the bytearray ends with the suffix string, return string[:len(suffix)]
    /// Otherwise, return a copy of the original bytearray.
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
        self.inner().split(
            options,
            |s, vm| Self::new_ref(s.to_vec(), &vm.ctx).into(),
            vm,
        )
    }

    #[pymethod]
    fn rsplit(
        &self,
        options: ByteInnerSplitOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        self.inner().rsplit(
            options,
            |s, vm| Self::new_ref(s.to_vec(), &vm.ctx).into(),
            vm,
        )
    }

    #[pymethod]
    fn partition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
        let value = self.inner();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.new_tuple((
            Self::new_ref(front.to_vec(), &vm.ctx),
            Self::new_ref(if has_mid { sep.elements } else { Vec::new() }, &vm.ctx),
            Self::new_ref(back.to_vec(), &vm.ctx),
        )))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let value = self.inner();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.new_tuple((
            Self::new_ref(front.to_vec(), &vm.ctx),
            Self::new_ref(if has_mid { sep.elements } else { Vec::new() }, &vm.ctx),
            Self::new_ref(back.to_vec(), &vm.ctx),
        )))
    }

    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner().expandtabs(options).into()
    }

    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        self.inner()
            .splitlines(options, |x| Self::new_ref(x.to_vec(), &vm.ctx).into())
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
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().replace(old, new, count, vm)?.into())
    }

    #[pymethod]
    fn clear(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        zelf.try_resizable(vm)?.elements.clear();
        Ok(())
    }

    #[pymethod]
    fn copy(&self) -> Self {
        self.borrow_buf().to_vec().into()
    }

    #[pymethod]
    fn title(&self) -> Self {
        self.inner().title().into()
    }

    #[pymethod(name = "__rmul__")]
    #[pymethod(magic)]
    fn mul(&self, value: ArgSize, vm: &VirtualMachine) -> PyResult<Self> {
        self.repeat(value.into(), vm)
    }

    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, value: ArgSize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(&zelf, value.into(), vm)?;
        Ok(zelf)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        let formatted = self.inner().cformat(values, vm)?;
        Ok(formatted.into())
    }

    #[pymethod(magic)]
    fn rmod(&self, _values: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_buf_mut().reverse();
    }

    #[pymethod]
    fn decode(zelf: PyRef<Self>, args: DecodeArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        bytes_decode(zelf.into(), args, vm)
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
        let bytes = PyBytes::from(zelf.borrow_buf().to_vec()).to_pyobject(vm);
        (
            zelf.class().to_owned(),
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

impl Constructor for PyByteArray {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyByteArray::default()
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl Initializer for PyByteArray {
    type Args = ByteInnerNewOptions;

    fn init(zelf: PyRef<Self>, options: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // First unpack bytearray and *then* get a lock to set it.
        let mut inner = options.get_bytearray_inner(vm)?;
        std::mem::swap(&mut *zelf.inner_mut(), &mut inner);
        Ok(())
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &crate::Py<Self>,
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
            BufferDescriptor::simple(zelf.len(), false),
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
            length: atomic_func!(|mapping, _vm| Ok(PyByteArray::mapping_downcast(mapping).len())),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyByteArray::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyByteArray::mapping_downcast(mapping);
                if let Some(value) = value {
                    PyByteArray::setitem(zelf.to_owned(), needle.to_owned(), value, vm)
                } else {
                    zelf.delitem(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyByteArray {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyByteArray::sequence_downcast(seq).len())),
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
                PyByteArray::sequence_downcast(seq).contains(other, vm)
            }),
            inplace_concat: atomic_func!(|seq, other, vm| {
                let other = ArgBytesLike::try_from_object(vm, other.to_owned())?;
                let zelf = PyByteArray::sequence_downcast(seq).to_owned();
                PyByteArray::iadd(zelf, other, vm).map(|x| x.into())
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
            remainder: Some(|number, other, vm| {
                if let Some(number) = number.obj.downcast_ref::<PyByteArray>() {
                    number.mod_(other.to_owned(), vm).to_pyresult(vm)
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

// fn set_value(obj: &PyObject, value: Vec<u8>) {
//     obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
// }

#[pyclass(module = false, name = "bytearray_iterator")]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    internal: PyMutex<PositionIterInternal<PyByteArrayRef>>,
}

impl PyPayload for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.bytearray_iterator_type
    }
}

#[pyclass(with(Constructor, IterNext))]
impl PyByteArrayIterator {
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
impl Unconstructible for PyByteArrayIterator {}

impl IterNextIterable for PyByteArrayIterator {}
impl IterNext for PyByteArrayIterator {
    fn next(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|bytearray, pos| {
            let buf = bytearray.borrow_buf();
            Ok(PyIterReturn::from_result(
                buf.get(pos).map(|&x| vm.new_pyobj(x)).ok_or(None),
            ))
        })
    }
}

//! Implementation of the python bytearray object.
use super::{
    PositionIterInternal, PyBytes, PyBytesRef, PyDictRef, PyIntRef, PyStrRef, PyTuple, PyTupleRef,
    PyTypeRef,
};
use crate::{
    anystr::{self, AnyStr},
    builtins::PyType,
    bytesinner::{
        bytes_decode, bytes_from_object, value_from_object, ByteInnerFindOptions,
        ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
        ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
    },
    common::{
        atomic::{AtomicUsize, Ordering},
        lock::{
            PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyMutex, PyRwLock,
            PyRwLockReadGuard, PyRwLockWriteGuard,
        },
    },
    function::{ArgBytesLike, ArgIterable, FuncArgs, IntoPyObject, OptionalArg, OptionalOption},
    protocol::{
        BufferDescriptor, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
        PyMappingMethods,
    },
    sliceable::{PySliceableSequence, PySliceableSequenceMut, SequenceIndex},
    types::{
        AsBuffer, AsMapping, Callable, Comparable, Constructor, Hashable, IterNext,
        IterNextIterable, Iterable, PyComparisonOp, Unconstructible, Unhashable,
    },
    utils::Either,
    IdProtocol, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef,
    PyObjectView, PyObjectWrap, PyRef, PyResult, PyValue, TypeProtocol, VirtualMachine,
};
use bstr::ByteSlice;
use std::mem::size_of;

#[pyclass(module = false, name = "bytearray")]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicUsize,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    pub fn new_ref(data: Vec<u8>, ctx: &PyContext) -> PyRef<Self> {
        PyRef::new_ref(Self::from(data), ctx.types.bytearray_type.clone(), None)
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

impl PyValue for PyByteArray {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytearray_type
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &PyContext) {
    PyByteArray::extend_class(context, &context.types.bytearray_type);
    PyByteArrayIterator::extend_class(context, &context.types.bytearray_iterator_type);
}

#[pyimpl(
    flags(BASETYPE),
    with(Hashable, Comparable, AsBuffer, AsMapping, Iterable)
)]
impl PyByteArray {
    #[pyslot]
    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyByteArray::default().into_pyresult_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn init(&self, options: ByteInnerNewOptions, vm: &VirtualMachine) -> PyResult<()> {
        // First unpack bytearray and *then* get a lock to set it.
        let mut inner = options.get_bytearray_inner(vm)?;
        std::mem::swap(&mut *self.inner_mut(), &mut inner);
        Ok(())
    }

    #[cfg(debug_assertions)]
    #[pyproperty]
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
    fn repr(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let class = zelf.class();
        let class_name = class.name();
        let s = zelf.inner().repr(Some(&class_name));
        Ok(s)
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
        self.inner().add(&*other.borrow_buf()).into()
    }

    #[pymethod(magic)]
    fn contains(
        &self,
        needle: Either<PyBytesInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner().contains(needle, vm)
    }

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(i) => {
                let value = value_from_object(vm, &value)?;
                let mut elements = zelf.borrow_buf_mut();
                if let Some(i) = elements.wrap_index(i) {
                    elements[i] = value;
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => {
                let slice = slice.to_saturated(vm)?;
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytes_from_object(vm, &value)?
                };
                if let Ok(mut w) = zelf.try_resizable(vm) {
                    w.elements.set_slice_items(vm, slice, items.as_slice())
                } else {
                    zelf.borrow_buf_mut()
                        .set_slice_items_no_resize(vm, slice, items.as_slice())
                }
            }
        }
    }

    #[pymethod(magic)]
    fn iadd(zelf: PyRef<Self>, other: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_resizable(vm)?
            .elements
            .extend(&*other.borrow_buf());
        Ok(zelf)
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner().getitem(Self::NAME, needle, vm)
    }

    #[pymethod(magic)]
    pub fn delitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(int) => {
                let elements = &mut self.try_resizable(vm)?.elements;
                if let Some(idx) = elements.wrap_index(int) {
                    elements.remove(idx);
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => {
                let slice = slice.to_saturated(vm)?;
                let elements = &mut self.try_resizable(vm)?.elements;
                elements.delete_slice(vm, slice)
            }
        }
    }

    #[pymethod]
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

    fn irepeat(zelf: &crate::PyObjectView<Self>, n: isize, vm: &VirtualMachine) -> PyResult<()> {
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
            |s, x: &PyBytesInner| s.starts_with(&x.elements[..]),
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
    fn lstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().lstrip(chars).into()
    }

    #[pymethod]
    fn rstrip(&self, chars: OptionalOption<PyBytesInner>) -> Self {
        self.inner().rstrip(chars).into()
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
    fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
        self.inner().mul(value, vm).map(|x| x.into())
    }

    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(&zelf, value, vm)?;
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
        let bytes = PyBytes::from(zelf.borrow_buf().to_vec()).into_pyobject(vm);
        (
            zelf.as_object().clone_class(),
            PyTuple::new_ref(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(&zelf, &other) {
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
    fn as_buffer(zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        Ok(PyBuffer::new(
            zelf.to_owned().into_object(),
            BufferDescriptor::simple(zelf.len(), false),
            &BUFFER_METHODS,
        ))
    }
}

impl<'a> BufferResizeGuard<'a> for PyByteArray {
    type Resizable = PyRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
        let w = self.inner.upgradable_read();
        if self.exports.load(Ordering::SeqCst) == 0 {
            Ok(parking_lot::lock_api::RwLockUpgradableReadGuard::upgrade(w))
        } else {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
        }
    }
}

impl AsMapping for PyByteArray {
    fn as_mapping(_zelf: &crate::PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: Some(Self::ass_subscript),
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

    #[inline]
    fn ass_subscript(
        zelf: PyObjectRef,
        needle: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            Some(value) => {
                Self::downcast(zelf, vm).map(|zelf| Self::setitem(zelf, needle, value, vm))
            }
            None => Self::downcast_ref(&zelf, vm).map(|zelf| zelf.delitem(needle, vm)),
        }?
    }
}

impl Unhashable for PyByteArray {}

impl Iterable for PyByteArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyByteArrayIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_object(vm))
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

impl PyValue for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytearray_iterator_type
    }
}

#[pyimpl(with(Constructor, IterNext))]
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
    fn next(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        zelf.internal.lock().next(|bytearray, pos| {
            let buf = bytearray.borrow_buf();
            Ok(PyIterReturn::from_result(
                buf.get(pos).map(|&x| vm.new_pyobj(x)).ok_or(None),
            ))
        })
    }
}

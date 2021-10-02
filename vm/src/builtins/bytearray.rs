//! Implementation of the python bytearray object.
use super::{PyBytes, PyBytesRef, PyDictRef, PyIntRef, PyStrRef, PyTupleRef, PyTypeRef};
use crate::common::{
    borrow::{BorrowedValue, BorrowedValueMut},
    lock::{
        PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
        PyRwLockWriteGuard,
    },
};
use crate::{
    anystr::{self, AnyStr},
    bytesinner::{
        bytes_decode, bytes_from_object, value_from_object, ByteInnerFindOptions,
        ByteInnerNewOptions, ByteInnerPaddingOptions, ByteInnerSplitOptions,
        ByteInnerTranslateOptions, DecodeArgs, PyBytesInner,
    },
    function::{ArgBytesLike, ArgIterable, FuncArgs, OptionalArg, OptionalOption},
    protocol::{BufferInternal, BufferOptions, PyBuffer, PyIterReturn, ResizeGuard},
    sliceable::{PySliceableSequence, PySliceableSequenceMut, SequenceIndex},
    slots::{
        AsBuffer, Callable, Comparable, Hashable, Iterable, IteratorIterable, PyComparisonOp,
        SlotIterator, Unhashable,
    },
    utils::Either,
    IdProtocol, IntoPyObject, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObjectRef,
    PyRef, PyResult, PyValue, TypeProtocol, VirtualMachine,
};
use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use std::mem::size_of;

/// "bytearray(iterable_of_ints) -> bytearray\n\
///  bytearray(string, encoding[, errors]) -> bytearray\n\
///  bytearray(bytes_or_buffer) -> mutable copy of bytes_or_buffer\n\
///  bytearray(int) -> bytes array of size given by the parameter initialized with null bytes\n\
///  bytearray() -> empty bytes array\n\n\
///  Construct a mutable bytearray object from:\n  \
///  - an iterable yielding integers in range(256)\n  \
///  - a text string encoded using the specified encoding\n  \
///  - a bytes or a buffer object\n  \
///  - any object implementing the buffer API.\n  \
///  - an integer";
#[pyclass(module = false, name = "bytearray")]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyRwLock<PyBytesInner>,
    exports: AtomicCell<usize>,
}

pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    fn from_inner(inner: PyBytesInner) -> Self {
        PyByteArray {
            inner: PyRwLock::new(inner),
            exports: AtomicCell::new(0),
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
    let bytearray_type = &context.types.bytearray_type;
    extend_class!(context, bytearray_type, {
        "maketrans" => context.new_method("maketrans", bytearray_type.clone(), PyBytesInner::maketrans),
    });

    PyByteArrayIterator::extend_class(context, &context.types.bytearray_iterator_type);
}

#[pyimpl(flags(BASETYPE), with(Hashable, Comparable, AsBuffer, Iterable))]
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

    #[inline]
    fn inner(&self) -> PyRwLockReadGuard<'_, PyBytesInner> {
        self.inner.read()
    }
    #[inline]
    fn inner_mut(&self) -> PyRwLockWriteGuard<'_, PyBytesInner> {
        self.inner.write()
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        self.inner().repr("bytearray(", ")")
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
    fn add(&self, other: ArgBytesLike, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytearray(self.inner().add(&*other.borrow_buf()))
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
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytes_from_object(vm, &value)?
                };
                if let Ok(mut w) = zelf.try_resizable(vm) {
                    w.elements.set_slice_items(vm, &slice, items.as_slice())
                } else {
                    zelf.borrow_buf_mut()
                        .set_slice_items_no_resize(vm, &slice, items.as_slice())
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
    pub fn delitem(&self, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult<()> {
        let elements = &mut self.try_resizable(vm)?.elements;
        match needle {
            SequenceIndex::Int(int) => {
                if let Some(idx) = elements.wrap_index(int) {
                    elements.remove(idx);
                    Ok(())
                } else {
                    Err(vm.new_index_error("index out of range".to_owned()))
                }
            }
            SequenceIndex::Slice(slice) => elements.delete_slice(vm, &slice),
        }
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

    fn irepeat(zelf: &PyRef<Self>, n: usize, vm: &VirtualMachine) -> PyResult<()> {
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
        let elements = &mut w.elements;

        if n == 0 {
            elements.clear();
        } else if n != 1 {
            let old = elements.clone();

            elements.reserve((n - 1) * old.len());
            for _ in 1..n {
                elements.extend(&old);
            }
        }
        Ok(())
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
        Callable::call(&cls, vec![bytes].into(), vm)
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
        self.borrow_buf().py_startsendswith(
            options,
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
        self.borrow_buf().py_startsendswith(
            options,
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
    fn split(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    #[pymethod]
    fn rsplit(&self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        self.inner()
            .rsplit(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()), vm)
    }

    #[pymethod]
    fn partition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyBytesInner
        let value = self.inner();
        let (front, has_mid, back) = value.partition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    #[pymethod]
    fn rpartition(&self, sep: PyBytesInner, vm: &VirtualMachine) -> PyResult {
        let value = self.inner();
        let (back, has_mid, front) = value.rpartition(&sep, vm)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(front.to_vec()),
            vm.ctx
                .new_bytearray(if has_mid { sep.elements } else { Vec::new() }),
            vm.ctx.new_bytearray(back.to_vec()),
        ]))
    }

    #[pymethod]
    fn expandtabs(&self, options: anystr::ExpandTabsArgs) -> Self {
        self.inner().expandtabs(options).into()
    }

    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> PyObjectRef {
        let lines = self
            .inner()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()));
        vm.ctx.new_list(lines)
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
        vm.check_repeat_or_memory_error(self.len(), value)
            .map(|value| self.inner().repeat(value).into())
    }

    #[pymethod(magic)]
    fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        vm.check_repeat_or_memory_error(zelf.len(), value)
            .and_then(|value| Self::irepeat(&zelf, value, vm).map(|_| zelf))
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
        bytes_decode(zelf.into_object(), args, vm)
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
            PyTupleRef::with_elements(vec![bytes], &vm.ctx),
            zelf.as_object().dict(),
        )
    }
}

impl Comparable for PyByteArray {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if let Some(res) = op.identical_optimization(&zelf, &other) {
            return Ok(res.into());
        }
        Ok(zelf.inner().cmp(other, op, vm))
    }
}

impl AsBuffer for PyByteArray {
    fn as_buffer(zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer = PyBuffer::new(
            zelf.as_object().clone(),
            zelf.clone(),
            BufferOptions {
                readonly: false,
                len: zelf.len(),
                ..Default::default()
            },
        );
        Ok(buffer)
    }
}

impl BufferInternal for PyRef<PyByteArray> {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.borrow_buf().into()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        PyRwLockWriteGuard::map(self.inner_mut(), |inner| &mut *inner.elements).into()
    }

    fn release(&self) {
        self.exports.fetch_sub(1);
    }

    fn retain(&self) {
        self.exports.fetch_add(1);
    }
}

impl<'a> ResizeGuard<'a> for PyByteArray {
    type Resizable = PyRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
        let w = self.inner.upgradable_read();
        if self.exports.load() == 0 {
            Ok(parking_lot::lock_api::RwLockUpgradableReadGuard::upgrade(w))
        } else {
            Err(vm
                .new_buffer_error("Existing exports of data: object cannot be re-sized".to_owned()))
        }
    }
}

impl Unhashable for PyByteArray {}

impl Iterable for PyByteArray {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyByteArrayIterator {
            position: AtomicCell::new(0),
            bytearray: zelf,
        }
        .into_object(vm))
    }
}

// fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
//     obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
// }

#[pyclass(module = false, name = "bytearray_iterator")]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    position: AtomicCell<usize>,
    bytearray: PyByteArrayRef,
}

impl PyValue for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bytearray_iterator_type
    }
}

#[pyimpl(with(SlotIterator))]
impl PyByteArrayIterator {}
impl IteratorIterable for PyByteArrayIterator {}
impl SlotIterator for PyByteArrayIterator {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let pos = zelf.position.fetch_add(1);
        let r = if let Some(&ret) = zelf.bytearray.borrow_buf().get(pos) {
            PyIterReturn::Return(ret.into_pyobject(vm))
        } else {
            PyIterReturn::StopIteration(None)
        };
        Ok(r)
    }
}

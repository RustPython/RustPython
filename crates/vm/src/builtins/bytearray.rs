//! Implementation of the python bytearray object.
use super::{
    PositionIterInternal, PyBytes, PyDictRef, PyGenericAlias, PyInt, PyStrRef, PyTuple, PyTupleRef,
    PyType, PyTypeRef, iter::builtins_iter, locked_next,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    anystr::{self, AnyStr, StringRange},
    atomic_func,
    byte::{bytearray_extend_from_object, bytearray_from_object, value_from_object},
    bytes_inner::{
        ByteInnerFindOptions, ByteInnerHexOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
        ByteInnerReplaceOptions, ByteInnerSplitOptions, ByteInnerStripOptions, ByteInnerSub,
        ByteInnerTranslateOptions, ByteOr, BytesJoin, DecodeArgs, PyBytesInner, bytes_decode,
    },
    class::PyClassImpl,
    common::lock::{
        PyDetachingRwLock, PyDetachingRwLockReadGuard, PyDetachingRwLockWriteGuard,
        PyMappedDetachingRwLockReadGuard, PyMappedDetachingRwLockWriteGuard, PyMutex,
    },
    convert::{ToPyObject, ToPyResult},
    function::{ArgBytesLike, FuncArgs, OptionalArg, PyComparisonValue, PySsize},
    protocol::{
        BufferDescriptor, BufferFlags, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
        PyMappingMethods, PyNumberMethods, PySequenceMethods,
    },
    sliceable::{NameBytearray, SequenceIndex, SliceableSequenceMutOp, SliceableSequenceOp},
    types::{
        AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor,
        DefaultConstructor, Initializer, IterNext, Iterable, PyComparisonOp, Representable,
        SelfIter,
    },
};
use bstr::ByteSlice;
use core::mem::size_of;
use num_traits::ToPrimitive;

#[pyclass(module = false, name = "bytearray", unhashable = true)]
#[derive(Debug, Default)]
pub struct PyByteArray {
    inner: PyDetachingRwLock<PyBytesInner>,
    exports: PyMutex<ExportState>,
}

// While count is nonzero, len is the stable logical length of the payload.
// Publish both together; the length is meaningless between export epochs.
#[derive(Debug, Default)]
struct ExportState {
    count: usize,
    len: usize,
}

impl ExportState {
    fn snapshot(&self) -> Option<usize> {
        (self.count != 0).then_some(self.len)
    }

    fn retain_existing(&mut self) -> Option<usize> {
        let len = self.snapshot()?;
        self.count += 1;
        Some(len)
    }

    // The caller holds a payload read guard, so first publication cannot
    // overlap a resize and an existing epoch has the same length.
    fn retain(&mut self, len: usize) {
        if self.count == 0 {
            self.len = len;
        }
        self.count += 1;
    }

    fn release(&mut self) {
        self.count -= 1;
    }
}

struct ByteArrayExportGuard<'a> {
    owner: &'a PyByteArray,
    len: usize,
}

impl Drop for ByteArrayExportGuard<'_> {
    fn drop(&mut self) {
        self.owner.exports.lock().release();
    }
}

enum ResizeAccess<'a> {
    Exclusive(PyDetachingRwLockWriteGuard<'a, PyBytesInner>),
    Exported { len: usize },
}

pub(crate) type PyByteArrayRef = PyRef<PyByteArray>;

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

// Acquisitions can call Python, and releases must follow acquisition order.
// No bytearray lock may be held when this owner is dropped.
#[derive(Default)]
struct ByteArrayBuffers {
    first: Option<PyBuffer>,
    second: Option<PyBuffer>,
}

impl Drop for ByteArrayBuffers {
    fn drop(&mut self) {
        drop(self.first.take());
        drop(self.second.take());
    }
}

/// Fill bytearray class methods dictionary.
pub(crate) fn init(context: &'static Context) {
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
            inner: PyDetachingRwLock::new(inner),
            exports: PyMutex::new(ExportState { count: 0, len: 0 }),
        }
    }

    pub fn borrow_buf(&self) -> PyMappedDetachingRwLockReadGuard<'_, [u8]> {
        PyDetachingRwLockReadGuard::map(self.inner.read(), |inner| &*inner.elements)
    }

    /// Borrow for in-place byte replacement only. Do not change the length,
    /// replace or reallocate storage, or hold this guard across Python calls.
    /// Operations that may resize or reallocate must use export-checked access.
    pub fn borrow_buf_mut(&self) -> PyMappedDetachingRwLockWriteGuard<'_, Vec<u8>> {
        PyDetachingRwLockWriteGuard::map(self.inner.write(), |inner| &mut inner.elements)
    }

    fn begin_export(&self) -> ByteArrayExportGuard<'_> {
        // Existing epochs need no payload lock, including in codec callbacks
        // that already hold a payload read guard.
        let existing_len = { self.exports.lock().retain_existing() };
        let len = match existing_len {
            Some(len) => len,
            None => {
                // Never wait for the payload with the metadata lock held.
                // An earlier resize must finish before first publication.
                let inner = self.inner.read();
                let len = inner.elements.len();
                self.exports.lock().retain(len);
                len
            }
        };
        ByteArrayExportGuard { owner: self, len }
    }

    fn resize_access(&self) -> ResizeAccess<'_> {
        // An export is a borrow someone else still holds, so it is answered
        // before the lock rather than by waiting on it.
        let exported_len = { self.exports.lock().snapshot() };
        if let Some(len) = exported_len {
            return ResizeAccess::Exported { len };
        }
        let inner = self.inner.write();
        // An export may have started while we waited. Capture its length in
        // the same decision, without borrowing the payload a second time.
        let exported_len = { self.exports.lock().snapshot() };
        match exported_len {
            Some(len) => {
                drop(inner);
                ResizeAccess::Exported { len }
            }
            None => ResizeAccess::Exclusive(inner),
        }
    }

    fn with_export<R>(&self, f: impl FnOnce(usize) -> R) -> R {
        let export = self.begin_export();
        f(export.len)
    }

    fn buffer_snapshot(buffer: &PyBuffer, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(buffer.desc.len)
            .map_err(|_| vm.no_memory_error())?;
        buffer.append_to(&mut bytes);
        Ok(bytes)
    }

    fn search_exported<R>(
        &self,
        options: ByteInnerFindOptions,
        search: impl FnOnce(&[u8], &[u8], core::ops::Range<usize>) -> R,
        vm: &VirtualMachine,
    ) -> PyResult<R> {
        self.with_export(|len| {
            let (sub, range) = options.parts(len);
            let buffer = if sub.check_buffer() {
                Some(PyBuffer::from_object(vm, sub, BufferFlags::SIMPLE)?)
            } else {
                None
            };
            let needle = match &buffer {
                Some(buffer) => Self::buffer_snapshot(buffer, vm)?,
                None => ByteInnerSub::try_from_object(vm, sub.to_owned())?.into_vec(vm)?,
            };
            let result = search(&self.borrow_buf(), &needle, range);
            drop(buffer);
            Ok(result)
        })
    }

    fn starts_ends_with(
        &self,
        options: anystr::StartsEndsWithArgs,
        ends: bool,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.with_export(|len| {
            let (affix, range) = options.parts(len);
            let tailmatch = |item: &PyObject| -> PyResult<bool> {
                // bytes subclasses use their bytes directly, as in tailmatch.
                let buffer = if item.downcast_ref::<PyBytes>().is_some() {
                    None
                } else {
                    Some(PyBuffer::from_object(vm, item, BufferFlags::SIMPLE)?)
                };
                let needle = match &buffer {
                    Some(buffer) => Self::buffer_snapshot(buffer, vm)?,
                    None => item.downcast_ref::<PyBytes>().unwrap().as_bytes().to_vec(),
                };
                let result = {
                    let bytes = self.borrow_buf();
                    let slice = match &range {
                        Some(range) if !range.is_normal() => None,
                        Some(range) => bytes.get(range.clone()),
                        None => Some(&*bytes),
                    };
                    slice.is_some_and(|slice| {
                        if ends {
                            slice.ends_with(&needle)
                        } else {
                            slice.starts_with(&needle)
                        }
                    })
                };
                drop(buffer);
                Ok(result)
            };
            if let Some(tuple) = affix.downcast_ref::<PyTuple>() {
                for item in tuple {
                    if tailmatch(item)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            } else {
                tailmatch(affix).map_err(|error| {
                    if error.fast_isinstance(vm.ctx.exceptions.type_error) {
                        vm.new_type_error(format!(
                            "{} first arg must be bytes or a tuple of bytes, not {}",
                            if ends { "endswith" } else { "startswith" },
                            affix.class().name()
                        ))
                    } else {
                        error
                    }
                })
            }
        })
    }

    fn repeat(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
        self.inner().mul(value, vm).map(|x| x.into())
    }

    fn _setitem_by_index(&self, i: isize, value: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, value)?;
        self.borrow_buf_mut().setitem_by_index(vm, i, value)
    }

    fn _setitem(
        zelf: &Py<Self>,
        needle: &PyObject,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match SequenceIndex::try_from_borrowed_object::<NameBytearray>(vm, needle)? {
            SequenceIndex::Int(i) => zelf._setitem_by_index(i, value, vm),
            SequenceIndex::Slice(slice) => {
                let items = if zelf.is(&value) {
                    zelf.borrow_buf().to_vec()
                } else {
                    bytearray_from_object(vm, value)?
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
        match SequenceIndex::try_from_borrowed_object::<NameBytearray>(vm, needle)? {
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
        match SequenceIndex::try_from_borrowed_object::<NameBytearray>(vm, needle)? {
            SequenceIndex::Int(i) => self.delitem_by_index(i, vm),
            SequenceIndex::Slice(slice) => {
                // TODO: delete 0 elements don't need resizable
                self.try_resizable(vm)?.elements.delitem_by_slice(vm, slice)
            }
        }
    }

    fn delitem_by_index(&self, index: isize, vm: &VirtualMachine) -> PyResult<()> {
        // Validate before export rejection when the bytes are available without
        // waiting. A codec callback may already hold a read guard with a writer
        // queued, so a blocking read here could deadlock before that rejection.
        let valid = self
            .inner
            .try_read()
            .map(|inner| inner.elements.wrap_index(index).is_some());
        if valid == Some(false) {
            return Err(vm.new_index_error("bytearray index out of range"));
        }
        let mut inner = self.try_resizable(vm)?;
        // Another thread may have resized the bytearray between the locks.
        let index = inner
            .elements
            .wrap_index(index)
            .ok_or_else(|| vm.new_index_error("bytearray index out of range"))?;
        inner.elements.remove(index);
        Ok(())
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

impl PyByteArray {
    #[inline]
    fn inner(&self) -> PyDetachingRwLockReadGuard<'_, PyBytesInner> {
        self.inner.read()
    }
    fn __len__(&self) -> usize {
        self.borrow_buf().len()
    }

    fn __add__(&self, other: ArgBytesLike) -> Self {
        self.inner().add(&other.borrow_buf()).into()
    }

    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        // Exact builtins cannot call Python while preparing the needle. The
        // read lock alone protects their search; a temporary export would make
        // a concurrent resize fail instead of waiting for that lock.
        if let Some(bytes) = needle.downcast_ref_if_exact::<PyBytes>(vm) {
            return Ok(self.borrow_buf().contains_str(bytes.as_bytes()));
        }
        let int = if needle.class().is(vm.ctx.types.bool_type) {
            needle.downcast_ref::<PyInt>()
        } else {
            needle.downcast_ref_if_exact::<PyInt>(vm)
        };
        if let Some(int) = int {
            let byte = int.as_bigint().byte_or(vm)?;
            return Ok(self.borrow_buf().contains(&byte));
        }
        // Converting the needle runs Python, which must not resize this bytearray.
        let _export = self.begin_export();
        let needle = ByteInnerSub::from_contains_arg(needle, vm)?;
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

    fn __getitem__(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
        self._getitem(needle, vm)
    }

    pub fn __delitem__(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        self._delitem(needle, vm)
    }

    fn __mul__(&self, value: PySsize, vm: &VirtualMachine) -> PyResult<Self> {
        self.repeat(value, vm)
    }

    fn __imul__(zelf: PyRef<Self>, value: PySsize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::irepeat(&zelf, value, vm)?;
        Ok(zelf)
    }

    fn __mod__(&self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        // Formatting calls the values' conversion methods, which can reach
        // this bytearray, so the format is taken by value. It also stays
        // exported until formatting ends, so resizing it raises BufferError.
        let _export = self.begin_export();
        let format = self.inner().clone();
        let formatted = format.cformat(values, vm)?;
        Ok(formatted.into())
    }

    // TODO: Uncomment when Python adds __class_getitem__ to bytearray
    // #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
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
impl PyByteArray {}

#[derive(FromArgs)]
struct ByteArrayReduceExArgs {
    #[pyarg(positional, default)]
    proto: usize,
}

#[derive(FromArgs)]
struct PopArgs {
    #[pyarg(positional, default = -1)]
    index: isize,
}

#[pyclass]
impl Py<PyByteArray> {
    fn __setitem__(
        &self,
        needle: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        PyByteArray::_setitem(self, needle, &value, vm)
    }

    #[pymethod]
    fn pop(&self, index: PopArgs, vm: &VirtualMachine) -> PyResult<u8> {
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .wrap_index(index.index)
            .ok_or_else(|| vm.new_index_error("index out of range"))?;
        Ok(elements.remove(index))
    }

    #[pymethod]
    fn insert(&self, index: isize, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &item)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements.saturate_index(index);
        elements.insert(index, value);
        Ok(())
    }

    #[pymethod]
    fn append(&self, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let value = value_from_object(vm, &item)?;
        self.try_resizable(vm)?.elements.push(value);
        Ok(())
    }

    #[pymethod]
    fn remove(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let byte = value_from_object(vm, &value)?;
        let elements = &mut self.try_resizable(vm)?.elements;
        let index = elements
            .find_byte(byte)
            .ok_or_else(|| vm.new_value_error("value not found in bytearray"))?;
        elements.remove(index);
        Ok(())
    }

    #[pymethod]
    fn extend(&self, iterable_of_ints: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.is(&iterable_of_ints) {
            return PyByteArray::irepeat(self, 2, vm);
        }
        // bytearray_setslice keeps the export alive across the resize, so a value
        // looking at this bytearray is what stops it from growing.
        let buffer = iterable_of_ints
            .check_buffer()
            .then(|| {
                PyBuffer::from_object(vm, &iterable_of_ints, BufferFlags::SIMPLE).map_err(|_| {
                    // What an exporter refuses to hand out leaves the value simply
                    // not usable here, whatever the exporter's own complaint was.
                    vm.new_type_error(format!(
                        "can't set bytearray slice from {}",
                        iterable_of_ints.class().name()
                    ))
                })
            })
            .transpose()?;
        let items = match &buffer {
            Some(buffer) => buffer
                .as_contiguous()
                .ok_or_else(|| {
                    vm.new_buffer_error("non-contiguous buffer is not a bytes-like object")
                })?
                .to_vec(),
            None => bytearray_extend_from_object(vm, &iterable_of_ints)?,
        };
        self.try_resizable(vm)?.elements.extend(items);
        Ok(())
    }

    #[pymethod]
    fn clear(&self, vm: &VirtualMachine) -> PyResult<()> {
        let old = match self.resize_access() {
            ResizeAccess::Exported { len: 0 } => return Ok(()),
            ResizeAccess::Exported { .. } => {
                return Err(
                    vm.new_buffer_error("Existing exports of data: object cannot be re-sized")
                );
            }
            ResizeAccess::Exclusive(mut inner) => {
                if inner.elements.is_empty() {
                    return Ok(());
                }
                core::mem::take(&mut inner.elements)
            }
        };
        drop(old);
        Ok(())
    }

    #[pymethod]
    fn take_bytes(&self, n: OptionalArg, vm: &VirtualMachine) -> PyResult<PyBytes> {
        // __index__ can resize self or acquire/release exports. Convert before
        // observing either the payload or the export epoch.
        let n = match n.into_option().filter(|n| !vm.is_none(n)) {
            None => None,
            Some(n) => {
                let index = n
                    .try_index_opt(vm)
                    .ok_or_else(|| vm.new_type_error("n must be an integer or None"))??;
                Some(index.as_bigint().to_isize().ok_or_else(|| {
                    vm.new_index_error(format!(
                        "cannot fit '{}' into an index-sized integer",
                        n.class().slot_name()
                    ))
                })?)
            }
        };
        let access = self.resize_access();
        let len = match &access {
            ResizeAccess::Exclusive(inner) => inner.elements.len(),
            ResizeAccess::Exported { len } => *len,
        };
        // Vec<u8> lengths fit isize. Adding a nonnegative length to a negative
        // index cannot overflow; preserve the adjusted index in the error.
        let to_take = match n {
            None => len as isize,
            Some(n) if n < 0 => n + len as isize,
            Some(n) => n,
        };
        if to_take < 0 || to_take as usize > len {
            drop(access);
            return Err(
                vm.new_index_error(format!("can't take {to_take} bytes outside size {len}"))
            );
        }
        let ResizeAccess::Exclusive(mut inner) = access else {
            return Err(vm.new_buffer_error("Existing exports of data: object cannot be re-sized"));
        };
        let to_take = to_take as usize;
        let result = if to_take == 0 {
            Vec::new()
        } else if to_take == len {
            core::mem::take(&mut inner.elements)
        } else {
            let mut result = Vec::new();
            if result.try_reserve_exact(to_take).is_err() {
                drop(inner);
                return Err(vm.no_memory_error());
            }
            result.extend_from_slice(&inner.elements[..to_take]);
            inner.elements.drain(..to_take);
            result
        };
        drop(inner);
        Ok(PyBytes::from(result))
    }

    #[pymethod]
    fn __reduce_ex__(
        &self,
        args: ByteArrayReduceExArgs,
        vm: &VirtualMachine,
    ) -> (PyTypeRef, PyTupleRef, Option<PyDictRef>) {
        let _ = args.proto;
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

    #[cfg(debug_assertions)]
    #[pygetset]
    fn exports(&self) -> usize {
        self.exports.lock().count
    }

    #[pymethod]
    fn __alloc__(&self) -> usize {
        self.inner().capacity()
    }

    #[pymethod]
    fn __sizeof__(&self) -> usize {
        size_of::<PyByteArray>() + self.borrow_buf().len() * size_of::<u8>()
    }

    #[pyslot]
    fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf
            .downcast_ref::<PyByteArray>()
            .expect("expected bytearray");
        PyBytesInner::warn_on_str("str() on a bytearray instance", vm)?;
        let class_name = zelf.class().name();
        let repr = zelf.inner().repr_with_name(&class_name, vm)?;
        Ok(vm.ctx.new_str(repr))
    }

    #[pystaticmethod]
    fn maketrans(frm: PyBytesInner, to: PyBytesInner, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        PyBytesInner::maketrans(frm, to, vm)
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
    fn lower(&self) -> PyByteArray {
        self.inner().lower().into()
    }

    #[pymethod]
    fn upper(&self) -> PyByteArray {
        self.inner().upper().into()
    }

    #[pymethod]
    fn capitalize(&self) -> PyByteArray {
        self.inner().capitalize().into()
    }

    #[pymethod]
    fn swapcase(&self) -> PyByteArray {
        self.inner().swapcase().into()
    }

    #[pymethod]
    fn hex(&self, options: ByteInnerHexOptions, vm: &VirtualMachine) -> PyResult<String> {
        // Measuring the separator runs Python, so it happens before the buffer
        // is borrowed, with the buffer exported so that resizing this
        // bytearray from there raises BufferError.
        let (sep, bytes_per_sep) = {
            let _export = self.begin_export();
            options.resolve(vm)?
        };
        Ok(self.inner().hex(sep, bytes_per_sep))
    }

    #[pyclassmethod]
    fn fromhex(cls: PyTypeRef, string: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bytes = PyBytesInner::fromhex_object(&string, vm)?;
        let bytes = vm.ctx.new_bytes(bytes);
        let args = vec![bytes.into()].into();
        PyType::call(&cls, args, vm)
    }

    #[pymethod]
    fn center(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        let (width, fillchar) = options.get_value("center", vm)?;
        Ok(self
            .inner()
            .pad_with(width, fillchar, AnyStr::py_center, vm)?
            .into())
    }

    #[pymethod]
    fn ljust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        let (width, fillchar) = options.get_value("ljust", vm)?;
        Ok(self
            .inner()
            .pad_with(width, fillchar, AnyStr::py_ljust, vm)?
            .into())
    }

    #[pymethod]
    fn rjust(
        &self,
        options: ByteInnerPaddingOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        let (width, fillchar) = options.get_value("rjust", vm)?;
        Ok(self
            .inner()
            .pad_with(width, fillchar, AnyStr::py_rjust, vm)?
            .into())
    }

    #[pymethod]
    fn count(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.search_exported(
            options,
            |bytes, needle, range| bytes.py_count(needle, range, |h, n| h.find_iter(n).count()),
            vm,
        )
    }

    #[pymethod]
    fn join(&self, iterable_of_bytes: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        // Export before driving the iterable: resizing is forbidden, but
        // same-size mutations must be reflected in the joined result.
        let _export = self.begin_export();
        let separator_len = _export.len;
        let items = BytesJoin::new(iterable_of_bytes, vm)?;
        Ok(items
            .join(separator_len, || self.borrow_buf().into(), vm)?
            .into())
    }

    #[pymethod]
    fn endswith(&self, options: anystr::StartsEndsWithArgs, vm: &VirtualMachine) -> PyResult<bool> {
        self.starts_ends_with(options, true, vm)
    }

    #[pymethod]
    fn startswith(
        &self,
        options: anystr::StartsEndsWithArgs,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.starts_ends_with(options, false, vm)
    }

    /// Converting the needle runs Python, which must not resize this
    /// bytearray, so the buffer stays exported while it does.
    fn find_exported<F>(
        &self,
        options: ByteInnerFindOptions,
        find: F,
        vm: &VirtualMachine,
    ) -> PyResult<Option<usize>>
    where
        F: Fn(&[u8], &[u8]) -> Option<usize>,
    {
        self.search_exported(
            options,
            |bytes, needle, range| bytes.py_find(needle, range, find),
            vm,
        )
    }

    #[pymethod]
    fn find(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.find_exported(options, |h, n| h.find(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn index(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.find_exported(options, |h, n| h.find(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn rfind(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let index = self.find_exported(options, |h, n| h.rfind(n), vm)?;
        Ok(index.map_or(-1, |v| v as isize))
    }

    #[pymethod]
    fn rindex(&self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        let index = self.find_exported(options, |h, n| h.rfind(n), vm)?;
        index.ok_or_else(|| vm.new_value_error("substring not found"))
    }

    #[pymethod]
    fn translate(
        &self,
        options: ByteInnerTranslateOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        let mut buffers = ByteArrayBuffers::default();
        if let Some(table) = &options.table {
            let buffer = PyBuffer::from_object(vm, table, BufferFlags::SIMPLE)?;
            if buffer.desc.len != 256 {
                return Err(vm.new_value_error("translation table must be 256 characters long"));
            }
            buffers.first = Some(buffer);
        }
        buffers.second = Some(PyBuffer::from_object(
            vm,
            &options.delete,
            BufferFlags::SIMPLE,
        )?);
        // Both acquisition callbacks finish before any input bytes are read.
        let table = match &buffers.first {
            Some(buffer) => PyByteArray::buffer_snapshot(buffer, vm)?,
            None => (0..=u8::MAX).collect(),
        };
        let delete = PyByteArray::buffer_snapshot(buffers.second.as_ref().unwrap(), vm)?;
        let result = self.inner().translate_with(&table, &delete).into();
        drop(buffers);
        Ok(result)
    }

    #[pymethod]
    fn strip(&self, options: ByteInnerStripOptions) -> PyByteArray {
        self.inner().strip(options.bytes).to_vec().into()
    }

    #[pymethod]
    fn removeprefix(&self, prefix: PyBytesInner) -> PyByteArray {
        self.inner().removeprefix(prefix).to_vec().into()
    }

    #[pymethod]
    fn removesuffix(&self, suffix: PyBytesInner) -> PyByteArray {
        self.inner().removesuffix(suffix).to_vec().into()
    }

    #[pymethod]
    fn split(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // Converting the separator runs Python, which must not resize this
        // bytearray, so the arguments are bound with the buffer exported.
        let _export = self.begin_export();
        let options: ByteInnerSplitOptions = args.bind_for(vm, "split")?;
        self.inner()
            .split(options, |s, vm| vm.ctx.new_bytearray(s.to_vec()).into(), vm)
    }

    #[pymethod]
    fn rsplit(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        let _export = self.begin_export();
        let options: ByteInnerSplitOptions = args.bind_for(vm, "rsplit")?;
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
    fn expandtabs(
        &self,
        options: anystr::ExpandTabsArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        self.inner().expandtabs(options, vm).map(Into::into)
    }

    #[pymethod]
    fn splitlines(&self, options: anystr::SplitLinesArgs, vm: &VirtualMachine) -> Vec<PyObjectRef> {
        self.inner()
            .splitlines(options, |x| vm.ctx.new_bytearray(x.to_vec()).into())
    }

    #[pymethod]
    fn zfill(&self, width: isize, vm: &VirtualMachine) -> PyResult<PyByteArray> {
        Ok(self.inner().zfill(width, vm)?.into())
    }

    #[pymethod]
    fn replace(
        &self,
        options: ByteInnerReplaceOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArray> {
        Ok(self.inner().replace(options, vm)?.into_owned().into())
    }

    #[pymethod]
    fn copy(&self) -> PyByteArray {
        self.borrow_buf().to_vec().into()
    }

    #[pymethod]
    fn title(&self) -> PyByteArray {
        self.inner().title().into()
    }

    #[pymethod]
    fn reverse(&self) {
        self.borrow_buf_mut().reverse();
    }

    #[pymethod]
    pub fn resize(&self, size: isize, vm: &VirtualMachine) -> PyResult<()> {
        if size < 0 {
            return Err(vm.new_value_error("bytearray.resize(): new size must be >= 0"));
        }
        let size = size as usize;
        // An export's stable length avoids re-entering the payload lock.
        let access = self.resize_access();
        let len = match &access {
            ResizeAccess::Exclusive(inner) => inner.elements.len(),
            ResizeAccess::Exported { len } => *len,
        };
        if size == len {
            return Ok(());
        }
        let ResizeAccess::Exclusive(mut inner) = access else {
            return Err(vm.new_buffer_error("Existing exports of data: object cannot be re-sized"));
        };
        let elements = &mut inner.elements;
        elements
            .try_reserve_exact(size.saturating_sub(elements.len()))
            .map_err(|_| vm.no_memory_error())?;
        elements.resize(size, 0);
        Ok(())
    }
}

#[pyclass]
impl PyRef<PyByteArray> {
    #[pymethod]
    fn lstrip(self, options: ByteInnerStripOptions, vm: &VirtualMachine) -> Self {
        let inner = self.inner();
        let stripped = inner.lstrip(options.bytes);
        let elements = &inner.elements;
        if stripped == elements {
            drop(inner);
            self
        } else {
            vm.ctx.new_pyref(PyByteArray::from(stripped.to_vec()))
        }
    }

    #[pymethod]
    fn rstrip(self, options: ByteInnerStripOptions, vm: &VirtualMachine) -> Self {
        let inner = self.inner();
        let stripped = inner.rstrip(options.bytes);
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

    fn init(zelf: &Py<Self>, options: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Reject exports and clear existing contents before converting source.
        zelf.try_resizable(vm)?.elements = Vec::new();
        // Conversion can call Python, so do not hold the lock across it and
        // check again for exports acquired by a callback or another thread.
        let (mut inner, zero_count) =
            options.get_inner_with_zero_count(bytearray_from_object, vm)?;
        // A zero integer count performs no further writes, including when its
        // __index__ callback changed this bytearray or acquired an export.
        if zero_count {
            return Ok(());
        }
        core::mem::swap(&mut *zelf.try_resizable(vm)?, &mut inner);
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
        if !zelf.as_object().check_buffer() || !other.check_buffer() {
            return Ok(PyComparisonValue::NotImplemented);
        }
        let first = match PyBuffer::from_object(vm, zelf.as_object(), BufferFlags::SIMPLE) {
            Ok(buffer) => buffer,
            Err(_) => return Ok(PyComparisonValue::NotImplemented),
        };
        let mut buffers = ByteArrayBuffers {
            first: Some(first),
            second: None,
        };
        buffers.second = match PyBuffer::from_object(vm, other, BufferFlags::SIMPLE) {
            Ok(buffer) => Some(buffer),
            Err(_) => return Ok(PyComparisonValue::NotImplemented),
        };
        let left = buffers.first.as_ref().unwrap();
        let right = buffers.second.as_ref().unwrap();
        let result = if left.desc.len != right.desc.len && op.eval_eq(|| false).is_some() {
            op.eval_eq(|| false).unwrap()
        } else {
            let left = Self::buffer_snapshot(left, vm)?;
            let right = Self::buffer_snapshot(right, vm)?;
            op.eval_eq(|| left == right)
                .unwrap_or_else(|| op.eval_ord(left.cmp(&right)))
        };
        drop(buffers);
        Ok(result.into())
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyByteArray>().borrow_buf().into(),
    obj_bytes_mut: |buffer| {
        PyMappedDetachingRwLockWriteGuard::map(
            buffer.obj_as::<PyByteArray>().borrow_buf_mut(),
            |x| x.as_mut_slice(),
        )
        .into()
    },
    release: |buffer| buffer.obj_as::<PyByteArray>().exports.lock().release(),
    retain: |buffer| {
        // as_buffer holds the payload read guard until PyBuffer::new returns.
        buffer
            .obj_as::<PyByteArray>()
            .exports
            .lock()
            .retain(buffer.desc.len);
    },
};

impl AsBuffer for PyByteArray {
    const RELEASE_BUFFER: bool = true;

    fn slot_as_buffer(
        zelf: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer"))?;
        flags.fill_info_check(false, vm)?;
        Self::as_buffer(zelf, vm)
    }

    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        // Keep the length stable until PyBuffer::new retains the export.
        let inner = zelf.inner();
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(inner.elements.len(), false),
            &BUFFER_METHODS,
        ))
    }
}

impl BufferResizeGuard for PyByteArray {
    type Resizable<'a> = PyDetachingRwLockWriteGuard<'a, PyBytesInner>;

    fn try_resizable_opt(&self) -> Option<Self::Resizable<'_>> {
        match self.resize_access() {
            ResizeAccess::Exclusive(inner) => Some(inner),
            ResizeAccess::Exported { .. } => None,
        }
    }
}

impl AsMapping for PyByteArray {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, _vm| Ok(
                PyByteArray::mapping_downcast(mapping).__len__()
            )),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyByteArray::mapping_downcast(mapping).__getitem__(needle, vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyByteArray::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.__setitem__(needle, value, vm)
                } else {
                    zelf.__delitem__(needle, vm)
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
                let concat_error = || {
                    vm.new_type_error(format!(
                        "can't concat {} to {}",
                        other.class().name(),
                        seq.obj.class().name()
                    ))
                };
                // Protect the left operand before the right buffer callback,
                // without holding either operand's lock across Python calls.
                let left = PyBuffer::from_object(vm, seq.obj, BufferFlags::SIMPLE)
                    .map_err(|_| concat_error())?;
                let right = match PyBuffer::from_object(vm, other, BufferFlags::SIMPLE) {
                    Ok(buffer) => buffer,
                    Err(_) => {
                        let error = concat_error();
                        drop(left);
                        return Err(error);
                    }
                };
                let result = (|| {
                    let len = left
                        .desc
                        .len
                        .checked_add(right.desc.len)
                        .ok_or_else(|| vm.no_memory_error())?;
                    let mut bytes = Vec::new();
                    bytes
                        .try_reserve_exact(len)
                        .map_err(|_| vm.no_memory_error())?;
                    left.append_to(&mut bytes);
                    right.append_to(&mut bytes);
                    Ok(PyByteArray::from(bytes).into_pyobject(vm))
                })();
                // Buffer release can run Python too; all slice guards are gone.
                drop(left);
                drop(right);
                result
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
                    zelf._setitem_by_index(i, &value, vm)
                } else {
                    zelf.delitem_by_index(i, vm)
                }
            }),
            contains: atomic_func!(|seq, other, vm| {
                PyByteArray::sequence_downcast(seq).__contains__(other.to_owned(), vm)
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
pub(crate) struct PyByteArrayIterator {
    internal: PyMutex<PositionIterInternal<PyByteArrayRef>>,
}

impl PyPayload for PyByteArrayIterator {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bytearray_iterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl Py<PyByteArrayIterator> {
    #[pymethod]
    fn __length_hint__(&self) -> usize {
        self.internal.lock().length_hint(|obj| obj.__len__())
    }
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let func = builtins_iter(vm)?;
        Ok(self.internal.lock().reduce(
            func,
            |x| x.clone().into(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        ))
    }

    #[pymethod]
    fn __setstate__(&self, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.internal
            .lock()
            .set_state(&object, |obj, pos| pos.min(obj.__len__()), vm)
    }
}

impl SelfIter for PyByteArrayIterator {}
impl IterNext for PyByteArrayIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        locked_next(&zelf.internal, |bytearray, pos| {
            let buf = bytearray.borrow_buf();
            Ok(PyIterReturn::from_result(
                buf.get(pos).map(|&x| vm.new_pyobj(x)).ok_or(None),
            ))
        })
    }
}

#[cfg(test)]
mod export_state_tests {
    use super::ExportState;

    #[test]
    fn export_epoch_length_is_stable_until_last_release() {
        let mut state = ExportState::default();
        assert_eq!(state.snapshot(), None);
        assert_eq!(state.retain_existing(), None);
        state.retain(3);
        assert_eq!(state.snapshot(), Some(3));
        assert_eq!(state.retain_existing(), Some(3));
        state.retain(3);
        assert_eq!(state.count, 3);
        state.release();
        state.release();
        assert_eq!(state.count, 1);
        assert_eq!(state.snapshot(), Some(3));
        let rejected_length = state.snapshot();
        state.release();
        assert_eq!(state.snapshot(), None);
        state.retain(7);
        assert_eq!(state.snapshot(), Some(7));
        assert_eq!(rejected_length, Some(3));
        state.release();
        assert_eq!(state.snapshot(), None);
    }

    #[test]
    fn empty_export_is_distinct_from_no_export() {
        let mut state = ExportState::default();
        state.retain(0);
        assert_eq!(state.snapshot(), Some(0));
        assert_eq!(state.retain_existing(), Some(0));
        state.release();
        assert_eq!(state.snapshot(), Some(0));
        state.release();
        assert_eq!(state.snapshot(), None);
    }
}

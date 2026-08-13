//! Buffer protocol
//! <https://docs.python.org/3/c-api/buffer.html>

use crate::{
    Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        lock::{MapImmutable, PyMutex, PyMutexGuard},
    },
    object::PyObjectPayload,
    sliceable::SequenceIndexOp,
};
use alloc::borrow::Cow;
use bitflags::bitflags;
use core::{fmt::Debug, ops::Range};
use itertools::Itertools;

bitflags! {
    /// Capabilities a consumer asks a buffer exporter for, the `flags` argument of
    /// `bf_getbuffer` and of `__buffer__` (`PyBUF_*`).
    ///
    /// The composite requests are supersets of the simpler ones, so
    /// [`contains`](Self::contains) answers the `REQ_*` questions an exporter asks:
    /// `flags.contains(BufferFlags::C_CONTIGUOUS)` is `REQ_C_CONTIGUOUS(flags)`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct BufferFlags: u32 {
        const WRITABLE = 0x0001;
        const FORMAT = 0x0004;
        const ND = 0x0008;
        const STRIDES = 0x0010 | Self::ND.bits();
        const C_CONTIGUOUS = 0x0020 | Self::STRIDES.bits();
        const F_CONTIGUOUS = 0x0040 | Self::STRIDES.bits();
        const ANY_CONTIGUOUS = 0x0080 | Self::STRIDES.bits();
        const INDIRECT = 0x0100 | Self::STRIDES.bits();
    }
}

impl BufferFlags {
    /// `PyBUF_SIMPLE`: a plain read-only block of bytes.
    pub const SIMPLE: Self = Self::empty();
    /// `PyBUF_CONTIG`
    pub const CONTIG: Self = Self::ND.union(Self::WRITABLE);
    /// `PyBUF_CONTIG_RO`
    pub const CONTIG_RO: Self = Self::ND;
    /// `PyBUF_STRIDED`
    pub const STRIDED: Self = Self::STRIDES.union(Self::WRITABLE);
    /// `PyBUF_STRIDED_RO`
    pub const STRIDED_RO: Self = Self::STRIDES;
    /// `PyBUF_RECORDS`
    pub const RECORDS: Self = Self::STRIDED.union(Self::FORMAT);
    /// `PyBUF_RECORDS_RO`
    pub const RECORDS_RO: Self = Self::STRIDED_RO.union(Self::FORMAT);
    /// `PyBUF_FULL`: everything an exporter can describe, writable.
    pub const FULL: Self = Self::INDIRECT.union(Self::WRITABLE).union(Self::FORMAT);
    /// `PyBUF_FULL_RO`: everything an exporter can describe, read-only.
    pub const FULL_RO: Self = Self::INDIRECT.union(Self::FORMAT);

    /// `PyBUF_READ`. Belongs to `PyMemoryView_FromMemory`, not to `bf_getbuffer`.
    const MEMORY_READ: Self = Self::from_bits_retain(0x100);
    /// `PyBUF_WRITE`. Belongs to `PyMemoryView_FromMemory`, not to `bf_getbuffer`.
    const MEMORY_WRITE: Self = Self::from_bits_retain(0x200);

    /// Whether this request is really a `PyMemoryView_FromMemory` access mode,
    /// which no exporter can serve.
    #[must_use]
    pub const fn is_memory_access_mode(self) -> bool {
        self.bits() == Self::MEMORY_READ.bits() || self.bits() == Self::MEMORY_WRITE.bits()
    }

    /// Whether the consumer demands a writable buffer.
    #[must_use]
    pub const fn is_writable(self) -> bool {
        self.intersects(Self::WRITABLE)
    }

    /// The argument checks `PyBuffer_FillInfo` performs, for exporters that hand
    /// out a flat block of bytes.
    pub fn fill_info_check(self, readonly: bool, vm: &VirtualMachine) -> PyResult<()> {
        if self == Self::SIMPLE {
            return Ok(());
        }
        if self.is_memory_access_mode() {
            return Err(vm.new_system_error("bad argument to internal function"));
        }
        self.check_writable(readonly, "Object is not writable.", vm)
    }

    /// Reject a writable request against a read-only export.
    pub fn check_writable(
        self,
        readonly: bool,
        message: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.is_writable() && readonly {
            return Err(vm.new_buffer_error(message.to_owned()));
        }
        Ok(())
    }
}

pub struct BufferMethods {
    pub obj_bytes: fn(&PyBuffer) -> BorrowedValue<'_, [u8]>,
    pub obj_bytes_mut: fn(&PyBuffer) -> BorrowedValueMut<'_, [u8]>,
    pub release: fn(&PyBuffer),
    pub retain: fn(&PyBuffer),
}

impl Debug for BufferMethods {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BufferMethods")
            .field("obj_bytes", &(self.obj_bytes as usize))
            .field("obj_bytes_mut", &(self.obj_bytes_mut as usize))
            .field("release", &(self.release as usize))
            .field("retain", &(self.retain as usize))
            .finish()
    }
}

#[derive(Debug, Traverse)]
pub struct PyBuffer {
    pub obj: PyObjectRef,
    #[pytraverse(skip)]
    pub desc: BufferDescriptor,
    #[pytraverse(skip)]
    methods: &'static BufferMethods,
    /// Set on the buffer an exporter handed out, and not on the additional
    /// references taken from it, so `bf_releasebuffer` runs once per acquisition.
    #[pytraverse(skip)]
    acquired: bool,
}

/// Cloning takes another reference to the same export rather than acquiring a
/// new one, so the clone carries no release of its own.
impl Clone for PyBuffer {
    fn clone(&self) -> Self {
        Self {
            obj: self.obj.clone(),
            desc: self.desc.clone(),
            methods: self.methods,
            acquired: false,
        }
    }
}

impl PyBuffer {
    #[must_use]
    pub fn new(obj: PyObjectRef, desc: BufferDescriptor, methods: &'static BufferMethods) -> Self {
        #[cfg(debug_assertions)]
        let desc = desc.validate();

        let zelf = Self {
            obj,
            desc,
            methods,
            acquired: true,
        };
        zelf.retain();
        zelf
    }

    #[must_use]
    pub fn as_contiguous(&self) -> Option<BorrowedValue<'_, [u8]>> {
        self.desc
            .is_contiguous()
            .then(|| unsafe { self.contiguous_unchecked() })
    }

    #[must_use]
    pub fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<'_, [u8]>> {
        (!self.desc.readonly && self.desc.is_contiguous())
            .then(|| unsafe { self.contiguous_mut_unchecked() })
    }

    pub fn from_byte_vector(bytes: Vec<u8>, vm: &VirtualMachine) -> Self {
        let bytes_len = bytes.len();
        Self::new(
            PyPayload::into_pyobject(VecBuffer::from(bytes), vm),
            BufferDescriptor::simple(bytes_len, true),
            &VEC_BUFFER_METHODS,
        )
    }

    /// # Safety
    /// assume the buffer is contiguous
    #[must_use]
    pub unsafe fn contiguous_unchecked(&self) -> BorrowedValue<'_, [u8]> {
        self.obj_bytes()
    }

    /// # Safety
    /// assume the buffer is contiguous and writable
    #[must_use]
    pub unsafe fn contiguous_mut_unchecked(&self) -> BorrowedValueMut<'_, [u8]> {
        self.obj_bytes_mut()
    }

    pub fn append_to(&self, buf: &mut Vec<u8>) {
        if let Some(bytes) = self.as_contiguous() {
            buf.extend_from_slice(&bytes);
        } else {
            let bytes = &*self.obj_bytes();
            self.desc.for_each_segment(true, |range| {
                buf.extend_from_slice(&bytes[range.start as usize..range.end as usize])
            });
        }
    }

    pub fn contiguous_or_collect<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        let borrowed;
        let mut collected;
        let v = if let Some(bytes) = self.as_contiguous() {
            borrowed = bytes;
            &*borrowed
        } else {
            collected = vec![];
            self.append_to(&mut collected);
            &collected
        };
        f(v)
    }

    #[must_use]
    pub fn obj_as<T: PyObjectPayload>(&self) -> &Py<T> {
        unsafe { self.obj.downcast_unchecked_ref() }
    }

    #[must_use]
    pub fn obj_bytes(&self) -> BorrowedValue<'_, [u8]> {
        (self.methods.obj_bytes)(self)
    }

    #[must_use]
    pub fn obj_bytes_mut(&self) -> BorrowedValueMut<'_, [u8]> {
        (self.methods.obj_bytes_mut)(self)
    }

    pub fn release(&self) {
        // slot_bf_releasebuffer: a Python-level `__release_buffer__` runs first,
        // then the exporter's own release so export counts stay balanced.
        if self.acquired && self.obj.class().slots.python_release_buffer.load() {
            crate::builtins::memory::release_buffer_call_python(self);
        }
        (self.methods.release)(self)
    }

    pub fn retain(&self) {
        (self.methods.retain)(self)
    }

    // drop PyBuffer without calling release
    // after this function, the owner should use forget()
    // or wrap PyBuffer in the ManuallyDrop to prevent drop()
    pub(crate) unsafe fn drop_without_release(&mut self) {
        // SAFETY: requirements forwarded from caller
        unsafe {
            core::ptr::drop_in_place(&mut self.obj);
            core::ptr::drop_in_place(&mut self.desc);
        }
    }
}

impl PyBuffer {
    /// Acquire a buffer from `obj`. PyObject_GetBuffer
    pub fn from_object(vm: &VirtualMachine, obj: &PyObject, flags: BufferFlags) -> PyResult<Self> {
        if flags.is_memory_access_mode() {
            return Err(vm.new_system_error("bad argument to internal function"));
        }
        let cls = obj.class();
        if let Some(f) = cls.slots.as_buffer.load() {
            return f(obj, flags, vm);
        }
        Err(vm.new_type_error(format!(
            "a bytes-like object is required, not '{}'",
            cls.name()
        )))
    }
}

impl PyObject {
    /// Whether this object's type exports the buffer protocol. PyObject_CheckBuffer
    ///
    /// A consumer that falls back to something else for non-buffer objects asks
    /// this instead of attempting an acquisition, so that an error raised by
    /// `__buffer__` is not mistaken for "not a buffer".
    #[must_use]
    pub fn check_buffer(&self) -> bool {
        self.class().slots.as_buffer.load().is_some()
    }
}

/// The request a conversion makes when the consumer has no say in it: describe
/// the export as fully as possible, read-only.
impl<'a> TryFromBorrowedObject<'a> for PyBuffer {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        Self::from_object(vm, obj, BufferFlags::FULL_RO)
    }
}

impl Drop for PyBuffer {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone)]
pub struct BufferDescriptor {
    /// product(shape) * itemsize
    /// bytes length, but not the length for obj_bytes() even is contiguous
    pub len: usize,
    pub readonly: bool,
    pub itemsize: usize,
    pub format: Cow<'static, str>,
    /// (shape, stride, suboffset) for each dimension
    pub dim_desc: Vec<(usize, isize, isize)>,
    // TODO: flags
}

impl BufferDescriptor {
    #[must_use]
    pub fn simple(bytes_len: usize, readonly: bool) -> Self {
        Self {
            len: bytes_len,
            readonly,
            itemsize: 1,
            format: Cow::Borrowed("B"),
            dim_desc: vec![(bytes_len, 1, 0)],
        }
    }

    #[must_use]
    pub fn format(
        bytes_len: usize,
        readonly: bool,
        itemsize: usize,
        format: Cow<'static, str>,
    ) -> Self {
        Self {
            len: bytes_len,
            readonly,
            itemsize,
            format,
            dim_desc: vec![(bytes_len / itemsize, itemsize as isize, 0)],
        }
    }

    #[cfg(debug_assertions)]
    #[must_use]
    pub fn validate(self) -> Self {
        // ndim=0 is valid for scalar types (e.g., ctypes Structure)
        if self.ndim() == 0 {
            // Empty structures (len=0) can have itemsize=0
            if self.len > 0 {
                debug_assert_ne!(self.itemsize, 0);
            }
            debug_assert_eq!(self.itemsize, self.len);
        } else {
            let mut shape_product = 1;
            let has_zero_dim = self.dim_desc.iter().any(|(s, _, _)| *s == 0);
            for (shape, stride, suboffset) in self.dim_desc.iter().copied() {
                shape_product *= shape;
                debug_assert!(suboffset >= 0);
                // For empty arrays (any dimension is 0), strides can be 0
                if !has_zero_dim {
                    debug_assert_ne!(stride, 0);
                }
            }
            debug_assert_eq!(shape_product * self.itemsize, self.len);
        }
        self
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        self.dim_desc.len()
    }

    #[must_use]
    pub fn is_contiguous(&self) -> bool {
        if self.len == 0 {
            return true;
        }
        let mut sd = self.itemsize;
        for (shape, stride, _) in self.dim_desc.iter().copied().rev() {
            if shape > 1 && stride != sd as isize {
                return false;
            }
            sd *= shape;
        }
        true
    }

    /// this function do not check the bound
    /// panic if indices.len() != ndim
    #[must_use]
    pub fn fast_position(&self, indices: &[usize]) -> isize {
        let mut pos = 0;
        for (i, (_, stride, suboffset)) in indices
            .iter()
            .copied()
            .zip_eq(self.dim_desc.iter().copied())
        {
            pos += i as isize * stride + suboffset;
        }
        pos
    }

    /// panic if indices.len() != ndim
    pub fn position(&self, indices: &[isize], vm: &VirtualMachine) -> PyResult<isize> {
        let mut pos = 0;
        for (i, (shape, stride, suboffset)) in indices
            .iter()
            .copied()
            .zip_eq(self.dim_desc.iter().copied())
        {
            let i = i.wrapped_at(shape).ok_or_else(|| {
                vm.new_index_error(format!("index out of bounds on dimension {i}"))
            })?;
            pos += i as isize * stride + suboffset;
        }
        Ok(pos)
    }

    pub fn for_each_segment<F>(&self, try_contiguous: bool, mut f: F)
    where
        F: FnMut(Range<isize>),
    {
        if self.ndim() == 0 {
            f(0..self.itemsize as isize);
            return;
        }
        if try_contiguous && self.is_last_dim_contiguous() {
            self._for_each_segment::<_, true>(0, 0, &mut f);
        } else {
            self._for_each_segment::<_, false>(0, 0, &mut f);
        }
    }

    fn _for_each_segment<F, const CONTIGUOUS: bool>(&self, mut index: isize, dim: usize, f: &mut F)
    where
        F: FnMut(Range<isize>),
    {
        let (shape, stride, suboffset) = self.dim_desc[dim];
        if dim + 1 == self.ndim() {
            if CONTIGUOUS {
                f(index..index + (shape * self.itemsize) as isize);
            } else {
                for _ in 0..shape {
                    let pos = index + suboffset;
                    f(pos..pos + self.itemsize as isize);
                    index += stride;
                }
            }
            return;
        }
        for _ in 0..shape {
            self._for_each_segment::<F, CONTIGUOUS>(index + suboffset, dim + 1, f);
            index += stride;
        }
    }

    /// zip two BufferDescriptor with the same shape
    pub fn zip_eq<F>(&self, other: &Self, try_contiguous: bool, mut f: F)
    where
        F: FnMut(Range<isize>, Range<isize>) -> bool,
    {
        if self.ndim() == 0 {
            f(0..self.itemsize as isize, 0..other.itemsize as isize);
            return;
        }
        if try_contiguous && self.is_last_dim_contiguous() {
            self._zip_eq::<_, true>(other, 0, 0, 0, &mut f);
        } else {
            self._zip_eq::<_, false>(other, 0, 0, 0, &mut f);
        }
    }

    fn _zip_eq<F, const CONTIGUOUS: bool>(
        &self,
        other: &Self,
        mut a_index: isize,
        mut b_index: isize,
        dim: usize,
        f: &mut F,
    ) where
        F: FnMut(Range<isize>, Range<isize>) -> bool,
    {
        let (shape, a_stride, a_suboffset) = self.dim_desc[dim];
        let (_b_shape, b_stride, b_suboffset) = other.dim_desc[dim];
        debug_assert_eq!(shape, _b_shape);
        if dim + 1 == self.ndim() {
            if CONTIGUOUS {
                if f(
                    a_index..a_index + (shape * self.itemsize) as isize,
                    b_index..b_index + (shape * other.itemsize) as isize,
                ) {
                    return;
                }
            } else {
                for _ in 0..shape {
                    let a_pos = a_index + a_suboffset;
                    let b_pos = b_index + b_suboffset;
                    if f(
                        a_pos..a_pos + self.itemsize as isize,
                        b_pos..b_pos + other.itemsize as isize,
                    ) {
                        return;
                    }
                    a_index += a_stride;
                    b_index += b_stride;
                }
            }
            return;
        }

        for _ in 0..shape {
            self._zip_eq::<F, CONTIGUOUS>(
                other,
                a_index + a_suboffset,
                b_index + b_suboffset,
                dim + 1,
                f,
            );
            a_index += a_stride;
            b_index += b_stride;
        }
    }

    #[must_use]
    fn is_last_dim_contiguous(&self) -> bool {
        let (_, stride, suboffset) = self.dim_desc[self.ndim() - 1];
        suboffset == 0 && stride == self.itemsize as isize
    }

    #[must_use]
    pub fn is_zero_in_shape(&self) -> bool {
        self.dim_desc.iter().any(|(shape, _, _)| *shape == 0)
    }

    // TODO: support column-major order
}

pub trait BufferResizeGuard {
    type Resizable<'a>: 'a
    where
        Self: 'a;
    fn try_resizable_opt(&self) -> Option<Self::Resizable<'_>>;
    fn try_resizable(&self, vm: &VirtualMachine) -> PyResult<Self::Resizable<'_>> {
        self.try_resizable_opt().ok_or_else(|| {
            vm.new_buffer_error("Existing exports of data: object cannot be re-sized")
        })
    }
}

#[pyclass(module = false, name = "vec_buffer")]
#[derive(Debug, PyPayload)]
pub struct VecBuffer {
    data: PyMutex<Vec<u8>>,
}

#[pyclass(flags(BASETYPE, DISALLOW_INSTANTIATION))]
impl VecBuffer {
    pub fn take(&self) -> Vec<u8> {
        core::mem::take(&mut self.data.lock())
    }
}

impl From<Vec<u8>> for VecBuffer {
    fn from(data: Vec<u8>) -> Self {
        Self {
            data: PyMutex::new(data),
        }
    }
}

impl PyRef<VecBuffer> {
    #[must_use]
    pub fn into_pybuffer(self, readonly: bool) -> PyBuffer {
        let len = self.data.lock().len();
        PyBuffer::new(
            self.into(),
            BufferDescriptor::simple(len, readonly),
            &VEC_BUFFER_METHODS,
        )
    }

    #[must_use]
    pub fn into_pybuffer_with_descriptor(self, desc: BufferDescriptor) -> PyBuffer {
        PyBuffer::new(self.into(), desc, &VEC_BUFFER_METHODS)
    }
}

static VEC_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        PyMutexGuard::map_immutable(buffer.obj_as::<VecBuffer>().data.lock(), |x| x.as_slice())
            .into()
    },
    obj_bytes_mut: |buffer| {
        PyMutexGuard::map(buffer.obj_as::<VecBuffer>().data.lock(), |x| {
            x.as_mut_slice()
        })
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

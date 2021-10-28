//! Buffer protocol

use itertools::Itertools;

use crate::{
    builtins::PyTypeRef,
    common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        lock::{MapImmutable, PyMutex, PyMutexGuard},
    },
    types::{Constructor, Unconstructible},
    PyObject, PyObjectPayload, PyObjectRef, PyObjectView, PyObjectWrap, PyRef, PyResult, PyValue,
    TryFromBorrowedObject, TypeProtocol, VirtualMachine,
};
use std::{borrow::Cow, fmt::Debug, ops::Range};

#[allow(clippy::type_complexity)]
pub struct BufferMethods {
    pub obj_bytes: fn(&PyBuffer) -> BorrowedValue<[u8]>,
    pub obj_bytes_mut: fn(&PyBuffer) -> BorrowedValueMut<[u8]>,
    pub release: Option<fn(&PyBuffer)>,
    pub retain: Option<fn(&PyBuffer)>,
}

impl Debug for BufferMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferMethods")
            .field("obj_bytes", &(self.obj_bytes as usize))
            .field("obj_bytes_mut", &(self.obj_bytes_mut as usize))
            .field("release", &self.release.map(|x| x as usize))
            .field("retain", &self.retain.map(|x| x as usize))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct PyBuffer {
    pub obj: PyObjectRef,
    pub options: BufferOptions,
    methods: &'static BufferMethods,
}

impl PyBuffer {
    pub fn new(obj: PyObjectRef, options: BufferOptions, methods: &'static BufferMethods) -> Self {
        let zelf = Self {
            obj,
            options: options.validate(),
            methods,
        };
        zelf.retain();
        zelf
    }

    pub fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        self.options.is_contiguous().then(|| self.obj_bytes())
    }

    pub fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.options.is_contiguous().then(|| self.obj_bytes_mut())
    }

    pub fn collect(&self, buf: &mut Vec<u8>) {
        if self.options.is_contiguous() {
            buf.extend_from_slice(&self.obj_bytes());
        } else {
            let bytes = &*self.obj_bytes();
            self.options
                .for_each_segment(|range| buf.extend_from_slice(&bytes[range]));
        }
    }

    pub fn contiguous_or_collect<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        let borrowed;
        let mut collected;
        let v = if self.options.is_contiguous() {
            borrowed = self.obj_bytes();
            &*borrowed
        } else {
            collected = vec![];
            self.collect(&mut collected);
            &collected
        };
        f(v)
    }

    pub fn obj_as<T: PyObjectPayload>(&self) -> &PyObjectView<T> {
        self.obj.downcast_ref().unwrap()
    }

    pub fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        (self.methods.obj_bytes)(self)
    }

    pub fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        (self.methods.obj_bytes_mut)(self)
    }

    pub fn release(&self) {
        if let Some(f) = self.methods.release {
            f(self)
        }
    }

    pub fn retain(&self) {
        if let Some(f) = self.methods.retain {
            f(self)
        }
    }

    // drop PyBuffer without calling release
    // after this function, the owner should use forget()
    // or wrap PyBuffer in the ManaullyDrop to prevent drop()
    pub(crate) unsafe fn drop_without_release(&mut self) {
        std::ptr::drop_in_place(&mut self.obj);
        std::ptr::drop_in_place(&mut self.options);
    }
}

impl TryFromBorrowedObject for PyBuffer {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
        let cls = obj.class();
        if let Some(f) = cls.mro_find_map(|cls| cls.slots.as_buffer) {
            return f(obj, vm);
        }
        Err(vm.new_type_error(format!(
            "a bytes-like object is required, not '{}'",
            cls.name()
        )))
    }
}

// What we actually want to implement is:
// impl<T> Drop for T where T: BufferInternal
// but it is not supported by Rust
impl Drop for PyBuffer {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone)]
pub struct BufferOptions {
    /// product(shape) * itemsize
    /// NOT the bytes length if buffer is discontiguous
    pub len: usize,
    pub readonly: bool,
    pub itemsize: usize,
    pub format: Cow<'static, str>,
    // pub ndim: usize,
    /// (shape, stride, suboffset) for each dimension
    pub dim_descriptor: Vec<(usize, isize, isize)>,
    // pub shape: Vec<usize>,
    // pub strides: Vec<isize>,
    // pub suboffsets: Vec<isize>,
}

impl BufferOptions {
    pub fn simple(bytes_len: usize, readonly: bool) -> Self {
        Self {
            len: bytes_len,
            readonly,
            itemsize: 1,
            format: Cow::Borrowed("B"),
            dim_descriptor: vec![(bytes_len, 1, 0)],
        }
    }

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
            dim_descriptor: vec![(bytes_len / itemsize, itemsize as isize, 0)],
        }
    }

    #[cfg(debug_assertions)]
    pub fn validate(self) -> Self {
        assert!(self.itemsize != 0);
        assert!(self.ndim() != 0);
        let mut shape_product = 1;
        for (shape, stride, suboffset) in self.dim_descriptor {
            shape_product *= shape;
            assert!(suboffset >= 0);
            assert!(stride != 0);
            if stride.is_negative() {
                // if stride is negative, we access memory in reversed order
                // so the suboffset should be n*stride shift the index to the tail
                assert!(suboffset == -stride * shape as isize);
            } else {
                assert!(suboffset == 0);
            }
        }
        assert!(shape_product == self.len);
        self
    }

    #[cfg(not(debug_assertions))]
    pub fn validate(self) -> Self {
        self
    }

    pub fn ndim(&self) -> usize {
        self.dim_descriptor.len()
    }

    pub fn is_contiguous(&self) -> bool {
        if self.len == 0 {
            return true;
        }
        let mut sd = self.itemsize;
        for (shape, stride, _) in self.dim_descriptor.into_iter().rev() {
            if shape > 1 && stride != sd as isize {
                return false;
            }
            sd *= shape;
        }
        true
    }

    /// this function do not check the bound
    pub fn get_position(&self, indices: &[usize]) -> usize {
        let pos = 0;
        for (&i, (_, stride, suboffset)) in indices.iter().zip_eq(self.dim_descriptor) {
            pos += (i as isize * stride + suboffset) as usize;
        }
        pos
    }

    pub fn for_each<F>(&self, f: F)
    where
        F: FnMut(usize),
    {
        self._for_each(0, 0, f);
    }

    fn _for_each<F>(&self, mut index: isize, dim: usize, f: F)
    where
        F: FnMut(usize),
    {
        let (shape, stride, suboffset) = self.dim_descriptor[dim];
        if dim + 1 == self.ndim() {
            for i in 0..shape {
                f((index + suboffset) as usize);
                index += stride;
            }
            return;
        }
        for i in 0..shape {
            self._for_each(index + suboffset, dim + 1, f);
            index += stride;
        }
    }

    pub fn for_each_segment<F>(&self, f: F)
    where
        F: FnMut(Range<usize>),
    {
        if self.is_last_dim_contiguous() {
            self._for_each_segment::<_, true>(0, 0, f);
        } else {
            self._for_each_segment::<_, false>(0, 0, f)
        }
    }

    fn _for_each_segment<F, const CONTI: bool>(&self, mut index: isize, dim: usize, f: F)
    where
        F: FnMut(Range<usize>),
    {
        let (shape, stride, suboffset) = self.dim_descriptor[dim];
        if dim + 1 == self.ndim() {
            if CONTI {
                f(index as usize..index as usize + shape * self.itemsize);
            } else {
                for i in 0..shape {
                    let pos = (index + suboffset) as usize;
                    f(pos..pos + self.itemsize);
                    index += stride;
                }
            }
            return;
        }
        for i in 0..shape {
            self._for_each_segment(index + suboffset, dim + 1, f);
            index += stride;
        }
    }

    fn is_last_dim_contiguous(&self) -> bool {
        let (_, stride, suboffset) = self.dim_descriptor[self.ndim() - 1];
        suboffset == 0 && stride == self.itemsize as isize
    }

    // TODO: support fortain order
}

pub trait BufferResizeGuard<'a> {
    type Resizable: 'a;
    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable>;
}

#[pyclass(module = false, name = "vec_buffer")]
#[derive(Debug)]
pub struct VecBuffer(PyMutex<Vec<u8>>);

#[pyimpl(flags(BASETYPE), with(Constructor))]
impl VecBuffer {
    pub fn new(v: Vec<u8>) -> Self {
        Self(PyMutex::new(v))
    }
    pub fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock())
    }
}
impl Unconstructible for VecBuffer {}
impl PyValue for VecBuffer {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.vec_buffer_type
    }
}

impl PyRef<VecBuffer> {
    pub fn into_pybuffer(self) -> PyBuffer {
        let len = self.0.lock().len();
        PyBuffer::new(
            self.into_object(),
            BufferOptions::simple(len, false),
            &VEC_BUFFER_METHODS,
        )
    }
    pub fn into_readonly_pybuffer(self) -> PyBuffer {
        let len = self.0.lock().len();
        PyBuffer::new(
            self.into_object(),
            BufferOptions::simple(len, true),
            &VEC_BUFFER_METHODS,
        )
    }
}

static VEC_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        PyMutexGuard::map_immutable(buffer.obj_as::<VecBuffer>().0.lock(), |x| x.as_slice()).into()
    },
    obj_bytes_mut: |buffer| {
        PyMutexGuard::map(buffer.obj_as::<VecBuffer>().0.lock(), |x| x.as_mut_slice()).into()
    },
    release: None,
    retain: None,
};

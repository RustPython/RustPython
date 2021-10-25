//! Buffer protocol

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
use std::{borrow::Cow, fmt::Debug};

#[allow(clippy::type_complexity)]
pub struct BufferMethods {
    // always reflecting the whole bytes of the most top object
    pub obj_bytes: fn(&PyBuffer) -> BorrowedValue<[u8]>,
    // always reflecting the whole bytes of the most top object
    pub obj_bytes_mut: fn(&PyBuffer) -> BorrowedValueMut<[u8]>,
    // GUARANTEE: called only if the buffer option is contiguous
    pub contiguous: Option<fn(&PyBuffer) -> BorrowedValue<[u8]>>,
    // GUARANTEE: called only if the buffer option is contiguous
    pub contiguous_mut: Option<fn(&PyBuffer) -> BorrowedValueMut<[u8]>>,
    // collect bytes to buf when buffer option is not contiguous
    pub collect_bytes: Option<fn(&PyBuffer, buf: &mut Vec<u8>)>,
    pub release: Option<fn(&PyBuffer)>,
    pub retain: Option<fn(&PyBuffer)>,
}

impl Debug for BufferMethods {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferMethods")
            .field("obj_bytes", &(self.obj_bytes as usize))
            .field("obj_bytes_mut", &(self.obj_bytes_mut as usize))
            .field("contiguous", &self.contiguous.map(|x| x as usize))
            .field("contiguous_mut", &self.contiguous_mut.map(|x| x as usize))
            .field("collect_bytes", &self.collect_bytes.map(|x| x as usize))
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
            options,
            methods,
        };
        zelf.retain();
        zelf
    }

    pub fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        self.options.contiguous.then(|| self._contiguous())
    }

    pub fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.options.contiguous.then(|| self._contiguous_mut())
    }

    pub fn collect_bytes(&self, buf: &mut Vec<u8>) {
        if self.options.contiguous {
            buf.extend_from_slice(&self._contiguous());
        } else {
            self._collect_bytes(buf);
        }
    }

    pub fn contiguous_or_collect<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        let borrowed;
        let mut collected;
        let v = if self.options.contiguous {
            borrowed = self._contiguous();
            &*borrowed
        } else {
            collected = vec![];
            self._collect_bytes(&mut collected);
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

    // SAFETY: should only called if option has contiguous
    pub(crate) fn _contiguous(&self) -> BorrowedValue<[u8]> {
        self.methods
            .contiguous
            .map(|f| f(self))
            .unwrap_or_else(|| self.obj_bytes())
    }

    // SAFETY: should only called if option has contiguous
    pub(crate) fn _contiguous_mut(&self) -> BorrowedValueMut<[u8]> {
        self.methods
            .contiguous_mut
            .map(|f| f(self))
            .unwrap_or_else(|| self.obj_bytes_mut())
    }

    // WARNING: should always try to clone from the contiguous first
    pub(crate) fn _collect_bytes(&self, buf: &mut Vec<u8>) {
        self.methods
            .collect_bytes
            .map(|f| f(self, buf))
            .unwrap_or_else(|| buf.extend_from_slice(&self.obj_bytes()))
    }

    // drop PyBuffer without calling release
    // after this function, the owner should use forget()
    // or wrap PyBuffer in the ManaullyDrop to prevent drop()
    pub(crate) unsafe fn drop_without_release(&mut self) {
        // self.obj = PyObjectRef::from_raw(0 as *const PyObject);
        // self.options = BufferOptions::default();
        std::ptr::drop_in_place(&mut self.obj);
        std::ptr::drop_in_place(&mut self.options);
    }
}

#[derive(Debug, Clone)]
pub struct BufferOptions {
    // buf
    pub len: usize,
    pub readonly: bool,
    pub itemsize: usize,
    pub format: Cow<'static, str>,
    pub ndim: usize, // TODO: support multiple dimension array
    pub shape: Vec<usize>,
    pub strides: Vec<isize>,
    // suboffsets

    // RustPython fields
    pub contiguous: bool,
}

impl BufferOptions {
    pub const DEFAULT: Self = BufferOptions {
        len: 0,
        readonly: true,
        itemsize: 1,
        format: Cow::Borrowed("B"),
        ndim: 1,
        shape: Vec::new(),
        strides: Vec::new(),
        contiguous: true,
    };
}

impl Default for BufferOptions {
    fn default() -> Self {
        Self::DEFAULT
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
            BufferOptions {
                len,
                readonly: false,
                ..Default::default()
            },
            &VEC_BUFFER_METHODS,
        )
    }
    pub fn into_readonly_pybuffer(self) -> PyBuffer {
        let len = self.0.lock().len();
        PyBuffer::new(
            self.into_object(),
            BufferOptions {
                len,
                readonly: true,
                ..Default::default()
            },
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
    contiguous: None,
    contiguous_mut: None,
    collect_bytes: None,
    release: None,
    retain: None,
};

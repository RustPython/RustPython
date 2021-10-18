//! Buffer protocol

use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::rc::PyRc;
use crate::PyThreadingConstraint;
use crate::{PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, TypeProtocol, VirtualMachine};
use std::{borrow::Cow, fmt::Debug};

pub struct BufferMethods {
    // always reflecting the whole bytes of the most top object
    obj_bytes: fn(&PyObjectRef) -> BorrowedValue<[u8]>,
    // always reflecting the whole bytes of the most top object
    obj_bytes_mut: fn(&PyObjectRef) -> BorrowedValueMut<[u8]>,
    // GUARANTEE: called only if the buffer option is contiguous
    contiguous: Option<fn(&PyObjectRef) -> BorrowedValue<[u8]>>,
    // GUARANTEE: called only if the buffer option is contiguous
    contiguous_mut: Option<fn(&PyObjectRef) -> BorrowedValueMut<[u8]>>,
    // collect bytes to buf when buffer option is not contiguous
    collect_bytes: Option<fn(&PyObjectRef, buf: &mut Vec<u8>)>,
    release: Option<fn(&PyObjectRef)>,
    retain: Option<fn(&PyObjectRef)>,
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

#[derive(Debug)]
pub struct PyBuffer {
    pub obj: PyObjectRef,
    pub options: BufferOptions,
    pub(crate) methods: &'static BufferMethods,
}

impl PyBuffer {
    pub fn new(obj: PyObjectRef, options: BufferOptions, methods: &'static BufferMethods) -> Self {
        let zelf = Self {
            obj,
            options,
            methods,
        };
        zelf._retain();
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
        let collected;
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

    pub fn clone_with_options(&self, options: BufferOptions) -> Self {
        Self::new(self.obj.clone(), options, self.methods)
    }

    pub fn move_with_options(self, options: BufferOptions) -> Self {
        Self { options, ..self }
    }

    // SAFETY: should only called if option has contiguous
    pub(crate) fn _contiguous(&self) -> BorrowedValue<[u8]> {
        self.methods
            .contiguous
            .map(|f| f(&self.obj))
            .unwrap_or_else(|| (self.methods.obj_bytes)(&self.obj))
    }

    // SAFETY: should only called if option has contiguous
    pub(crate) fn _contiguous_mut(&self) -> BorrowedValueMut<[u8]> {
        self.methods
            .contiguous_mut
            .map(|f| f(&self.obj))
            .unwrap_or_else(|| (self.methods.obj_bytes_mut)(&self.obj))
    }

    pub(crate) fn _obj_bytes(&self) -> BorrowedValue<[u8]> {
        (self.methods.obj_bytes)(&self.obj)
    }

    pub(crate) fn _obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        (self.methods.obj_bytes_mut)(&self.obj)
    }

    pub(crate) fn _release(&self) {
        if let Some(f) = self.methods.release {
            f(&self.obj)
        }
    }

    pub(crate) fn _retain(&self) {
        if let Some(f) = self.methods.retain {
            f(&self.obj)
        }
    }

    pub(crate) fn _collect_bytes(&self, buf: &mut Vec<u8>) {
        self.methods
            .collect_bytes
            .map(|f| f(&self.obj, buf))
            .unwrap_or_else(|| buf.extend_from_slice(&(self.methods.obj_bytes)(&self.obj)))
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
        let obj_cls = obj.class();
        for cls in obj_cls.iter_mro() {
            if let Some(f) = cls.slots.as_buffer.as_ref() {
                return f(obj, vm);
            }
        }
        Err(vm.new_type_error(format!(
            "a bytes-like object is required, not '{}'",
            obj_cls.name()
        )))
    }
}

// What we actually want to implement is:
// impl<T> Drop for T where T: BufferInternal
// but it is not supported by Rust
impl Drop for PyBuffer {
    fn drop(&mut self) {
        self._release();
    }
}

impl Clone for PyBuffer {
    fn clone(&self) -> Self {
        self.clone_with_options(self.options.clone())
    }
}

pub trait BufferResizeGuard<'a> {
    type Resizable: 'a;
    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable>;
}

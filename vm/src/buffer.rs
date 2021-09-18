//! Buffer protocol

use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::rc::PyRc;
use crate::PyThreadingConstraint;
use crate::{PyObjectRef, PyResult, TryFromBorrowedObject, TypeProtocol, VirtualMachine};
use std::{borrow::Cow, fmt::Debug};

pub trait PyBufferInternal: Debug + PyThreadingConstraint {
    /// Get the full inner buffer of this memory. You probably want [`as_contiguous()`], as
    /// `obj_bytes` doesn't take into account the range a memoryview might operate on, among other
    /// footguns.
    fn obj_bytes(&self) -> BorrowedValue<[u8]>;
    /// Get the full inner buffer of this memory, mutably. You probably want
    /// [`as_contiguous_mut()`], as `obj_bytes` doesn't take into account the range a memoryview
    /// might operate on, among other footguns.
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]>;
    fn release(&self);
    // not included in PyBuffer protocol itself
    fn retain(&self);
}

#[derive(Debug)]
pub struct PyBuffer {
    pub obj: PyObjectRef,
    pub options: BufferOptions,
    pub(crate) internal: PyRc<dyn PyBufferInternal>,
}

impl PyBuffer {
    pub fn new(
        obj: PyObjectRef,
        buffer: impl PyBufferInternal + 'static,
        options: BufferOptions,
    ) -> Self {
        buffer.retain();
        Self {
            obj,
            options,
            internal: PyRc::new(buffer),
        }
    }
    pub fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        if !self.options.contiguous {
            return None;
        }
        Some(self.internal.obj_bytes())
    }

    pub fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        if !self.options.contiguous {
            return None;
        }
        Some(self.internal.obj_bytes_mut())
    }

    pub fn to_contiguous(&self) -> Vec<u8> {
        self.internal.obj_bytes().to_vec()
    }

    pub fn clone_with_options(&self, options: BufferOptions) -> Self {
        self.internal.retain();
        Self {
            obj: self.obj.clone(),
            options,
            internal: self.internal.clone(),
        }
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
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
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
// impl<T> Drop for T where T: PyBufferInternal
// but it is not supported by Rust
impl Drop for PyBuffer {
    fn drop(&mut self) {
        self.internal.release();
    }
}

impl Clone for PyBuffer {
    fn clone(&self) -> Self {
        self.clone_with_options(self.options.clone())
    }
}

pub(crate) trait ResizeGuard<'a> {
    type Resizable: 'a;
    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable>;
}

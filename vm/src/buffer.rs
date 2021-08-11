//! Buffer protocol

use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::rc::PyRc;
use crate::PyThreadingConstraint;
use crate::{PyObjectRef, PyResult, TypeProtocol};
use crate::{TryFromBorrowedObject, VirtualMachine};
use std::{borrow::Cow, fmt::Debug, ops::Deref};

pub trait PyBuffer: Debug + PyThreadingConstraint {
    fn get_options(&self) -> &BufferOptions;
    /// Get the full inner buffer of this memory. You probably want [`as_contiguous()`], as
    /// `obj_bytes` doesn't take into account the range a memoryview might operate on, among other
    /// footguns.
    fn obj_bytes(&self) -> BorrowedValue<[u8]>;
    /// Get the full inner buffer of this memory, mutably. You probably want
    /// [`as_contiguous_mut()`], as `obj_bytes` doesn't take into account the range a memoryview
    /// might operate on, among other footguns.
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]>;
    fn release(&self);

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        if !self.get_options().contiguous {
            return None;
        }
        Some(self.obj_bytes())
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        if !self.get_options().contiguous {
            return None;
        }
        Some(self.obj_bytes_mut())
    }

    fn to_contiguous(&self) -> Vec<u8> {
        self.obj_bytes().to_vec()
    }
}

#[derive(Debug, Clone)]
pub struct BufferOptions {
    pub readonly: bool,
    pub len: usize,
    pub itemsize: usize,
    pub contiguous: bool,
    pub format: Cow<'static, str>,
    // TODO: support multiple dimension array
    pub ndim: usize,
    pub shape: Vec<usize>,
    pub strides: Vec<isize>,
}

impl BufferOptions {
    pub const DEFAULT: Self = BufferOptions {
        readonly: true,
        len: 0,
        itemsize: 1,
        contiguous: true,
        format: Cow::Borrowed("B"),
        ndim: 1,
        shape: Vec::new(),
        strides: Vec::new(),
    };
}

impl Default for BufferOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug)]
pub struct PyBufferRef(Box<dyn PyBuffer>);

impl TryFromBorrowedObject for PyBufferRef {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let obj_cls = obj.class();
        for cls in obj_cls.iter_mro() {
            if let Some(f) = cls.slots.as_buffer.as_ref() {
                return f(obj, vm).map(|x| PyBufferRef(x));
            }
        }
        Err(vm.new_type_error(format!(
            "a bytes-like object is required, not '{}'",
            obj_cls.name
        )))
    }
}

impl Drop for PyBufferRef {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl Deref for PyBufferRef {
    type Target = dyn PyBuffer;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl PyBufferRef {
    pub fn new(buffer: impl PyBuffer + 'static) -> Self {
        Self(Box::new(buffer))
    }

    pub fn into_rcbuf(self) -> RcBuffer {
        // move self.0 out of self; PyBufferRef impls Drop so it's tricky
        let this = std::mem::ManuallyDrop::new(self);
        let buf_box = unsafe { std::ptr::read(&this.0) };
        RcBuffer(buf_box.into())
    }
}

impl From<Box<dyn PyBuffer>> for PyBufferRef {
    fn from(buffer: Box<dyn PyBuffer>) -> Self {
        PyBufferRef(buffer)
    }
}

#[derive(Debug, Clone)]
pub struct RcBuffer(PyRc<dyn PyBuffer>);
impl Deref for RcBuffer {
    type Target = dyn PyBuffer;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl Drop for RcBuffer {
    fn drop(&mut self) {
        // check if this is the last rc before the inner buffer gets dropped
        if let Some(buf) = PyRc::get_mut(&mut self.0) {
            buf.release()
        }
    }
}

impl PyBuffer for RcBuffer {
    fn get_options(&self) -> &BufferOptions {
        self.0.get_options()
    }
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.0.obj_bytes()
    }
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        self.0.obj_bytes_mut()
    }
    fn release(&self) {}
    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        self.0.as_contiguous()
    }
    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.0.as_contiguous_mut()
    }
    fn to_contiguous(&self) -> Vec<u8> {
        self.0.to_contiguous()
    }
}

pub(crate) trait ResizeGuard<'a> {
    type Resizable: 'a;
    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable>;
}

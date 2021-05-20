use crate::builtins::memory::{try_buffer_from_object, BufferRef};
use crate::builtins::PyStrRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::vm::VirtualMachine;
use crate::{PyObjectRef, PyResult, TryFromObject};

#[derive(Debug)]
pub struct PyBytesLike(BufferRef);

#[derive(Debug)]
pub struct PyRwBytesLike(BufferRef);

impl PyBytesLike {
    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&*self.borrow_buf())
    }

    pub fn len(&self) -> usize {
        self.borrow_buf().len()
    }

    pub fn is_empty(&self) -> bool {
        self.borrow_buf().is_empty()
    }

    pub fn to_cow(&self) -> std::borrow::Cow<[u8]> {
        self.borrow_buf().to_vec().into()
    }
}

impl PyRwBytesLike {
    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut *self.borrow_buf_mut())
    }

    pub fn len(&self) -> usize {
        self.borrow_buf_mut().len()
    }

    pub fn is_empty(&self) -> bool {
        self.borrow_buf_mut().is_empty()
    }
}

impl PyBytesLike {
    pub fn new(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let buffer = try_buffer_from_object(vm, &obj)?;
        if buffer.get_options().contiguous {
            Ok(Self(buffer))
        } else {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        }
    }

    pub fn into_buffer(self) -> BufferRef {
        self.0
    }

    pub fn borrow_buf(&self) -> BorrowedValue<'_, [u8]> {
        self.0.as_contiguous().unwrap()
    }
}

impl TryFromObject for PyBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::new(vm, &obj)
    }
}

pub fn try_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&[u8]) -> R,
) -> PyResult<R> {
    let buffer = try_buffer_from_object(vm, obj)?;
    buffer.as_contiguous().map(|x| f(&*x)).ok_or_else(|| {
        vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned())
    })
}

pub fn try_rw_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&mut [u8]) -> R,
) -> PyResult<R> {
    let buffer = try_buffer_from_object(vm, obj)?;
    buffer
        .as_contiguous_mut()
        .map(|mut x| f(&mut *x))
        .ok_or_else(|| vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
}

impl PyRwBytesLike {
    pub fn new(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let buffer = try_buffer_from_object(vm, &obj)?;
        let options = buffer.get_options();
        if !options.contiguous {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        } else if options.readonly {
            Err(vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
        } else {
            Ok(Self(buffer))
        }
    }

    pub fn into_buffer(self) -> BufferRef {
        self.0
    }

    pub fn borrow_buf_mut(&self) -> BorrowedValueMut<'_, [u8]> {
        self.0.as_contiguous_mut().unwrap()
    }
}

impl TryFromObject for PyRwBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::new(vm, &obj)
    }
}

/// A buffer or utf8 string. Like the `s*` format code for `PyArg_Parse` in CPython.
pub enum BufOrStr {
    Buf(PyBytesLike),
    Str(PyStrRef),
}

impl TryFromObject for BufOrStr {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast()
            .map(Self::Str)
            .or_else(|obj| PyBytesLike::try_from_object(vm, obj).map(Self::Buf))
    }
}

impl BufOrStr {
    pub fn borrow_bytes(&self) -> BorrowedValue<'_, [u8]> {
        match self {
            Self::Buf(b) => b.borrow_buf(),
            Self::Str(s) => s.as_str().as_bytes().into(),
        }
    }
}

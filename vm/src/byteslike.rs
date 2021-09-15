use crate::buffer::PyBuffer;
use crate::builtins::PyStrRef;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::vm::VirtualMachine;
use crate::{PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject};

// Python/getargs.c

/// any bytes-like object. Like the `y*` format code for `PyArg_Parse` in CPython.
#[derive(Debug)]
pub struct ArgBytesLike(PyBuffer);

/// A memory buffer, read-write access. Like the `w*` format code for `PyArg_Parse` in CPython.
#[derive(Debug)]
pub struct ArgMemoryBuffer(PyBuffer);

impl ArgBytesLike {
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

impl ArgMemoryBuffer {
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

impl ArgBytesLike {
    pub fn new(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let buffer = PyBuffer::try_from_borrowed_object(vm, obj)?;
        if buffer.options.contiguous {
            Ok(Self(buffer))
        } else {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        }
    }

    pub fn into_buffer(self) -> PyBuffer {
        self.0
    }

    pub fn borrow_buf(&self) -> BorrowedValue<'_, [u8]> {
        self.0.as_contiguous().unwrap()
    }
}

impl TryFromBorrowedObject for ArgBytesLike {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        Self::new(vm, obj)
    }
}

pub fn try_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&[u8]) -> R,
) -> PyResult<R> {
    let buffer = PyBuffer::try_from_borrowed_object(vm, obj)?;
    buffer.as_contiguous().map(|x| f(&*x)).ok_or_else(|| {
        vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned())
    })
}

pub fn try_rw_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&mut [u8]) -> R,
) -> PyResult<R> {
    let buffer = PyBuffer::try_from_borrowed_object(vm, obj)?;
    buffer
        .as_contiguous_mut()
        .map(|mut x| f(&mut *x))
        .ok_or_else(|| vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
}

impl ArgMemoryBuffer {
    pub fn new(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let buffer = PyBuffer::try_from_borrowed_object(vm, obj)?;
        if !buffer.options.contiguous {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        } else if buffer.options.readonly {
            Err(vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
        } else {
            Ok(Self(buffer))
        }
    }

    pub fn into_buffer(self) -> PyBuffer {
        self.0
    }

    pub fn borrow_buf_mut(&self) -> BorrowedValueMut<'_, [u8]> {
        self.0.as_contiguous_mut().unwrap()
    }
}

impl TryFromBorrowedObject for ArgMemoryBuffer {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        Self::new(vm, obj)
    }
}

/// A text string or bytes-like object. Like the `s*` format code for `PyArg_Parse` in CPython.
pub enum ArgStrOrBytesLike {
    Buf(ArgBytesLike),
    Str(PyStrRef),
}

impl TryFromObject for ArgStrOrBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast()
            .map(Self::Str)
            .or_else(|obj| ArgBytesLike::try_from_object(vm, obj).map(Self::Buf))
    }
}

impl ArgStrOrBytesLike {
    pub fn borrow_bytes(&self) -> BorrowedValue<'_, [u8]> {
        match self {
            Self::Buf(b) => b.borrow_buf(),
            Self::Str(s) => s.as_str().as_bytes().into(),
        }
    }
}
